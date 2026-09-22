"""Chunked upload storage for corpus import packs.

Owns two directories on disk::

    {UPLOADS_DIR}/{upload_id}/        staging area for an in-progress upload
        data.bin                      raw bytes, appended in order, part by part
        meta.json                     {upload_id, name, size, sha256, part_size,
                                        received, bytes, complete, pack_id, created_at}

    {PACKS_DIR}/{pack_id}.sopack       the finished upload, moved here by complete()
    {PACKS_DIR}/{pack_id}.meta.json    {pack_id, name, bytes, sha256, uploaded_at}

Both roots are overridable — set the module attributes directly (tests do this)
or the ``UPLOADS_DIR`` / ``PACKS_DIR`` env vars before import, the same pattern
``keystore.py`` uses for ``_DB_PATH``. ``PACKS_DIR`` matches the env var
``app/import_service.py`` reads for the same directory.

Wire contract: docs/IMPORT-API.md §1 (uploads) and §2 (packs). This module is
the only thing that reads or writes those two directories; the admin routes in
``app/admin.py`` call it, they never touch the filesystem directly.

Parts must arrive strictly in order: ``write_part`` rejects a part number ahead
of what has been received (a gap raises ``part_gap``) and treats a retry of an
already-received part as a no-op, so the browser's per-part retry logic can
safely call the same part twice.

Stdlib only.
"""

from __future__ import annotations

import hashlib
import json
import os
import secrets
import shutil
import threading
import time
import zipfile
from pathlib import Path

PART_SIZE = 8 * 1024 * 1024              # 8 MiB, per docs/IMPORT-API.md
MAX_UPLOAD_AGE_S = 24 * 60 * 60          # uploads not completed within 24h are pruned

# Overridable roots. Read from the environment once at import time; tests
# reassign these module attributes directly (functions below look them up by
# name on every call, so reassignment after import takes effect immediately).
# ``PACKS_DIR`` intentionally matches the env var name ``app/import_service.py``
# reads (its ``_packs_dir()``) — both sides must agree on where a completed
# upload lands, or the job pipeline looks for the pack in the wrong place.
UPLOADS_DIR = os.environ.get("UPLOADS_DIR", "/data/uploads")
PACKS_DIR = os.environ.get("PACKS_DIR", "/data/packs")

_locks_guard = threading.Lock()
_upload_locks: dict[str, threading.Lock] = {}


class UploadError(Exception):
    """A client-facing upload failure.

    ``code``/``detail`` map directly onto the ``{"error", "detail"}`` JSON
    body docs/IMPORT-API.md specifies; ``status`` is the HTTP status the
    admin route should answer with.
    """

    def __init__(self, code: str, detail: str, status: int = 400):
        super().__init__(detail)
        self.code = code
        self.detail = detail
        self.status = status


def _not_found(kind: str = "upload") -> UploadError:
    return UploadError("not_found", f"no such {kind}", 404)


def _upload_lock(upload_id: str) -> threading.Lock:
    with _locks_guard:
        lk = _upload_locks.get(upload_id)
        if lk is None:
            lk = _upload_locks[upload_id] = threading.Lock()
        return lk


def _dir(upload_id: str) -> Path:
    return Path(UPLOADS_DIR) / upload_id


def _meta_path(upload_id: str) -> Path:
    return _dir(upload_id) / "meta.json"


def _data_path(upload_id: str) -> Path:
    return _dir(upload_id) / "data.bin"


def _pack_path(pack_id: str) -> Path:
    return Path(PACKS_DIR) / f"{pack_id}.sopack"


def _pack_meta_path(pack_id: str) -> Path:
    return Path(PACKS_DIR) / f"{pack_id}.meta.json"


def _write_json_atomic(path: Path, obj: dict) -> None:
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(obj, ensure_ascii=False), encoding="utf-8")
    os.replace(tmp, path)


def _read_json(path: Path) -> dict | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def init() -> None:
    os.makedirs(UPLOADS_DIR, exist_ok=True)
    os.makedirs(PACKS_DIR, exist_ok=True)


def _load_meta(upload_id: str) -> dict:
    meta = _read_json(_meta_path(upload_id))
    if meta is None:
        raise _not_found()
    return meta


def _rmtree(d: Path) -> None:
    shutil.rmtree(d, ignore_errors=True)


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# ── uploads ──────────────────────────────────────────────────────────────────

def create(name, size, sha256) -> dict:
    """Start a chunked upload. Returns the full stored record (``POST
    /import/uploads`` returns a subset of it — see ``app/admin.py``)."""
    prune_stale()
    if not isinstance(name, str) or not name.strip():
        raise UploadError("bad_request", "name is required")
    if not isinstance(size, int) or isinstance(size, bool) or size <= 0:
        raise UploadError("bad_request", "size must be a positive integer")
    if (not isinstance(sha256, str) or len(sha256) != 64
            or not all(c in "0123456789abcdef" for c in sha256.lower())):
        raise UploadError("bad_request", "sha256 must be a 64-character hex digest")

    init()
    upload_id = secrets.token_hex(16)
    d = _dir(upload_id)
    d.mkdir(parents=True, exist_ok=True)
    _data_path(upload_id).touch()
    meta = {
        "upload_id": upload_id, "name": name.strip(), "size": size,
        "sha256": sha256.lower(), "part_size": PART_SIZE,
        "received": 0, "bytes": 0, "complete": False,
        "pack_id": None, "created_at": time.time(),
    }
    _write_json_atomic(_meta_path(upload_id), meta)
    return meta


def status(upload_id: str) -> dict:
    """``GET /import/uploads/{id}`` body."""
    meta = _load_meta(upload_id)
    return {
        "upload_id": meta["upload_id"], "name": meta["name"], "size": meta["size"],
        "bytes": meta["bytes"], "received": meta["received"],
        "part_size": meta["part_size"], "complete": meta["complete"],
    }


async def write_part(upload_id: str, n: int, chunks) -> dict:
    """Append part *n* (0-based) from the async byte-chunk iterable *chunks*.

    Streams straight to disk — a part is never buffered whole in memory; each
    chunk handed over by the ASGI server is written as it arrives. Returns
    ``{"received", "bytes"}``.
    """
    if not isinstance(n, int) or n < 0:
        raise UploadError("bad_request", "part number must be a non-negative integer")

    lk = _upload_lock(upload_id)
    with lk:
        meta = _load_meta(upload_id)
        if meta["complete"]:
            raise UploadError("already_complete", "upload is already complete")
        expected = meta["received"]

        if n > expected:
            async for _ in chunks:      # drain so the connection isn't left dangling
                pass
            raise UploadError("part_gap", f"expected part {expected}, got {n}")

        if n < expected:
            async for _ in chunks:      # retry of an already-accepted part — no-op
                pass
            return {"received": meta["received"], "bytes": meta["bytes"]}

        data_path = _data_path(upload_id)
        with open(data_path, "ab") as fh:
            async for chunk in chunks:
                if chunk:
                    fh.write(chunk)

        meta["bytes"] = data_path.stat().st_size
        meta["received"] = n + 1
        _write_json_atomic(_meta_path(upload_id), meta)
        return {"received": meta["received"], "bytes": meta["bytes"]}


def complete(upload_id: str) -> dict:
    """Verify size + sha256 and finalize. Returns ``{"pack_id", "manifest"}``.

    A mismatch discards the staging file (``checksum_mismatch``, 400).
    Re-calling on an already-completed upload is idempotent.
    """
    lk = _upload_lock(upload_id)
    with lk:
        meta = _load_meta(upload_id)

        if meta["complete"] and meta.get("pack_id"):
            manifest = _read_manifest(_pack_path(meta["pack_id"]))
            if manifest is not None:
                return {"pack_id": meta["pack_id"], "manifest": manifest}

        data_path = _data_path(upload_id)
        if meta["bytes"] != meta["size"] or not data_path.exists():
            _rmtree(_dir(upload_id))
            raise UploadError(
                "checksum_mismatch",
                f"received {meta['bytes']} bytes, declared {meta['size']}")

        got = _sha256_file(data_path)
        if got != meta["sha256"]:
            _rmtree(_dir(upload_id))
            raise UploadError(
                "checksum_mismatch",
                f"sha256 {got[:12]}… does not match declared {meta['sha256'][:12]}…")

        pack_id = upload_id
        init()
        dest = _pack_path(pack_id)
        os.replace(data_path, dest)

        manifest = _read_manifest(dest)
        if manifest is None:
            dest.unlink(missing_ok=True)
            raise UploadError(
                "invalid_pack",
                "upload is not a readable .sopack (bad zip or missing manifest.json)")

        _write_json_atomic(_pack_meta_path(pack_id), {
            "pack_id": pack_id, "name": meta["name"], "bytes": meta["bytes"],
            "sha256": meta["sha256"], "uploaded_at": time.time(),
        })

        meta["complete"] = True
        meta["pack_id"] = pack_id
        _write_json_atomic(_meta_path(upload_id), meta)
        _rmtree(_dir(upload_id))   # data.bin already moved out; nothing left to keep

        return {"pack_id": pack_id, "manifest": manifest}


def delete(upload_id: str) -> None:
    """``DELETE /import/uploads/{id}`` — discard an in-progress upload."""
    d = _dir(upload_id)
    if not d.exists():
        raise _not_found()
    _rmtree(d)
    with _locks_guard:
        _upload_locks.pop(upload_id, None)


def prune_stale(max_age_s: float = MAX_UPLOAD_AGE_S, now: float | None = None) -> int:
    """Remove incomplete uploads older than *max_age_s*. Returns how many."""
    root = Path(UPLOADS_DIR)
    if not root.exists():
        return 0
    now = time.time() if now is None else now
    pruned = 0
    for d in root.iterdir():
        if not d.is_dir():
            continue
        meta = _read_json(d / "meta.json")
        if meta is None or meta.get("complete"):
            continue
        if now - meta.get("created_at", now) > max_age_s:
            _rmtree(d)
            with _locks_guard:
                _upload_locks.pop(d.name, None)
            pruned += 1
    return pruned


# ── packs ────────────────────────────────────────────────────────────────────

def _read_manifest(path: Path) -> dict | None:
    """Cheap manifest read — parses ``manifest.json`` without a full CRC scan
    of the (possibly very large) vectors entry. Contract/embedding validation
    is the import job's ``open``/``contract`` stages, not this module's job."""
    try:
        with zipfile.ZipFile(path) as zf:
            return json.loads(zf.read("manifest.json"))
    except (zipfile.BadZipFile, KeyError, json.JSONDecodeError, FileNotFoundError):
        return None


def list_packs() -> list[dict]:
    root = Path(PACKS_DIR)
    if not root.exists():
        return []
    out = []
    for p in sorted(root.glob("*.sopack")):
        pack_id = p.stem
        side = _read_json(_pack_meta_path(pack_id)) or {}
        manifest = _read_manifest(p) or {}
        counts = manifest.get("counts") or {}
        out.append({
            "pack_id": pack_id,
            "name": side.get("name", p.name),
            "bytes": side.get("bytes", p.stat().st_size),
            "uploaded_at": side.get("uploaded_at"),
            "profile": manifest.get("profile"),
            "points": counts.get("points"),
            "books": counts.get("books"),
        })
    out.sort(key=lambda r: r["uploaded_at"] or 0, reverse=True)
    return out


def get_pack(pack_id: str) -> dict:
    """One pack incl. its full manifest — ``GET /import/packs/{id}``."""
    p = _pack_path(pack_id)
    if not p.exists():
        raise _not_found("pack")
    side = _read_json(_pack_meta_path(pack_id)) or {}
    manifest = _read_manifest(p)
    if manifest is None:
        raise UploadError("invalid_pack", "pack is not a readable .sopack", 500)
    return {
        "pack_id": pack_id, "name": side.get("name", p.name),
        "bytes": side.get("bytes", p.stat().st_size),
        "uploaded_at": side.get("uploaded_at"), "manifest": manifest,
    }


def delete_pack(pack_id: str) -> None:
    p = _pack_path(pack_id)
    if not p.exists():
        raise _not_found("pack")
    p.unlink()
    _pack_meta_path(pack_id).unlink(missing_ok=True)

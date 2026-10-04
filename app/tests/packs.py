"""Test-only ``.sopack`` writer and pack builder.

The server only ever *reads* packs (``app/pack/format.py``); production packs
are written by the Rust ``sopack`` CLI. Tests still need structurally real
packs, so the writer half of the format lives here, next to a tiny builder
that embeds with a deterministic fake instead of the 2 GB model.
"""

from __future__ import annotations

import array
import hashlib
import json
import os
import random
import sys
import tempfile
import zipfile
from pathlib import Path

from app.pack import contract
from app.pack.format import ITEM_SIZE, MANIFEST, POINTS, PROBE, TITLES, VECTORS, PackError, _fields


def _f32_bytes(vector) -> bytes:
    """One vector as little-endian float32 bytes."""
    buf = array.array("f", vector)
    if sys.byteorder != "little":
        buf.byteswap()
    return buf.tobytes()


class _Hashing:
    """Wraps a zip entry stream, hashing every byte written to it."""

    def __init__(self, stream):
        self._stream = stream
        self._h = hashlib.sha256()
        self.size = 0

    def write(self, data: bytes) -> None:
        self._h.update(data)
        self.size += len(data)
        self._stream.write(data)

    @property
    def digest(self) -> str:
        return self._h.hexdigest()


class PackWriter:
    """Streams points into a .sopack.

    ::

        with PackWriter(path, profile="sop", pack_id="pioneers-…") as w:
            for uid, payload, vector in rows:
                w.add(uid, payload, vector)
            w.set_books([...]); w.set_probe(...); w.set_titles({...})
    """

    def __init__(self, path, profile: str, pack_id: str, created_by: str,
                 id_rule: str | None = None, dim: int = contract.VECTOR_SIZE,
                 runtime: str | None = None, device: str = "cpu",
                 threads: int = 1, batch_tokens: int | None = None):
        self.path = Path(path)
        self.profile = contract.get_profile(profile)
        self.pack_id = pack_id
        self.created_by = created_by
        self.id_rule = id_rule or self.profile.default_id_rule
        self.dim = dim
        # Embedding-block provenance (R10) — informational only, never
        # compared by check_embedding(). `runtime` defaults to `created_by`
        # (already "sopack <ver> on <platform>"), which is a reasonable
        # stand-in when a caller (e.g. a test) does not supply one.
        self.runtime = runtime or created_by
        self.device = device
        self.threads = threads
        self.batch_tokens = batch_tokens
        self.count = 0
        self._books: list[dict] = []
        self._probe: dict | None = None
        self._probe_vectors: list[list[float]] = []
        self._titles: dict | None = None
        self._zf = None
        self._points = None
        self._vectors = None

    def __enter__(self):
        # Build at a unique temp path and rename into place only on success.
        # Two consequences, both wanted: a partially written .sopack is never
        # visible at the final path (a reader can only ever see a complete
        # pack), and a writer that fails deletes ITS OWN file rather than
        # whatever now sits at the shared destination — without this, a second
        # writer aimed at the same output unlinks the first one's finished work
        # on its way down.
        fd, tmp_out = tempfile.mkstemp(dir=self.path.parent,
                                       prefix=f".{self.path.name}.", suffix=".part")
        os.close(fd)
        self._tmp_out = Path(tmp_out)
        self._zf = zipfile.ZipFile(self._tmp_out, "w", zipfile.ZIP_DEFLATED)
        self._points_raw = self._zf.open(POINTS, "w")
        self._points = _Hashing(self._points_raw)
        # A ZipFile allows only one open write handle at a time, so vectors are
        # spooled to a sibling temp file and copied in once points.jsonl is
        # closed. Spooling to disk rather than a list keeps `pack` at a few MB
        # of RAM on a 250 MB corpus.
        #
        # The name MUST be unique, not derived from `path`. A derived name is
        # shared by every writer aiming at the same output, so a second run —
        # or a dying earlier one, whose cleanup unlinks it — silently destroys
        # the spool of a live run. That cost a 40-minute embed of a real book:
        # all 2486 blocks embedded, then __exit__ died on a missing temp file.
        fd, spool = tempfile.mkstemp(dir=self.path.parent,
                                     prefix=f".{self.path.name}.", suffix=".vectors.tmp")
        self._spool_path = Path(spool)
        self._spool = os.fdopen(fd, "wb")
        self._vhash = hashlib.sha256()
        return self

    def add(self, uid: str, payload: dict, vector) -> str:
        """Append one point. Returns the point id."""
        if len(vector) != self.dim:
            raise PackError(
                f"point {uid!r}: vector has {len(vector)} dims, expected {self.dim}")
        problems = contract.validate_payload(self.profile, payload)
        if problems:
            raise PackError(f"point {uid!r}: " + "; ".join(problems))
        pid = contract.point_id(self.id_rule, _fields(self.profile, payload, uid))
        line = json.dumps({"uid": uid, "id": pid, "payload": payload},
                          ensure_ascii=False) + "\n"
        self._points.write(line.encode("utf-8"))
        raw = _f32_bytes(vector)
        self._vhash.update(raw)
        self._spool.write(raw)
        self.count += 1
        return pid

    def set_books(self, books: list[dict]) -> None:
        self._books = books

    def set_titles(self, titles: dict | None) -> None:
        self._titles = titles

    def set_probe(self, entries: list[dict], vectors: list[list[float]], *,
                  self_check: dict, fixture_sha256: str | None = None) -> None:
        """*entries* are the calibration fixture's own ``{"id", "profile"[,
        "uid"]}`` records (SOPACK-2-FORMAT.md §3), in fixture order; *vectors*
        are THIS pack's fresh embeddings of their text, same order — computed
        by the same model instance that embedded the books, before any book
        was embedded (§3, R11). *self_check* is what the caller measured
        comparing *vectors* against the fixture's own stored vectors: ``{"n",
        "min_cosine", "mean_cosine", "threshold"}``. The importer repeats an
        equivalent comparison offline (its own copy of the fixture) — this
        pack-time check exists so a broken environment fails on the laptop in
        seconds, not on the server after a long embed.

        *fixture_sha256* is the sha256 the manifest declares this pack was
        calibrated against — defaults to the contract's own committed
        fixture (``contract.CALIBRATION_SHA256``), which is what every
        production pack uses; a caller that built *entries* from an
        overridden fixture (tests) passes that fixture's own sha
        (``contract.calibration_fixture_sha256``) so the importer, given the
        same override, agrees."""
        if len(entries) != len(vectors):
            raise PackError("probe: fixture entry/vector count mismatch")
        self._probe = {
            "kind": "calibration",
            "fixture_sha256": (fixture_sha256 if fixture_sha256 is not None
                              else contract.CALIBRATION_SHA256),
            "vectors": PROBE,
            "entries": [
                {"id": e["id"], "profile": e.get("profile"), "vector_offset": i}
                for i, e in enumerate(entries)
            ],
            "self_check": dict(self_check),
        }
        self._probe_vectors = vectors

    def __exit__(self, exc_type, exc, tb):
        self._spool.close()
        if exc_type is not None:
            self._points_raw.close()
            self._zf.close()
            self._tmp_out.unlink(missing_ok=True)
            self._spool_path.unlink(missing_ok=True)
            return False

        self._points_raw.close()
        points_sha, points_size = self._points.digest, self._points.size

        try:
            spooled = self._spool_path.stat().st_size
        except OSError as exc:
            self._zf.close()
            self._tmp_out.unlink(missing_ok=True)
            raise PackError(
                f"the vector spool {self._spool_path} disappeared before the pack "
                f"could be written ({exc}). Every embedded vector for this run is "
                f"lost. This should be impossible now that spool names are unique "
                f"per writer — if it recurs, something outside sopack is deleting "
                f"temp files in {self.path.parent}.") from exc
        if spooled != self.count * self.dim * ITEM_SIZE:
            self._zf.close()
            self._tmp_out.unlink(missing_ok=True)
            self._spool_path.unlink(missing_ok=True)
            raise PackError(
                f"vector bytes ({spooled}) do not match point count "
                f"({self.count} x {self.dim} x {ITEM_SIZE})")

        # STORED, not DEFLATED: normalised float32 is incompressible noise, so
        # deflating it buys ~2 % and costs the SERVER a full decompression pass
        # over every byte at import time. Storing it means the bytes go from the
        # zip into array.frombytes with no work in between.
        info = zipfile.ZipInfo(VECTORS)
        info.compress_type = zipfile.ZIP_STORED
        with self._zf.open(info, "w") as fh, open(self._spool_path, "rb") as src:
            for chunk in iter(lambda: src.read(1 << 20), b""):
                fh.write(chunk)
        self._spool_path.unlink(missing_ok=True)
        sha = {POINTS: points_sha, VECTORS: self._vhash.hexdigest()}

        if self._probe is not None:
            pinfo = zipfile.ZipInfo(PROBE)
            pinfo.compress_type = zipfile.ZIP_STORED
            with self._zf.open(pinfo, "w") as fh:
                ph = _Hashing(fh)
                for v in self._probe_vectors:
                    ph.write(_f32_bytes(v))
            sha[PROBE] = ph.digest

        if self._titles is not None:
            blob = json.dumps(self._titles, ensure_ascii=False, indent=2).encode("utf-8")
            self._zf.writestr(TITLES, blob)
            sha[TITLES] = hashlib.sha256(blob).hexdigest()

        manifest = {
            "schema": contract.SCHEMA_PACK,
            "profile": self.profile.name,
            "pack_id": self.pack_id,
            "created_by": self.created_by,
            "target": {"profile": self.profile.name, "contract": contract.CONTRACT_ID},
            "contract": {
                "id": contract.CONTRACT_ID,
                "sha256": contract.CONTRACT_SHA256,
                "calibration_sha256": contract.CALIBRATION_SHA256,
            },
            "embedding": {
                # Exactly the checked keys (SOPACK-2-FORMAT.md §2) plus
                # provenance — not the whole contract.EMBEDDING dict, which
                # also carries query_prefix/reference_runtimes (contract-only
                # concerns a pack never needs to declare about itself).
                "model": contract.EMBEDDING["model"],
                "pooling": contract.EMBEDDING["pooling"],
                "normalized": contract.EMBEDDING["normalized"],
                "dim": self.dim,
                "distance": contract.EMBEDDING["distance"],
                "max_tokens": contract.EMBEDDING["max_tokens"],
                "passage_prefix": contract.EMBEDDING["passage_prefix"],
                "runtime": self.runtime,
                "device": self.device,
                "threads": self.threads,
                "batch_tokens": self.batch_tokens,
            },
            "id_rule": self.id_rule,
            "id_rule_doc": contract.ID_RULE_DOC[self.id_rule],
            "counts": {"points": self.count, "books": len(self._books), "dim": self.dim,
                       "points_bytes": points_size},
            "sha256": sha,
            "books": self._books,
            "probe": self._probe,
        }
        self._zf.writestr(MANIFEST,
                          json.dumps(manifest, ensure_ascii=False, indent=2))
        self._zf.close()
        # Atomic publish: until this line the destination either does not exist
        # or still holds a previous, complete pack. os.replace is atomic within
        # a filesystem, so no reader ever observes a half-written .sopack.
        os.replace(self._tmp_out, self.path)
        return False




# ── a minimal stand-in for the packer ────────────────────────────────────────

def fake_embed(text: str, dim: int = contract.VECTOR_SIZE) -> list[float]:
    """Deterministic stand-in for the real model: the same text always gets
    the same vector."""
    rnd = random.Random(f"sopack-fake-embed:{text}")
    return [rnd.random() for _ in range(dim)]


def fake_calibration(texts, *, dim: int = contract.VECTOR_SIZE, profile: str = "sop") -> dict:
    """A calibration fixture doc whose stored vectors are exactly what
    :func:`fake_embed` produces for ``passage_prefix + text``."""
    prefix = contract.EMBEDDING["passage_prefix"]
    entries = [{"id": f"fixture-{i}", "profile": profile, "uid": None, "lang": "en",
                "note": f"test fixture {i}", "text": text,
                "vector": fake_embed(prefix + text, dim)}
               for i, text in enumerate(texts)]
    return {"schema": contract.SCHEMA_CALIBRATION, "contract": contract.CONTRACT_ID,
            "entries": entries}


def build_sop_pack(out_path, *, lang: str, book_code: str, n_blocks: int = 3,
                   calibration: dict, meta: dict) -> dict:
    """A real, checked ``sopack/2`` ``sop`` pack of one book — the shape the
    Rust ``sopack pack`` writes (book metadata on every point and in the
    manifest's book entry), embedded with :func:`fake_embed`. Returns the
    manifest."""
    profile = contract.get_profile("sop")
    prefix = contract.EMBEDDING["passage_prefix"]
    id_rule = profile.default_id_rule
    with PackWriter(out_path, profile="sop", pack_id=f"test-{book_code}",
                    created_by="tests", id_rule=id_rule) as writer:
        first_id = None
        for i in range(n_blocks):
            para_key = f"{i + 1}.1"
            payload = {"lang": lang, "book_code": book_code, "page": i + 1, "para": 1,
                       "para_key": para_key, "raw_text": f"{book_code} block {i + 1} text.",
                       "aligned": None, **{k: v for k, v in meta.items() if v is not None}}
            pid = writer.add(f"{lang}:{book_code}:{para_key}#0", payload,
                             fake_embed(prefix + payload["raw_text"]))
            first_id = first_id or pid
        writer.set_books([{"book_code": book_code, "lang": lang, "points": n_blocks,
                           "first_id": first_id, "id_rule": id_rule, **meta}])
        vectors = [fake_embed(prefix + e["text"]) for e in calibration["entries"]]
        writer.set_probe([{"id": e["id"], "profile": e["profile"]} for e in calibration["entries"]],
                         vectors, self_check={"n": len(vectors), "min_cosine": 1.0,
                                              "mean_cosine": 1.0, "threshold": 1.0},
                         fixture_sha256=contract.calibration_fixture_sha256(calibration))
    from app.pack.format import PackReader
    with PackReader(out_path) as reader:
        return reader.manifest

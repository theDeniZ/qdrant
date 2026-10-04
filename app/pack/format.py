""".sopack container — the server's reader.

A .sopack is a ZIP holding (``sopack/2``, docs/SOPACK-2-FORMAT.md)::

    manifest.json   contract, counts, checksums, calibration probe
    points.jsonl    one JSON object per point, in vector order
    vectors.f32     N x dim x 4 bytes, little-endian float32, same order
    probe.f32       the pack's own embeddings of the calibration fixture
    titles.json     legacy title fragment — ignored; book metadata travels on
                    every point's payload

Packs are written by the Rust ``sopack`` CLI (``qdrant/sopack-rs``); this
module only reads them. **Stdlib only.** Vectors are raw float32 precisely so
the server never converts anything: bytes go from the zip into
``array.array('f')`` and straight into the upsert body.

``PackReader.batches()`` never holds more than one batch, so peak RAM is
independent of pack size. It accepts ``sopack/1`` (legacy live-canary probe)
and ``sopack/2``, and rejects any other major schema version outright;
unknown manifest keys are always ignored.
"""

from __future__ import annotations

import array
import hashlib
import io
import json
import sys
import zipfile
from pathlib import Path

from . import contract

MANIFEST = "manifest.json"
POINTS = "points.jsonl"
VECTORS = "vectors.f32"
PROBE = "probe.f32"
TITLES = "titles.json"

ITEM_SIZE = 4  # float32


class PackError(Exception):
    """A pack is malformed, truncated or internally inconsistent."""


def _f32_read(raw: bytes, dim: int) -> list[list[float]]:
    """A run of little-endian float32 bytes back into vectors of *dim*."""
    buf = array.array("f")
    buf.frombytes(raw)
    if sys.byteorder != "little":
        buf.byteswap()
    n = len(buf) // dim
    return [buf[i * dim:(i + 1) * dim].tolist() for i in range(n)]


def _fields(profile, payload: dict, uid: str) -> dict:
    """Id-rule inputs for one payload. ``seq`` is carried on the uid's ``#n``
    suffix rather than the payload, because a point that was not split has no
    ``chunk`` key at all (verified against the live pioneer points)."""
    fields = dict(payload)
    if "#" in uid:
        try:
            fields["seq"] = int(uid.rsplit("#", 1)[1])
        except ValueError:
            pass
    else:
        fields.setdefault("seq", 0)
    return fields


class PackReader:
    """Streams a .sopack. Peak RAM is one batch, whatever the file size."""

    def __init__(self, path):
        self.path = Path(path)
        try:
            self._zf = zipfile.ZipFile(self.path)
        except zipfile.BadZipFile as exc:
            raise PackError(f"not a readable zip: {exc}") from exc
        bad = self._zf.testzip()
        if bad is not None:
            raise PackError(f"CRC failure in entry {bad!r}")
        names = set(self._zf.namelist())
        for required in (MANIFEST, POINTS, VECTORS):
            if required not in names:
                raise PackError(f"pack is missing {required!r}")
        try:
            self.manifest = json.loads(self._zf.read(MANIFEST))
        except json.JSONDecodeError as exc:
            raise PackError(f"manifest is not valid JSON: {exc}") from exc
        self._names = names
        self._checked = False

    # ── declared facts ───────────────────────────────────────────────────────
    @property
    def profile(self):
        return contract.get_profile(self.manifest.get("profile", ""))

    @property
    def dim(self) -> int:
        return int(self.manifest["counts"]["dim"])

    @property
    def count(self) -> int:
        return int(self.manifest["counts"]["points"])

    @property
    def id_rule(self) -> str:
        return self.manifest["id_rule"]

    def titles(self) -> dict | None:
        if TITLES not in self._names:
            return None
        return json.loads(self._zf.read(TITLES))

    def probe_vectors(self) -> list[list[float]]:
        if PROBE not in self._names:
            return []
        return _f32_read(self._zf.read(PROBE), self.dim)

    # ── integrity ────────────────────────────────────────────────────────────
    def check(self) -> list[str]:
        """Everything verifiable without touching a store. Empty list is clean.

        Accepts both ``sopack/1`` (legacy) and ``sopack/2`` manifests; any
        other major schema (e.g. a future ``sopack/3``) is rejected outright,
        per SOPACK-2-FORMAT.md."""
        errors = []
        schema = self.manifest.get("schema")
        if schema not in contract.SUPPORTED_PACK_SCHEMAS:
            errors.append(f"unsupported schema {schema!r} "
                          f"(this reader accepts {contract.SUPPORTED_PACK_SCHEMAS!r})")
            return errors
        try:
            profile = self.profile
        except ValueError as exc:
            return [str(exc)]

        errors += contract.check_embedding(self.manifest.get("embedding") or {}, schema)
        errors += contract.check_target(self.manifest.get("target") or {}, profile, schema)

        if self.id_rule not in profile.id_rules:
            errors.append(f"id_rule {self.id_rule!r} is not valid for profile "
                          f"{profile.name!r} (allowed: {', '.join(profile.id_rules)})")
        if self.dim != contract.VECTOR_SIZE:
            errors.append(f"counts.dim is {self.dim}, contract requires {contract.VECTOR_SIZE}")

        declared = self.manifest.get("sha256") or {}
        for entry in (POINTS, VECTORS, PROBE, TITLES):
            if entry not in self._names:
                continue
            want = declared.get(entry)
            if not want:
                errors.append(f"manifest declares no sha256 for {entry!r}")
                continue
            got = self._sha256(entry)
            if got != want:
                errors.append(f"{entry}: sha256 {got[:12]}… != manifest {want[:12]}…")

        want_bytes = self.count * self.dim * ITEM_SIZE
        got_bytes = self._zf.getinfo(VECTORS).file_size
        if got_bytes != want_bytes:
            errors.append(f"{VECTORS} is {got_bytes} bytes, expected "
                          f"{want_bytes} for {self.count} x {self.dim} float32")
        self._checked = not errors
        return errors

    def _sha256(self, entry: str) -> str:
        h = hashlib.sha256()
        with self._zf.open(entry) as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()

    # ── streaming ────────────────────────────────────────────────────────────
    def batches(self, size: int = 128, verify_ids: bool = True,
                require_checked: bool = True):
        """Yield ``(points, vectors)`` pairs of at most *size* each.

        *points* are the parsed JSONL objects; *vectors* are lists of floats in
        the same order. With *verify_ids*, each line's ``id`` is recomputed from
        its ``uid`` and a disagreement raises.

        The id check alone is **not** an integrity check: editing ``raw_text``
        changes neither the uid nor the id, and only the manifest's sha256
        catches it. So reading refuses by default until ``check()`` has passed,
        which makes the safe order the only order a caller can take.
        """
        if require_checked and not self._checked:
            raise PackError(
                "refusing to read points before check() has passed — call "
                "check() first (payload tampering is caught only by its sha256)")
        rule = self.id_rule
        dim = self.dim
        stride = dim * ITEM_SIZE
        seen = 0
        with self._zf.open(POINTS) as pfh, self._zf.open(VECTORS) as vfh:
            text = io.TextIOWrapper(pfh, encoding="utf-8")
            points: list[dict] = []
            while True:
                line = text.readline()
                if not line:
                    break
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                except json.JSONDecodeError as exc:
                    raise PackError(f"{POINTS} line {seen + 1}: {exc}") from exc
                if verify_ids:
                    want = contract.point_id(rule, _fields(self.profile, rec["payload"],
                                                           rec["uid"]))
                    if rec.get("id") != want:
                        raise PackError(
                            f"{POINTS} line {seen + 1}: id {rec.get('id')} does not match "
                            f"{rule} of uid {rec['uid']!r} (expected {want}) — pack is "
                            f"corrupt or was edited by hand")
                points.append(rec)
                seen += 1
                if len(points) >= size:
                    raw = vfh.read(stride * len(points))
                    if len(raw) != stride * len(points):
                        raise PackError(f"{VECTORS} ran out after {seen} points")
                    yield points, _f32_read(raw, dim)
                    points = []
            if points:
                raw = vfh.read(stride * len(points))
                if len(raw) != stride * len(points):
                    raise PackError(f"{VECTORS} ran out after {seen} points")
                yield points, _f32_read(raw, dim)
        if seen != self.count:
            raise PackError(f"{POINTS} holds {seen} points, manifest declares {self.count}")

    def close(self) -> None:
        self._zf.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
        return False

""".sopack container — reader and writer.

A .sopack is a ZIP holding::

    manifest.json   contract, counts, checksums, canary probe
    points.jsonl    one JSON object per point, in vector order
    vectors.f32     N x dim x 4 bytes, little-endian float32, same order
    probe.f32       the canary vectors referenced by manifest["probe"]
    titles.json     additive title-table fragment (sop profile only)

**Stdlib only** — imported by the server. Vectors are raw float32 precisely so
the server never converts anything: bytes go from the zip into
``array.array('f')`` and straight into the upsert body. See the plan's §4.3 for
why that beats float16 and inline JSON arrays on a small machine.

Both sides stream: ``PackWriter`` never holds more than one point, and
``PackReader.batches()`` never holds more than one batch, so peak RAM is
independent of pack size.
"""

from __future__ import annotations

import array
import hashlib
import io
import json
import os
import sys
import tempfile
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


def _f32_bytes(vector) -> bytes:
    """One vector as little-endian float32 bytes."""
    buf = array.array("f", vector)
    if sys.byteorder != "little":
        buf.byteswap()
    return buf.tobytes()


def _f32_read(raw: bytes, dim: int) -> list[list[float]]:
    """A run of little-endian float32 bytes back into vectors of *dim*."""
    buf = array.array("f")
    buf.frombytes(raw)
    if sys.byteorder != "little":
        buf.byteswap()
    n = len(buf) // dim
    return [buf[i * dim:(i + 1) * dim].tolist() for i in range(n)]


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
                 id_rule: str | None = None, dim: int = contract.VECTOR_SIZE):
        self.path = Path(path)
        self.profile = contract.get_profile(profile)
        self.pack_id = pack_id
        self.created_by = created_by
        self.id_rule = id_rule or self.profile.default_id_rule
        self.dim = dim
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

    def set_probe(self, canaries: list[dict], vectors: list[list[float]]) -> None:
        """*canaries* are ``{"id", "collection"}`` of points ALREADY in the live
        collection; *vectors* are this machine's fresh embeddings of their text,
        in the same order. The server cosines the two (§4.4)."""
        if len(canaries) != len(vectors):
            raise PackError("probe: canary/vector count mismatch")
        self._probe = {
            "canaries": [
                {**c, "vector_offset": i, "cosine_expected_min": contract.PROBE_MIN_COSINE}
                for i, c in enumerate(canaries)
            ],
            "vectors": PROBE,
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
            "target": {
                "collection": self.profile.collection,
                "vector_name": contract.VECTOR_NAME,
                "vector_size": contract.VECTOR_SIZE,
                "distance": contract.DISTANCE,
            },
            "embedding": dict(contract.EMBEDDING),
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
        """Everything verifiable without touching Qdrant. Empty list is clean."""
        errors = []
        if self.manifest.get("schema") != contract.SCHEMA_PACK:
            errors.append(f"unsupported schema {self.manifest.get('schema')!r} "
                          f"(this server reads {contract.SCHEMA_PACK!r})")
            return errors
        try:
            profile = self.profile
        except ValueError as exc:
            return [str(exc)]

        errors += contract.check_embedding(self.manifest.get("embedding") or {})
        errors += contract.check_target(self.manifest.get("target") or {}, profile)

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

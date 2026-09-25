"""Store adapters — everything backend-specific about the corpus import
pipeline, behind one interface (SOPACK-AUTONOMY.md §3.2/§3.3).

``sopack/`` (the packer) knows nothing about a vector store at all — see
``sopack/tests/test_neutrality.py``. Everything a *pack* needs to become
points in a live collection — the collection name, the Qdrant named-vector
name, its payload index types, the "Cosine" spelling — is backend
configuration that lives here, on the importer side, not in the neutral
contract.

Two adapters:

* :class:`QdrantAdapter` — today's behaviour (``app/import_service.py``
  before this split), reimplemented as methods instead of module functions.
  Talks HTTP + JSON to a live (or fake, in tests) Qdrant.
* :class:`InMemoryAdapter` — no network at all; a plain-Python store used by
  tests to prove a ``.sopack`` imports the same way into a second backend
  (SOPACK-AUTONOMY.md §5.5). Never used in production.

Both implement :class:`StoreAdapter`. ``app/import_service.py`` drives a job
entirely through ``ctx.adapter`` — it does not know which one it has.

Backup (Qdrant's snapshot API — R2) is deliberately **not** part of this
interface for M1: it stays where it already lived, ``app/snapshots.py``,
called directly by ``import_service._stage_snapshot`` only in Qdrant's own
``apply`` mode. SOPACK-AUTONOMY.md §3.3 notes Chroma has no equivalent
snapshot API at all ("copy the persistent directory"), so backup is left as a
Qdrant-specific concern until a second backend is real, rather than forcing
a shape on an operation nothing else implements yet.
"""

from __future__ import annotations

import os
import threading
from typing import Protocol

import requests

# ── Qdrant adapter config (SOPACK-AUTONOMY.md §3.2) ──────────────────────────
#
# Exactly what used to live in sopack/contract.py's Profile.collection /
# Profile.indexes and the module-level VECTOR_NAME/DISTANCE constants. This is
# the ONE place in the whole server that says "sop" lives in a collection
# called "sop" — everything else (sopack, the neutral contract) only knows
# the profile *name*.

QDRANT_VECTOR_NAME = "fast-multilingual-e5-large"
QDRANT_DISTANCE = "Cosine"

QDRANT_PROFILE_CONFIG = {
    "sop": {
        "collection": "sop",
        "indexes": {"lang": "keyword", "book_code": "keyword", "page": "integer"},
    },
    "bible": {
        "collection": "bibles",
        "indexes": {"bible": "keyword", "osis": "keyword"},
    },
}


def collection_for(profile_name: str) -> str:
    try:
        return QDRANT_PROFILE_CONFIG[profile_name]["collection"]
    except KeyError:
        raise ValueError(f"no Qdrant collection configured for profile {profile_name!r}") from None


def indexes_for(profile_name: str) -> dict:
    try:
        return dict(QDRANT_PROFILE_CONFIG[profile_name]["indexes"])
    except KeyError:
        raise ValueError(f"no Qdrant indexes configured for profile {profile_name!r}") from None


class AdapterError(Exception):
    """A store operation failed, or the store is not in the state a stage
    requires (e.g. a collection is missing)."""


# ── the interface ────────────────────────────────────────────────────────────

class StoreAdapter(Protocol):
    """What ``import_service.py`` needs from a backend. One call per row in
    SOPACK-AUTONOMY.md §3.3's table, plus the small extras M1 needs (identity
    probe, retrievability, contract fingerprint for an empty store)."""

    def ensure_collection(self, profile_name: str, dim: int) -> None: ...

    def create_scratch_collection(self, collection: str, dim: int) -> None: ...

    def delete_collection(self, collection: str) -> None: ...

    def retrieve(self, collection: str, ids: list[str], *, with_payload: bool,
                 with_vector: bool) -> list[dict]: ...
    """Returns ``[{"id"[, "payload"][, "vector"]}, ...]`` for whichever of
    *ids* exist — a plain ``list[float]`` vector, never a named-vector dict;
    the adapter unwraps that (SOPACK-AUTONOMY.md §3.3: flattening a store's
    own shape is the adapter's job)."""

    def upsert(self, collection: str, points: list[dict]) -> None: ...
    """*points*: ``[{"id", "payload", "vector": [float, ...]}, ...]``."""

    def delete_points(self, collection: str, ids: list[str]) -> None: ...

    def scroll(self, collection: str, flt: dict | None, limit: int, *,
               with_payload: bool) -> list[dict]: ...

    def count(self, collection: str, flt: dict | None) -> int: ...

    def ensure_index(self, collection: str, field: str, schema: str) -> None: ...

    def get_fingerprint(self, collection: str) -> str | None: ...

    def set_fingerprint(self, collection: str, sha256: str) -> None: ...


# ── Qdrant adapter ───────────────────────────────────────────────────────────

class QdrantAdapter:
    """Plain ``requests`` calls against a live (or fake) Qdrant, matching
    ``app/sop_tools.py``'s style — no ``qdrant_client``. Behaviour is
    byte-for-byte what ``app/import_service.py`` did before this module
    existed: every call, body and timeout is unchanged, just reorganised as
    methods."""

    def __init__(self, base_url: str, *, timeout_s: float = 60.0,
                 upsert_timeout_s: float = 120.0,
                 fingerprint_path: str | None = None):
        self.base_url = base_url.rstrip("/")
        self.timeout_s = timeout_s
        self.upsert_timeout_s = upsert_timeout_s
        self._fingerprint_path = fingerprint_path or os.environ.get(
            "CONTRACT_FINGERPRINTS_JSON", "/data/contract_fingerprints.json")

    def _collection_url(self, collection: str, path: str = "") -> str:
        url = f"{self.base_url}/collections/{collection}"
        return f"{url}/{path}" if path else url

    def _post(self, collection: str, path: str, body: dict, timeout: float | None = None):
        try:
            r = requests.post(self._collection_url(collection, path), json=body,
                              timeout=timeout or self.timeout_s)
        except requests.RequestException as exc:
            raise AdapterError(f"POST {collection}/{path}: {exc}") from exc
        if r.status_code >= 300:
            raise AdapterError(f"POST {collection}/{path}: HTTP {r.status_code} {r.text[:400]}")
        return (r.json() or {}).get("result")

    def _put(self, collection: str, path: str, body: dict, params: dict | None = None,
             timeout: float | None = None):
        try:
            r = requests.put(self._collection_url(collection, path), json=body, params=params,
                             timeout=timeout or self.timeout_s)
        except requests.RequestException as exc:
            raise AdapterError(f"PUT {collection}/{path}: {exc}") from exc
        if r.status_code >= 300:
            raise AdapterError(f"PUT {collection}/{path}: HTTP {r.status_code} {r.text[:400]}")
        return (r.json() or {}).get("result")

    # ── collection lifecycle ────────────────────────────────────────────────

    def ensure_collection(self, profile_name: str, dim: int) -> None:
        collection = collection_for(profile_name)
        try:
            r = requests.get(self._collection_url(collection), timeout=self.timeout_s)
        except requests.RequestException as exc:
            raise AdapterError(f"GET {collection}: {exc}") from exc
        if r.status_code == 404:
            raise AdapterError(f"collection {collection!r} does not exist on {self.base_url}")
        if r.status_code >= 300:
            raise AdapterError(f"GET {collection}: HTTP {r.status_code} {r.text[:400]}")
        info = (r.json() or {}).get("result") or {}
        vectors_cfg = ((info.get("config") or {}).get("params") or {}).get("vectors") or {}
        vec = vectors_cfg.get(QDRANT_VECTOR_NAME)
        if not vec:
            raise AdapterError(f"collection {collection!r} has no vector named "
                               f"{QDRANT_VECTOR_NAME!r}")
        problems = []
        if int(vec.get("size", -1)) != dim:
            problems.append(f"vector size {vec.get('size')} != {dim}")
        if str(vec.get("distance", "")) != QDRANT_DISTANCE:
            problems.append(f"distance {vec.get('distance')!r} != {QDRANT_DISTANCE!r}")
        if problems:
            raise AdapterError("; ".join(problems))

    def create_scratch_collection(self, collection: str, dim: int) -> None:
        body = {"vectors": {QDRANT_VECTOR_NAME: {"size": dim, "distance": QDRANT_DISTANCE}}}
        try:
            r = requests.put(self._collection_url(collection), json=body, timeout=self.timeout_s)
        except requests.RequestException as exc:
            raise AdapterError(f"PUT {collection}: {exc}") from exc
        if r.status_code >= 300:
            raise AdapterError(f"create collection {collection}: HTTP {r.status_code} {r.text[:400]}")

    def delete_collection(self, collection: str) -> None:
        try:
            requests.delete(self._collection_url(collection), timeout=self.timeout_s)
        except requests.RequestException:
            pass  # best-effort: scratch-collection cleanup only

    # ── points ───────────────────────────────────────────────────────────────

    def retrieve(self, collection: str, ids: list[str], *, with_payload: bool,
                 with_vector: bool) -> list[dict]:
        if not ids:
            return []
        result = self._post(collection, "points",
                            {"ids": ids, "with_payload": with_payload, "with_vector": with_vector})
        out = []
        for p in (result or []):
            row = {"id": str(p["id"])}
            if with_payload:
                row["payload"] = p.get("payload") or {}
            if with_vector:
                vec = p.get("vector")
                if isinstance(vec, dict):
                    vec = vec.get(QDRANT_VECTOR_NAME)
                row["vector"] = vec
            out.append(row)
        return out

    def upsert(self, collection: str, points: list[dict]) -> None:
        body_points = [{"id": p["id"], "vector": {QDRANT_VECTOR_NAME: p["vector"]},
                        "payload": p["payload"]} for p in points]
        self._put(collection, "points", {"points": body_points}, params={"wait": "true"},
                 timeout=self.upsert_timeout_s)

    def delete_points(self, collection: str, ids: list[str]) -> None:
        if not ids:
            return
        self._post(collection, "points/delete", {"points": ids}, timeout=self.upsert_timeout_s)

    def scroll(self, collection: str, flt: dict | None, limit: int, *,
              with_payload: bool) -> list[dict]:
        body = {"limit": limit, "with_payload": with_payload, "with_vector": False}
        if flt:
            body["filter"] = flt
        result = self._post(collection, "points/scroll", body)
        points = (result or {}).get("points") or []
        out = []
        for p in points:
            row = {"id": str(p["id"])}
            if with_payload:
                row["payload"] = p.get("payload") or {}
            out.append(row)
        return out

    def count(self, collection: str, flt: dict | None) -> int:
        body = {"exact": True}
        if flt:
            body["filter"] = flt
        result = self._post(collection, "points/count", body)
        return int((result or {}).get("count", 0))

    def ensure_index(self, collection: str, field: str, schema: str) -> None:
        self._put(collection, "index", {"field_name": field, "field_schema": schema},
                 params={"wait": "true"})

    # ── contract fingerprint (SOPACK-2-FORMAT.md §4 step 3) ─────────────────
    #
    # "the importer's own job DB" (SOPACK-AUTONOMY.md §3.1's design table) — a
    # small local JSON file, not a write to the vector store itself (Qdrant
    # has no natural place for this that would not itself need a schema
    # migration; a future Chroma adapter can do the same).

    _fp_lock = threading.Lock()

    def _read_fingerprints(self) -> dict:
        import json
        try:
            with open(self._fingerprint_path, encoding="utf-8") as fh:
                return json.load(fh)
        except (FileNotFoundError, ValueError):
            return {}

    def get_fingerprint(self, collection: str) -> str | None:
        return self._read_fingerprints().get(collection)

    def set_fingerprint(self, collection: str, sha256: str) -> None:
        import json
        with self._fp_lock:
            data = self._read_fingerprints()
            data[collection] = sha256
            os.makedirs(os.path.dirname(self._fingerprint_path) or ".", exist_ok=True)
            tmp = self._fingerprint_path + ".tmp"
            with open(tmp, "w", encoding="utf-8") as fh:
                json.dump(data, fh, indent=2, sort_keys=True)
            os.replace(tmp, self._fingerprint_path)


# ── in-memory adapter (tests only) ───────────────────────────────────────────

class InMemoryAdapter:
    """A plain-Python store with the same method surface as
    :class:`QdrantAdapter`, backed by nothing but dicts. Proves a ``.sopack``
    imports into a *different* backend without changing shape
    (SOPACK-AUTONOMY.md §5.5) — never used outside tests, and adds no
    dependency (no chromadb)."""

    def __init__(self):
        self.collections: dict[str, dict] = {}
        self._fingerprints: dict[str, str] = {}

    # test-side seeding, mirroring app/tests/fake_qdrant.py's helpers

    def seed_collection(self, collection: str, dim: int) -> None:
        self.collections.setdefault(collection, {"dim": dim, "points": {}})

    def put_point(self, collection: str, point_id: str, payload: dict,
                 vector: list[float]) -> None:
        c = self.collections.setdefault(collection, {"dim": len(vector), "points": {}})
        c["points"][str(point_id)] = {"payload": dict(payload), "vector": list(vector)}

    def points(self, collection: str) -> dict:
        return self.collections.get(collection, {}).get("points", {})

    @staticmethod
    def _match(payload: dict, flt: dict | None) -> bool:
        if not flt:
            return True
        for cond in flt.get("must", []):
            key = cond["key"]
            val = payload.get(key)
            m = cond.get("match")
            if m is not None:
                if "value" in m and val != m["value"]:
                    return False
                if "any" in m and val not in m["any"]:
                    return False
        return True

    def ensure_collection(self, profile_name: str, dim: int) -> None:
        collection = collection_for(profile_name)
        c = self.collections.get(collection)
        if c is None:
            raise AdapterError(f"collection {collection!r} does not exist (in-memory store)")
        if c["dim"] != dim:
            raise AdapterError(f"vector size {c['dim']} != {dim}")

    def create_scratch_collection(self, collection: str, dim: int) -> None:
        self.collections[collection] = {"dim": dim, "points": {}}

    def delete_collection(self, collection: str) -> None:
        self.collections.pop(collection, None)

    def retrieve(self, collection: str, ids: list[str], *, with_payload: bool,
                 with_vector: bool) -> list[dict]:
        pts = self.points(collection)
        out = []
        for pid in ids:
            p = pts.get(str(pid))
            if p is None:
                continue
            row = {"id": str(pid)}
            if with_payload:
                row["payload"] = p["payload"]
            if with_vector:
                row["vector"] = p["vector"]
            out.append(row)
        return out

    def upsert(self, collection: str, points: list[dict]) -> None:
        c = self.collections.setdefault(collection, {"dim": None, "points": {}})
        for p in points:
            c["points"][str(p["id"])] = {"payload": dict(p["payload"]), "vector": list(p["vector"])}

    def delete_points(self, collection: str, ids: list[str]) -> None:
        pts = self.points(collection)
        for pid in ids:
            pts.pop(str(pid), None)

    def scroll(self, collection: str, flt: dict | None, limit: int, *,
              with_payload: bool) -> list[dict]:
        out = []
        for pid, p in self.points(collection).items():
            if not self._match(p["payload"], flt):
                continue
            row = {"id": pid}
            if with_payload:
                row["payload"] = p["payload"]
            out.append(row)
            if len(out) >= limit:
                break
        return out

    def count(self, collection: str, flt: dict | None) -> int:
        return sum(1 for p in self.points(collection).values() if self._match(p["payload"], flt))

    def ensure_index(self, collection: str, field: str, schema: str) -> None:
        pass  # no index types to create in a plain dict store

    def get_fingerprint(self, collection: str) -> str | None:
        return self._fingerprints.get(collection)

    def set_fingerprint(self, collection: str, sha256: str) -> None:
        self._fingerprints[collection] = sha256

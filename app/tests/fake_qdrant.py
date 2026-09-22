"""In-process fake Qdrant REST server for tests.

Implements exactly the calls ``app/import_service.py`` and ``app/snapshots.py``
make — collection get/create/delete, points retrieve/upsert/delete/scroll/
count, facet, payload index, snapshot create/list/delete/recover — as an
in-memory approximation. It is not a real Qdrant; it exists to exercise our
request shapes and stage logic without a network dependency or qdrant_client.

Not a test module itself (no ``test_`` prefix), so unittest discovery skips it.
"""

from __future__ import annotations

import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit


class _NotFound(Exception):
    pass


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
        rng = cond.get("range")
        if rng is not None:
            if val is None:
                return False
            if "gte" in rng and not (val >= rng["gte"]):
                return False
            if "lte" in rng and not (val <= rng["lte"]):
                return False
    return True


class FakeQdrant:
    """Starts a real HTTP server on 127.0.0.1:<random port> backed by an
    in-memory store. ``.url`` is what you set ``QDRANT_URL`` to."""

    def __init__(self):
        self.collections: dict[str, dict] = {}
        self.snapshots: dict[str, dict] = {}
        self._lock = threading.Lock()
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), _make_handler(self))
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self._server.server_address[1]}"

    def stop(self) -> None:
        self._server.shutdown()
        self._server.server_close()

    # ── test-side seeding helpers ────────────────────────────────────────────

    def create_collection(self, name: str, vector_name: str, size: int = 1024,
                          distance: str = "Cosine") -> None:
        with self._lock:
            self.collections[name] = {"vectors": {vector_name: {"size": size,
                                                                 "distance": distance}},
                                      "points": {}}

    def put_point(self, collection: str, point_id: str, payload: dict, vector: list[float]) -> None:
        with self._lock:
            self.collections.setdefault(collection, {"vectors": {}, "points": {}})
            self.collections[collection]["points"][str(point_id)] = {
                "payload": dict(payload), "vector": list(vector)}

    def points(self, collection: str) -> dict:
        return self.collections.get(collection, {}).get("points", {})

    def snapshot_of(self, name: str) -> str | None:
        """Convenience: dump a collection's points as a comparable snapshot."""
        pts = self.points(name)
        return json.dumps({pid: {"payload": p["payload"], "vector": p["vector"]}
                           for pid, p in pts.items()}, sort_keys=True)


def _dispatch(store: FakeQdrant, method: str, segs: list[str], body: dict):
    if not segs or segs[0] != "collections" or len(segs) < 2:
        raise _NotFound("unknown route")
    coll = segs[1]
    rest = segs[2:]

    with store._lock:
        if not rest:
            if method == "GET":
                c = store.collections.get(coll)
                if c is None:
                    raise _NotFound("collection not found")
                return {"config": {"params": {"vectors": c["vectors"]}}}, 200
            if method == "PUT":
                store.collections[coll] = {"vectors": dict(body.get("vectors") or {}), "points": {}}
                return {"acknowledged": True}, 200
            if method == "DELETE":
                store.collections.pop(coll, None)
                store.snapshots.pop(coll, None)
                return {"acknowledged": True}, 200
            raise _NotFound("method not allowed")

        c = store.collections.setdefault(coll, {"vectors": {}, "points": {}})
        vec_name = next(iter(c["vectors"]), None)

        if rest == ["points"] and method == "POST":
            ids = [str(i) for i in body.get("ids", [])]
            with_payload = body.get("with_payload", True)
            with_vector = body.get("with_vector", False)
            out = []
            for pid in ids:
                p = c["points"].get(pid)
                if p is None:
                    continue
                row = {"id": pid}
                if with_payload:
                    row["payload"] = p["payload"]
                if with_vector:
                    row["vector"] = {vec_name: p["vector"]} if vec_name else p["vector"]
                out.append(row)
            return out, 200

        if rest == ["points"] and method == "PUT":
            for pt in body.get("points", []):
                pid = str(pt["id"])
                vec = pt.get("vector")
                if isinstance(vec, dict):
                    vec = vec.get(vec_name) if vec_name else next(iter(vec.values()), None)
                c["points"][pid] = {"payload": pt.get("payload") or {}, "vector": vec}
            return {"status": "acknowledged"}, 200

        if rest == ["points", "delete"] and method == "POST":
            for pid in body.get("points", []):
                c["points"].pop(str(pid), None)
            return {"status": "acknowledged"}, 200

        if rest == ["points", "scroll"] and method == "POST":
            flt = body.get("filter")
            limit = int(body.get("limit", 10))
            matched = [{"id": pid, "payload": p["payload"]}
                      for pid, p in c["points"].items() if _match(p["payload"], flt)]
            return {"points": matched[:limit], "next_page_offset": None}, 200

        if rest == ["points", "count"] and method == "POST":
            flt = body.get("filter")
            n = sum(1 for p in c["points"].values() if _match(p["payload"], flt))
            return {"count": n}, 200

        if rest == ["facet"] and method == "POST":
            key = body["key"]
            flt = body.get("filter")
            limit = int(body.get("limit", 1000))
            counts: dict = {}
            for p in c["points"].values():
                if not _match(p["payload"], flt):
                    continue
                v = p["payload"].get(key)
                if v is None:
                    continue
                counts[v] = counts.get(v, 0) + 1
            hits = [{"value": v, "count": n} for v, n in sorted(counts.items(),
                                                                key=lambda kv: str(kv[0]))][:limit]
            return {"hits": hits}, 200

        if rest == ["index"] and method == "PUT":
            return {"status": "acknowledged"}, 200

        if rest == ["snapshots"] and method == "POST":
            name = f"{coll}-{len(store.snapshots.get(coll, {})) + 1}.snapshot"
            snap_points = {pid: {"payload": dict(p["payload"]), "vector": list(p["vector"])}
                          for pid, p in c["points"].items()}
            entry = {"points": snap_points, "vectors": dict(c["vectors"]),
                     "creation_time": f"{time.time():.6f}"}
            store.snapshots.setdefault(coll, {})[name] = entry
            return {"name": name, "creation_time": entry["creation_time"],
                    "size": len(snap_points)}, 200

        if rest == ["snapshots"] and method == "GET":
            snaps = store.snapshots.get(coll, {})
            return [{"name": n, "creation_time": s["creation_time"], "size": len(s["points"])}
                    for n, s in snaps.items()], 200

        if len(rest) == 2 and rest[0] == "snapshots" and method == "DELETE":
            store.snapshots.get(coll, {}).pop(rest[1], None)
            return {"status": "acknowledged"}, 200

        if rest == ["snapshots", "recover"] and method == "PUT":
            location = body.get("location", "")
            name = location.rstrip("/").rsplit("/", 1)[-1]
            snap = store.snapshots.get(coll, {}).get(name)
            if snap is None:
                raise _NotFound(f"snapshot not found: {name}")
            c["points"] = {pid: {"payload": dict(p["payload"]), "vector": list(p["vector"])}
                          for pid, p in snap["points"].items()}
            c["vectors"] = dict(snap["vectors"])
            return {"status": "acknowledged"}, 200

    raise _NotFound(f"unhandled route: {method} /{'/'.join(segs)}")


def _make_handler(store: FakeQdrant):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def _body(self) -> dict:
            length = int(self.headers.get("Content-Length", 0) or 0)
            raw = self.rfile.read(length) if length else b""
            return json.loads(raw) if raw else {}

        def _reply(self, status: int, result) -> None:
            payload = json.dumps({"result": result, "status": "ok", "time": 0}).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def _error(self, status: int, msg: str) -> None:
            payload = json.dumps({"status": {"error": msg}}).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def _route(self, method: str) -> None:
            segs = [s for s in urlsplit(self.path).path.split("/") if s]
            body = self._body() if method in ("POST", "PUT") else {}
            try:
                result, status = _dispatch(store, method, segs, body)
            except _NotFound as exc:
                self._error(404, str(exc))
                return
            except Exception as exc:
                self._error(400, str(exc))
                return
            self._reply(status, result)

        def do_GET(self):
            self._route("GET")

        def do_POST(self):
            self._route("POST")

        def do_PUT(self):
            self._route("PUT")

        def do_DELETE(self):
            self._route("DELETE")

    return Handler

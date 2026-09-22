#!/usr/bin/env python3
"""Payload index, filtered copy and filtered delete for the `sop` collection.

Stdlib only (urllib) — runs anywhere, no qdrant-client, no venv.

**Read this before splitting anything.** You probably do not need to. Every
pioneer point carries ``corpus: "pioneers"`` and every Ellen White point
carries no ``corpus`` key at all, so one filter already separates them:

    EGW only        {"must_not": [{"key": "corpus", "match": {"value": "pioneers"}}]}
    pioneers only   {"must":     [{"key": "corpus", "match": {"value": "pioneers"}}]}

That filter is free once ``corpus`` has a payload index (``index-payload``
below), it keeps one ranked result list across both corpora, and it needs no
change to ``_COLLECTION`` in ``sop_tools.py``. Two collections means two
queries and a merge you have to rank yourself.

The one good reason to split is governance: if a separate collection is how
you want to guarantee that a Spirit-of-Prophecy query can never reach a
pioneer paragraph, the filter is a promise in code and the split is a promise
in infrastructure. That is a real difference. This script makes the split
cheap either way.

**A copy does not re-embed.** Vectors are read from the source and written to
the target unchanged, so a copy is a network transfer, not a GPU job. The 49
pioneer works are about 60,000 points and copy in minutes.

Commands::

    # 1. make the corpus filter fast (do this whatever you decide)
    python3 split_corpus.py index-payload --execute

    # 2. see what a split would move
    python3 split_corpus.py copy --to pioneers --pioneers
    python3 split_corpus.py copy --to pioneers --pioneers --execute

    # 3. only after verifying the copy, prune the source
    python3 split_corpus.py count --pioneers --collection pioneers
    python3 split_corpus.py delete --pioneers --execute --yes-i-verified-the-copy

Every command is a dry run unless ``--execute`` is passed. ``delete`` also
requires the long flag, and refuses to run if the target collection does not
already hold at least as many matching points as the source.

Set ``QDRANT_URL`` or pass ``--qdrant-url``.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request

PIONEER_FILTER = {"must": [{"key": "corpus", "match": {"value": "pioneers"}}]}
EGW_FILTER = {"must_not": [{"key": "corpus", "match": {"value": "pioneers"}}]}


class QdrantError(Exception):
    """An HTTP error from Qdrant, carrying its status code."""

    def __init__(self, code: int, msg: str):
        super().__init__(msg)
        self.code = code


def req(method: str, url: str, body: dict | None = None, timeout: float = 120.0) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method,
                               headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=timeout) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        raise QdrantError(e.code, f"{method} {url} -> {e.code}\n{e.read().decode()[:800]}") from None
    except urllib.error.URLError as e:
        # Unreachable host must never be mistaken for "the collection is missing".
        raise SystemExit(f"cannot reach Qdrant at {url}: {e.reason}") from None


def count(base: str, coll: str, flt: dict | None) -> int:
    body = {"exact": True}
    if flt:
        body["filter"] = flt
    return req("POST", f"{base}/collections/{coll}/points/count", body)["result"]["count"]


def collection_config(base: str, coll: str) -> dict:
    return req("GET", f"{base}/collections/{coll}")["result"]


# --------------------------------------------------------------------- commands

def cmd_index_payload(a, base: str) -> int:
    """Create the keyword payload index on `corpus`, without which every
    corpus-filtered search is a full scan of 840 k points."""
    print(f"payload index: {a.collection}.{a.field} (keyword)")
    if not a.execute:
        print("[dry-run] pass --execute to create it")
        return 0
    req("PUT", f"{base}/collections/{a.collection}/index?wait=true",
        {"field_name": a.field, "field_schema": "keyword"})
    print("created")
    return 0


def cmd_count(a, base: str) -> int:
    flt = pick_filter(a)
    total = count(base, a.collection, None)
    match = count(base, a.collection, flt)
    print(f"{a.collection}: {total:,} points total, {match:,} match the filter")
    return 0


def cmd_copy(a, base: str) -> int:
    flt = pick_filter(a)
    src_cfg = collection_config(base, a.collection)
    vectors = src_cfg["config"]["params"]["vectors"]
    n = count(base, a.collection, flt)
    print(f"source {a.collection}: {n:,} matching points")
    print(f"target {a.to}: vectors {json.dumps(vectors)[:120]}")

    exists = True
    try:
        collection_config(base, a.to)
    except QdrantError as e:
        if e.code != 404:
            raise
        exists = False
    print(f"target exists: {exists}")

    if not a.execute:
        print("[dry-run] would create the target if missing, then copy "
              f"{n:,} points with their vectors (no re-embedding)")
        return 0

    if not exists:
        req("PUT", f"{base}/collections/{a.to}", {"vectors": vectors})
        print(f"created collection {a.to}")
        # The filters sop_tools uses need these; mirror them on the copy.
        for field in ("lang", "book_code", "page", "corpus"):
            req("PUT", f"{base}/collections/{a.to}/index?wait=true",
                {"field_name": field,
                 "field_schema": "integer" if field == "page" else "keyword"})
        print("created payload indexes: lang, book_code, page, corpus")

    moved, offset, t0 = 0, None, time.monotonic()
    while True:
        body = {"limit": a.batch, "with_payload": True, "with_vector": True,
                "filter": flt}
        if offset is not None:
            body["offset"] = offset
        res = req("POST", f"{base}/collections/{a.collection}/points/scroll", body)["result"]
        pts = res.get("points") or []
        if not pts:
            break
        req("PUT", f"{base}/collections/{a.to}/points?wait=true",
            {"points": [{"id": p["id"], "vector": p["vector"], "payload": p["payload"]}
                        for p in pts]})
        moved += len(pts)
        rate = moved / max(time.monotonic() - t0, 1e-9)
        print(f"  {moved:,}/{n:,}  {rate:.0f} pts/s", flush=True)
        offset = res.get("next_page_offset")
        if offset is None:
            break

    final = count(base, a.to, flt)
    print(f"DONE copied {moved:,}; target now holds {final:,} matching points")
    if final < n:
        print("WARNING: target count is below source count — do NOT delete from the source")
        return 1
    return 0


def cmd_delete(a, base: str) -> int:
    flt = pick_filter(a)
    n = count(base, a.collection, flt)
    print(f"{a.collection}: {n:,} points match the filter")

    if a.verify_in:
        try:
            mirrored = count(base, a.verify_in, flt)
        except QdrantError as e:
            if e.code != 404:
                raise
            raise SystemExit(f"refusing to delete: collection {a.verify_in!r} "
                             "does not exist, so there is no copy") from None
        print(f"{a.verify_in}: {mirrored:,} matching points")
        if mirrored < n:
            raise SystemExit("refusing to delete: the copy holds fewer matching "
                             f"points ({mirrored:,}) than the source ({n:,})")
    else:
        print("no --verify-in given: nothing is checking that a copy exists")

    if not (a.execute and a.yes_i_verified_the_copy):
        print("[dry-run] pass --execute --yes-i-verified-the-copy to delete")
        return 0
    req("POST", f"{base}/collections/{a.collection}/points/delete?wait=true",
        {"filter": flt})
    print(f"deleted; {count(base, a.collection, flt):,} matching points remain")
    return 0


# --------------------------------------------------------------------- plumbing

def pick_filter(a) -> dict:
    if a.pioneers and a.egw:
        raise SystemExit("--pioneers and --egw are mutually exclusive")
    if a.filter:
        return json.loads(a.filter)
    if a.pioneers:
        return PIONEER_FILTER
    if a.egw:
        return EGW_FILTER
    raise SystemExit("give --pioneers, --egw or --filter '<json>'")


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    # Defined on every subparser rather than the top level, so they can be
    # written after the command, which is where anyone will type them.
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--qdrant-url",
                        default=os.environ.get("QDRANT_URL", "http://localhost:6333"))
    common.add_argument("--collection", default="sop")
    common.add_argument("--execute", action="store_true", help="act; otherwise dry run")

    def with_filter(p):
        p.add_argument("--pioneers", action="store_true")
        p.add_argument("--egw", action="store_true")
        p.add_argument("--filter", help="raw Qdrant filter JSON, overrides the flags")
        return p

    p = sub.add_parser("index-payload", parents=[common], help="create the keyword index on `corpus`")
    p.add_argument("--field", default="corpus")
    p.set_defaults(fn=cmd_index_payload)

    p = with_filter(sub.add_parser("count", parents=[common], help="count matching points"))
    p.set_defaults(fn=cmd_count)

    p = with_filter(sub.add_parser("copy", parents=[common], help="copy matching points, vectors included"))
    p.add_argument("--to", required=True, help="target collection")
    p.add_argument("--batch", type=int, default=512)
    p.set_defaults(fn=cmd_copy)

    p = with_filter(sub.add_parser("delete", parents=[common], help="delete matching points from --collection"))
    p.add_argument("--verify-in", help="collection that must already hold the copy")
    p.add_argument("--yes-i-verified-the-copy", action="store_true")
    p.set_defaults(fn=cmd_delete)

    a = ap.parse_args()
    try:
        return a.fn(a, a.qdrant_url.rstrip("/"))
    except QdrantError as e:
        raise SystemExit(str(e)) from None


if __name__ == "__main__":
    sys.exit(main())

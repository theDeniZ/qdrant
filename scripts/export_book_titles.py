#!/usr/bin/env python3
"""Export SoP book code → title tables for ``sop_list_books``.

``sop_list_books`` does **not** read titles from Qdrant. It reads this static
table, so any book indexed straight into the collection without a matching
entry here comes back with ``"titles": []``. That is what happened to the
pioneer import: the payloads carry ``title`` / ``author`` / ``year``, but this
file was built only from the generator's local mirror, which has no pioneer
files. Hence 48 untitled codes.

Three sources, all optional and merged in this order (later wins only for
fields it actually provides):

1. **generator mirror** (positional ``SOP_DIR``) — ``book_map.json`` plus the
   ``en/<CODE>.json`` meta blocks. The EGW corpus.
2. **corpus JSONL** (``--merge``) — a build artifact such as
   ``pd-books/qdrant/pioneers_corpus.jsonl``: one JSON record per line with a
   ``payload`` holding ``lang``, ``book_code``, ``title``, ``author``,
   ``year``, ``corpus``. Offline, no network.
3. **live Qdrant** (``--qdrant-url``) — facet ``book_code`` per language and
   read one point per code. Source-agnostic: it picks up anything anyone has
   ever indexed, whatever pipeline wrote it.

Writes ``app/data/sop_books.json``::

    {"en": {"SC":  {"titles": ["Steps to Christ"]},
            "DAR": {"titles": ["Daniel and the Revelation"],
                    "author": "Uriah Smith", "year": 1882, "corpus": "pioneers"}},
     "de": {"BW": {"titles": ["Der bessere Weg ..."], "en_code": "SC"}, ...}}

``author`` / ``year`` / ``corpus`` are additive; readers that only know
``titles`` and ``en_code`` are unaffected.

Examples::

    # EGW only, as before
    python scripts/export_book_titles.py /path/to/generator/data/sop

    # EGW + the pioneer import, fully offline
    python scripts/export_book_titles.py /path/to/generator/data/sop \\
        --merge ../qdrant/pd-books/qdrant/pioneers_corpus.jsonl

    # whatever is actually in the collection right now
    python scripts/export_book_titles.py /path/to/generator/data/sop \\
        --qdrant-url http://10.10.10.10:6333
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "app" / "data" / "sop_books.json"


# --------------------------------------------------------------------------- table

def make_adder(out: dict[str, dict[str, dict]]):
    def add(lang: str, code: str, title: str = "", en_code: str | None = None,
            author: str | None = None, year: int | None = None,
            corpus: str | None = None) -> None:
        if not code:
            return
        entry = out.setdefault(lang, {}).setdefault(code, {"titles": []})
        if title and title not in entry["titles"]:
            entry["titles"].append(title)
        if en_code and lang != "en" and en_code not in ("N/A", "n/a"):
            entry["en_code"] = en_code
        if author and not entry.get("author"):
            entry["author"] = author
        if year and not entry.get("year"):
            entry["year"] = year
        if corpus and not entry.get("corpus"):
            entry["corpus"] = corpus
    return add


# --------------------------------------------------------------------------- sources

def from_generator(sop: Path, add) -> None:
    """The EGW file mirror: book_map.json + en/<CODE>.json meta blocks."""
    book_map = json.loads((sop / "book_map.json").read_text(encoding="utf-8"))
    for path in sorted((sop / "en").glob("*.json")):
        meta = json.loads(path.read_text(encoding="utf-8")).get("meta", {})
        add("en", meta.get("en_code", path.stem), meta.get("en_title", ""))
    for lang, titles in book_map.items():
        for title, v in titles.items():
            code = v.get(f"{lang}_code")
            if code:
                add(lang, code, title, v.get("en_code"))
            if v.get("en_code") and v.get("en_title"):
                add("en", v["en_code"], v["en_title"])


def from_corpus_jsonl(path: Path, add) -> int:
    """A build artifact: one record per point, payload carries the metadata.

    Only the first record per (lang, book_code) is read; the rest repeat it.
    """
    seen: set[tuple[str, str]] = set()
    with path.open(encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            pl = json.loads(line).get("payload", {})
            key = (pl.get("lang", "en"), pl.get("book_code", ""))
            if not key[1] or key in seen:
                continue
            seen.add(key)
            add(key[0], key[1], pl.get("title", "") or "",
                author=pl.get("author"), year=pl.get("year"),
                corpus=pl.get("corpus"))
    return len(seen)


def _post(url: str, body: dict, timeout: float) -> dict:
    req = urllib.request.Request(url, data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode()).get("result") or {}


def from_qdrant(base: str, collection: str, add, timeout: float = 60.0) -> int:
    """Whatever is actually indexed: facet book_code per lang, read one point each."""
    root = f"{base.rstrip('/')}/collections/{collection}"
    langs = [h["value"] for h in
             _post(f"{root}/facet", {"key": "lang", "limit": 1000, "exact": True},
                   timeout)["hits"]]
    n = 0
    for lang in langs:
        flt = {"must": [{"key": "lang", "match": {"value": lang}}]}
        hits = _post(f"{root}/facet",
                     {"key": "book_code", "limit": 10_000, "exact": True, "filter": flt},
                     timeout)["hits"]
        for h in hits:
            code = h["value"]
            res = _post(f"{root}/points/scroll", {
                "limit": 1, "with_payload": True, "with_vector": False,
                "filter": {"must": [{"key": "lang", "match": {"value": lang}},
                                    {"key": "book_code", "match": {"value": code}}]},
            }, timeout)
            pts = res.get("points") or []
            if not pts:
                continue
            pl = pts[0].get("payload") or {}
            add(lang, code, pl.get("title", "") or "",
                author=pl.get("author"), year=pl.get("year"),
                corpus=pl.get("corpus"))
            n += 1
    return n


# --------------------------------------------------------------------------- main

def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("sop_dir", nargs="?",
                    help="generator/data/sop (the EGW file mirror)")
    ap.add_argument("--merge", action="append", default=[], metavar="JSONL",
                    help="corpus JSONL to merge (repeatable)")
    ap.add_argument("--qdrant-url", metavar="URL",
                    help="harvest titles from a live collection")
    ap.add_argument("--collection", default="sop")
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--dry-run", action="store_true",
                    help="report what would change, write nothing")
    args = ap.parse_args()

    if not (args.sop_dir or args.merge or args.qdrant_url):
        ap.error("give SOP_DIR, --merge or --qdrant-url (at least one source)")

    out: dict[str, dict[str, dict]] = {}
    add = make_adder(out)

    if args.sop_dir:
        from_generator(Path(args.sop_dir), add)
        print(f"generator mirror: {sum(len(v) for v in out.values())} codes")
    for m in args.merge:
        before = sum(len(v) for v in out.values())
        n = from_corpus_jsonl(Path(m), add)
        after = sum(len(v) for v in out.values())
        print(f"{m}: {n} codes read, {after - before} new")
    if args.qdrant_url:
        before = sum(len(v) for v in out.values())
        n = from_qdrant(args.qdrant_url, args.collection, add)
        after = sum(len(v) for v in out.values())
        print(f"qdrant {args.qdrant_url}: {n} codes read, {after - before} new")

    for books in out.values():
        for entry in books.values():
            entry["titles"].sort(key=len)

    untitled = sorted(f"{lg}:{c}" for lg, books in out.items()
                      for c, e in books.items() if not e["titles"])
    if untitled:
        print(f"WARNING: {len(untitled)} codes still have no title: "
              + ", ".join(untitled[:20]) + (" …" if len(untitled) > 20 else ""))

    dest = Path(args.out)
    text = json.dumps(out, ensure_ascii=False, indent=1, sort_keys=True) + "\n"
    if args.dry_run:
        old = json.loads(dest.read_text(encoding="utf-8")) if dest.is_file() else {}
        for lg in sorted(set(old) | set(out)):
            was, now = len(old.get(lg, {})), len(out.get(lg, {}))
            print(f"  {lg}: {was} → {now}")
        print("[dry-run] nothing written")
        return 0
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(text, encoding="utf-8")
    print(f"wrote {dest} ({', '.join(f'{k}: {len(v)}' for k, v in sorted(out.items()))})")
    return 0


if __name__ == "__main__":
    sys.exit(main())

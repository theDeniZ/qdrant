#!/usr/bin/env python3
"""Add book codes from a corpus JSONL into an EXISTING app/data/sop_books.json.

``export_book_titles.py`` rebuilds the whole table from its three sources and
writes it wholesale, so every source must be present or codes silently vanish.
In particular ``generator/data/sop`` is the only source of the de/ja/ko
``en_code`` mappings, which makes a full rebuild impossible from a checkout of
``qdrant/`` alone.

Adding newly indexed books needs none of that. This script loads the current
table as the base and merges a corpus JSONL on top:

  * codes already in the table keep their titles, author, year and en_code
  * new codes are added with title / author / year / corpus from the payload
  * nothing is ever removed

It reuses ``make_adder`` and ``from_corpus_jsonl`` from ``export_book_titles``
so the entry shape stays identical.

Usage:
    python3 scripts/merge_corpus_titles.py pd-books/qdrant/pioneers_corpus.jsonl
    python3 scripts/merge_corpus_titles.py CORPUS.jsonl --dry-run
    python3 scripts/merge_corpus_titles.py A.jsonl B.jsonl --out other.json
"""
from __future__ import annotations

import argparse
import copy
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from export_book_titles import OUT, make_adder, from_corpus_jsonl  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("corpus", nargs="+", help="corpus JSONL (repeatable)")
    ap.add_argument("--out", default=str(OUT), help=f"table to update (default {OUT})")
    ap.add_argument("--dry-run", action="store_true", help="report, write nothing")
    ap.add_argument("--enrich", action="store_true",
                    help="also let the corpus add title variants and fill missing "
                         "author/year on codes ALREADY in the table (default: new "
                         "codes only, existing entries are left untouched)")
    args = ap.parse_args()

    dest = Path(args.out)
    if not dest.is_file():
        raise SystemExit(f"{dest} not found - this script UPDATES an existing table. "
                         "To build one from nothing, use export_book_titles.py.")

    out = json.loads(dest.read_text(encoding="utf-8"))
    before = copy.deepcopy(out)
    raw_add = make_adder(out)

    # Without this, a corpus whose title string differs even slightly from the
    # one already recorded appends a second variant to an existing entry. The
    # job here is to add newly indexed books, so by default an existing code is
    # not touched at all.
    if args.enrich:
        add = raw_add
    else:
        def add(lang, code, title="", en_code=None, author=None, year=None, corpus=None):
            if code in before.get(lang, {}):
                return
            raw_add(lang, code, title, en_code, author=author, year=year, corpus=corpus)

    for c in args.corpus:
        p = Path(c)
        if not p.is_file():
            raise SystemExit(f"corpus not found: {p}")
        n = from_corpus_jsonl(p, add)
        print(f"{p}: {n} (lang, code) pairs read")

    added, changed = [], []
    for lang in sorted(out):
        for code in sorted(out[lang]):
            old = before.get(lang, {}).get(code)
            if old is None:
                added.append(f"{lang}:{code}")
            elif old != out[lang][code]:
                changed.append(f"{lang}:{code}")

    lost = [f"{lg}:{c}" for lg in before for c in before[lg] if c not in out.get(lg, {})]
    if lost:                                    # cannot happen; the merge is additive
        raise SystemExit(f"refusing to write, {len(lost)} codes would be lost: "
                         + ", ".join(lost[:20]))

    for lang in sorted(set(before) | set(out)):
        print(f"  {lang}: {len(before.get(lang, {}))} -> {len(out.get(lang, {}))}")
    print(f"added {len(added)}: " + (", ".join(added[:30]) or "none")
          + (" …" if len(added) > 30 else ""))
    if changed:
        print(f"enriched {len(changed)}: " + ", ".join(changed[:30])
              + (" …" if len(changed) > 30 else ""))

    untitled = sorted(f"{lg}:{c}" for lg, books in out.items()
                      for c, e in books.items() if not e.get("titles"))
    if untitled:
        print(f"WARNING: {len(untitled)} codes still have no title: "
              + ", ".join(untitled[:20]) + (" …" if len(untitled) > 20 else ""))

    for books in out.values():
        for entry in books.values():
            entry.get("titles", []).sort(key=len)

    if args.dry_run:
        print("[dry-run] nothing written")
        return 0
    dest.write_text(json.dumps(out, ensure_ascii=False, indent=1, sort_keys=True) + "\n",
                    encoding="utf-8")
    print(f"wrote {dest} ({', '.join(f'{k}: {len(v)}' for k, v in sorted(out.items()))})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

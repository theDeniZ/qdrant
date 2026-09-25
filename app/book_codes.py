"""Export the offline book-code registry `sopack` uses for collision checks.

sopack 1.0 (`docs/SOPACK-1.0-PLAN.md` §3.2 "book-code collisions") never talks
to Qdrant — `propose`/`extract` check a candidate `book_code` against a
**committed** registry file instead:
`sopack-rs/contracts/<contract>/book_codes.json`. This module is the
importer-side command that (re)generates that file from the same authority
`sop_list_books` already reads, `app/data/sop_books.json` (see
`scripts/export_book_titles.py` for how *that* file itself is built/merged).

Stdlib only — no Qdrant client, no network, safe to run from a bare
`qdrant/` checkout.

    python -m app.book_codes export
    python -m app.book_codes export --sop-books app/data/sop_books.json \\
        --out sopack-rs/contracts/e5-large-v1/book_codes.json

Output shape::

    {"schema": "sopack.book_codes/1",
     "generated_from": "app/data/sop_books.json",
     "generated_at": "2026-09-24T00:00:00Z",
     "codes": {"SC": {"title": "Steps to Christ", "lang": ["en"],
                       "corpus": null, "slug": null},
               "BW": {"title": "Der bessere Weg zu einem neuen Leben",
                       "lang": ["de"], "corpus": null, "slug": null}}}

`sop_books.json` has no `slug` field today (nothing writes one), so every
`codes[...].slug` comes out `null` until a source that carries slugs (a
corpus JSONL, a future importer stage) is merged into `sop_books.json`
upstream of this export — this module only reshapes what is already there,
it does not invent one. `title` prefers the English entry when a code is
recorded in more than one language table (rare — most languages use their
own distinct code strings, e.g. German `BW` for English `SC`); `lang` lists
every language table the literal code string was found under; `corpus` and
`year` are passed through additively, same semantics as
`export_book_titles.py`'s `add()`.

**Operational note:** this command is not wired into any scheduled job. Like
`export_book_titles.py`, it is meant to be re-run by hand as part of the
admin refresh that follows every corpus import, so the registry a build of
`sopack` embeds never drifts far from what `sop_list_books` actually serves.
Re-running it is always safe — the output is fully recomputed from
`sop_books.json`, never hand-edited in place.
"""
from __future__ import annotations

import argparse
import datetime
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent          # qdrant/app
ROOT = HERE.parent                               # qdrant/
DEFAULT_SOP_BOOKS = HERE / "data" / "sop_books.json"
DEFAULT_OUT = ROOT / "sopack-rs" / "contracts" / "e5-large-v1" / "book_codes.json"

SCHEMA = "sopack.book_codes/1"

# English first: when a literal code string is recorded under more than one
# language table, the English title is preferred as the registry's display
# title. Any language not listed here is still included, just after these.
_LANG_PRIORITY = ["en", "de", "ja", "ko"]


def _ordered_langs(sop_books: dict) -> list[str]:
    present = sorted(sop_books.keys())
    ordered = [lg for lg in _LANG_PRIORITY if lg in sop_books]
    ordered += [lg for lg in present if lg not in _LANG_PRIORITY]
    return ordered


def build_registry(sop_books: dict) -> dict:
    """`sop_books.json`'s `{lang: {code: entry}}` shape -> `codes` mapping.

    One registry row per **literal** `book_code` string. Two different
    languages that happen to write the same literal code collapse into one
    row (`lang` lists both); the common case — a translation using its own
    distinct code, linked back only via `en_code` — produces two separate
    rows, which is correct: they are two different `book_code` values a
    `propose` candidate could collide with.
    """
    codes: dict[str, dict] = {}
    for lang in _ordered_langs(sop_books):
        books = sop_books.get(lang) or {}
        for code, entry in books.items():
            if not code:
                continue
            rec = codes.setdefault(
                code, {"title": None, "lang": [], "corpus": None, "slug": None,
                       "author": None, "year": None}
            )
            if lang not in rec["lang"]:
                rec["lang"].append(lang)
            titles = entry.get("titles") or []
            if titles and rec["title"] is None:
                rec["title"] = titles[0]
            if entry.get("corpus") and rec["corpus"] is None:
                rec["corpus"] = entry["corpus"]
            if entry.get("slug") and rec["slug"] is None:
                rec["slug"] = entry["slug"]
            # author/year let an agent confirm a registry title match
            # (re-import detection) against the source's own byline/date.
            if entry.get("author") and rec["author"] is None:
                rec["author"] = entry["author"]
            if entry.get("year") and rec["year"] is None:
                rec["year"] = entry["year"]
    for rec in codes.values():
        rec["lang"].sort()
    return codes


def export(sop_books_path: Path | str, out_path: Path | str,
           generated_at: str | None = None) -> dict:
    """Read *sop_books_path*, write the registry to *out_path*, return it.

    *generated_at* is exposed for tests (deterministic timestamp); real
    callers leave it as the current UTC time.
    """
    sop_books_path = Path(sop_books_path)
    out_path = Path(out_path)
    sop_books = json.loads(sop_books_path.read_text(encoding="utf-8"))
    registry = {
        "schema": SCHEMA,
        "generated_from": _relative_label(sop_books_path),
        "generated_at": generated_at or _now_iso(),
        "codes": build_registry(sop_books),
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(registry, ensure_ascii=False, indent=1, sort_keys=True) + "\n"
    out_path.write_text(text, encoding="utf-8")
    return registry


def _relative_label(path: Path) -> str:
    """*path* relative to `qdrant/` when it lives under there, else as given.

    The registry is a committed artifact (`sopack-rs/contracts/.../book_codes.json`)
    — recording the absolute path of whichever machine last ran `export` would
    make every re-run a no-op diff noise generator. A checkout-relative label
    (`app/data/sop_books.json`) is what's actually reproducible.
    """
    resolved = path.resolve()
    try:
        return str(resolved.relative_to(ROOT))
    except ValueError:
        return str(path)


def _now_iso() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    exp = sub.add_parser("export", help="(re)generate book_codes.json from sop_books.json")
    exp.add_argument("--sop-books", default=str(DEFAULT_SOP_BOOKS),
                      help=f"default: {DEFAULT_SOP_BOOKS}")
    exp.add_argument("--out", default=str(DEFAULT_OUT),
                      help=f"default: {DEFAULT_OUT}")

    args = ap.parse_args(argv)
    if args.cmd == "export":
        registry = export(args.sop_books, args.out)
        print(f"wrote {args.out} ({len(registry['codes'])} codes)")
        return 0
    ap.error(f"unknown command {args.cmd!r}")
    return 2


if __name__ == "__main__":
    sys.exit(main())

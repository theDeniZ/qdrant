"""``sopack`` — the Mac-side CLI: ``extract``, ``inspect``, ``canaries``,
``pack``, ``verify``, ``doctor``.

Every subcommand's real implementation is imported **inside** its handler,
never at module level, so that (a) running ``sopack doctor`` never needs
``fastembed`` to be installed at all, and (b) running ``sopack pack`` fails on
a missing/mismatched ``fastembed`` the instant that one subcommand is
invoked (``sopack.pack`` does the import-time assertion — see its docstring)
rather than delaying the failure into whatever handler runs later.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

__all__ = ["main"]


# Which metadata options each extractor actually accepts. These are NOT
# uniform: `sop_json` derives book_code, title and year from the file's own
# `meta` block, so offering them there would silently drop them. A blind
# argv -> kwargs passthrough is therefore wrong, and this table is the adapter.
_EXTRACT_OPTS = {
    "epub": ("book_code", "lang", "title", "author", "year", "corpus", "slug",
             "book_pair", "acquired_from", "rights"),
    "markdown": ("book_code", "lang", "title", "author", "year", "corpus", "slug",
                 "book_pair", "acquired_from", "rights"),
    "text": ("book_code", "lang", "title", "author", "year", "corpus", "slug",
             "book_pair", "acquired_from", "rights"),
    "sop_json": ("lang", "author", "corpus", "slug", "book_pair",
                 "acquired_from", "rights"),
}


def _cmd_extract(args: argparse.Namespace) -> int:
    try:
        from . import extract as extract_mod
        from . import book as book_mod
    except ImportError as exc:
        print(f"sopack.extract is not available: {exc}", file=sys.stderr)
        return 2

    allowed = _EXTRACT_OPTS[args.kind]
    supplied = {k: v for k, v in vars(args).items()
                if k in {o for opts in _EXTRACT_OPTS.values() for o in opts}
                and v is not None}

    # Refuse rather than silently drop: a caller who passes --year to a
    # sop_json extract must learn that the year comes from the file, not be
    # left believing they set it.
    rejected = sorted(set(supplied) - set(allowed))
    if rejected:
        print(f"--kind {args.kind} does not accept: "
              + ", ".join("--" + r.replace("_", "-") for r in rejected),
              file=sys.stderr)
        print(f"  ({args.kind} takes these from the source file itself)",
              file=sys.stderr)
        return 2

    try:
        book = extract_mod.extract(args.source, args.kind,
                                   **{k: v for k, v in supplied.items() if k in allowed})
    except Exception as exc:  # noqa: BLE001 — surface any extractor failure plainly
        print(f"extract failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1

    # page_kind and id_rule have no extract() kwarg at all — page_kind is
    # always computed internally per kind (epub: print/chapter from the
    # detected citation scheme; markdown/text: always "chapter"; sop_json:
    # always None), and id_rule is fixed by profile/kind (sop/seq for
    # chunkable sources, sop/plain for sop_json, which never splits). Apply
    # them as a direct overlay on the returned Book instead.
    if args.page_kind is not None:
        book.book["page_kind"] = args.page_kind
    if args.id_rule is not None:
        book.id_rule = args.id_rule

    # A book.json that does not validate must never reach `pack` — that is
    # where a missing book_code or a bad id_rule would turn into points written
    # under the wrong identity.
    problems = book_mod.validate(book)
    if problems:
        print(f"{args.source}: book.json is not valid, refusing to write:",
              file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1

    book_mod.dump(book, args.out)

    meta = book.book if isinstance(getattr(book, "book", None), dict) else {}
    stats = getattr(book, "stats", None) or {}
    print(f"wrote {args.out}")
    print(f"  book_code={meta.get('book_code')} lang={meta.get('lang')} "
          f"id_rule={getattr(book, 'id_rule', '?')}")
    print("  " + "  ".join(f"{k}={v}" for k, v in stats.items()
                           if k != "dropped_detail"))
    # The OPF <dc:date> of a modern digital edition is its release date, not the
    # work's publication year (the Gutenberg Andrews EPUB says 2022 for an 1873
    # book). A wrong year propagates into the title table and every citation.
    if "year" not in supplied and meta.get("year"):
        print(f"  NOTE: year={meta['year']} was taken from the source file, not "
              f"given by you — check it is the publication year, not a digital "
              f"edition date.")
    return 0


def _cmd_inspect(args: argparse.Namespace) -> int:
    try:
        from .book import load
    except ImportError as exc:
        print(f"sopack.book is not available yet: {exc}", file=sys.stderr)
        return 2
    book = load(args.book_json)
    blocks = getattr(book, "blocks", None) or []
    stats = getattr(book, "stats", None) or {}
    print(f"file: {args.book_json}")
    print(f"profile: {getattr(book, 'profile', '?')}")
    print(f"id_rule: {getattr(book, 'id_rule', '?')}")
    print(f"blocks: {len(blocks)}")
    if stats:
        for k, v in stats.items():
            print(f"  {k}: {v}")
    if hasattr(book, "book") and isinstance(book.book, dict):
        b = book.book
        print(f"book_code: {b.get('book_code')}  lang: {b.get('lang')}  "
              f"title: {b.get('title')!r}")
    return 0


def _cmd_canaries(args: argparse.Namespace) -> int:
    from . import canaries as canaries_mod
    result = canaries_mod.fetch(args.qdrant, args.collection, n=args.n)
    canaries_mod.save(result, args.out)
    print(f"wrote {len(result)} canaries to {args.out}")
    return 0


def _cmd_pack(args: argparse.Namespace) -> int:
    from . import pack as pack_mod
    try:
        manifest = pack_mod.pack(
            args.books, args.out, args.canaries,
            profile=args.profile, pack_id=args.pack_id,
            id_rule=args.id_rule, batch_size=args.batch_size,
            workers=args.workers)
    except pack_mod.PackBuildError as exc:
        print(f"pack failed: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(manifest["counts"], indent=2))
    return 0


def _cmd_verify(args: argparse.Namespace) -> int:
    from . import verify as verify_mod
    errors = verify_mod.verify(args.pack)
    if errors:
        print(f"{len(errors)} problem(s) found:")
        for e in errors:
            print(f"  - {e}")
        return 1
    print(f"{args.pack}: clean")
    return 0


def _cmd_doctor(args: argparse.Namespace) -> int:
    from . import doctor as doctor_mod
    return doctor_mod.main(["--quick"] if args.quick else [])


def build_parser() -> argparse.ArgumentParser:
    from . import __version__
    ap = argparse.ArgumentParser(prog="sopack", description=__doc__)
    ap.add_argument("--version", action="version", version=f"sopack {__version__}")
    sub = ap.add_subparsers(dest="command", required=True)

    p_extract = sub.add_parser("extract", help="source -> reviewable book.json")
    p_extract.add_argument("source", type=Path)
    p_extract.add_argument("--kind", required=True, choices=sorted(_EXTRACT_OPTS),
                           help="source format; sop_json takes its metadata from "
                                "the file's own meta block")
    p_extract.add_argument("-o", "--out", required=True, type=Path)
    p_extract.add_argument("--book-code")
    p_extract.add_argument("--lang")
    p_extract.add_argument("--title")
    p_extract.add_argument("--author")
    p_extract.add_argument("--year", type=int)
    p_extract.add_argument("--corpus")
    p_extract.add_argument("--slug")
    p_extract.add_argument("--book-pair")
    p_extract.add_argument("--acquired-from")
    p_extract.add_argument("--rights")
    p_extract.add_argument("--page-kind",
                           help="overlay on Book.book['page_kind'] after extraction "
                                "(no extract() kwarg exists for this — every kind "
                                "computes/fixes it internally)")
    p_extract.add_argument("--id-rule",
                           help="overlay on Book.id_rule after extraction (no "
                                "extract() kwarg exists — fixed by kind/profile); "
                                "validate() still checks it against the profile")
    p_extract.set_defaults(func=_cmd_extract)

    p_inspect = sub.add_parser("inspect", help="counts, damage, codes for a book.json")
    p_inspect.add_argument("book_json", type=Path)
    p_inspect.set_defaults(func=_cmd_inspect)

    p_canaries = sub.add_parser("canaries", help="refresh canaries.json from live Qdrant")
    p_canaries.add_argument("--qdrant", required=True, help="Qdrant base URL")
    p_canaries.add_argument("--collection", default="sop")
    p_canaries.add_argument("-n", type=int, default=8)
    p_canaries.add_argument("-o", "--out", default="canaries.json", type=Path)
    p_canaries.set_defaults(func=_cmd_canaries)

    p_pack = sub.add_parser("pack", help="book.json(s) -> .sopack (embeds, slow)")
    p_pack.add_argument("books", nargs="+", type=Path)
    p_pack.add_argument("--canaries", required=True, type=Path)
    p_pack.add_argument("-o", "--out", required=True, type=Path)
    p_pack.add_argument("--profile", default=None)
    p_pack.add_argument("--pack-id", default=None)
    p_pack.add_argument("--id-rule", default=None)
    p_pack.add_argument("--batch-size", type=int, default=128)
    p_pack.add_argument("--workers", type=int, default=None,
                        help="fastembed parallel worker PROCESSES (default: 0, "
                             "single-process). Each worker loads its own copy "
                             "of the ~2.2 GB model — only raise this on a "
                             "machine with the RAM for N x 2.2 GB free; a value "
                             "too high for available memory hangs, not fails.")
    p_pack.set_defaults(func=_cmd_pack)

    p_verify = sub.add_parser("verify", help="offline .sopack integrity check")
    p_verify.add_argument("pack", type=Path)
    p_verify.set_defaults(func=_cmd_verify)

    p_doctor = sub.add_parser("doctor", help="environment check")
    p_doctor.add_argument("--quick", action="store_true",
                          help="skip the ~30s real model load/embed check")
    p_doctor.set_defaults(func=_cmd_doctor)

    return ap


def main(argv: list[str] | None = None) -> int:
    ap = build_parser()
    args = ap.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())

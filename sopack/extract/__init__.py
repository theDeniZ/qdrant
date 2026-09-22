"""Deterministic, NO-LLM source → :class:`~sopack.book.Book` extraction.

``extract(source_path, kind, **opts) -> Book`` dispatches on *kind*:
``epub``, ``markdown``, ``text``, ``sop_json``. Every extractor is stdlib
only and never invents metadata (rule #9) — what it cannot read from the
source is left ``None`` in ``Book.book`` and reported missing by
``sopack.book.validate``.

Every extractor produces ``Book.stats`` with ``blocks_in``, ``blocks_out``,
``dropped``, ``damage``, ``words`` and ``dropped_detail`` (a list of what was
dropped and why — R8, nothing silently discarded).
"""

from __future__ import annotations

from pathlib import Path

from ..book import Book, BookError
from . import epub as _epub
from . import markdown as _markdown
from . import sop_json as _sop_json
from . import text as _text

__all__ = ["extract"]

_DISPATCH = {
    "epub": _epub.extract,
    "markdown": _markdown.extract,
    "text": _text.extract,
    "sop_json": _sop_json.extract,
}


def extract(source_path, kind: str, **opts) -> Book:
    """Extract *source_path* (any ``kind`` in ``epub``/``markdown``/``text``/
    ``sop_json``) into a reviewable :class:`~sopack.book.Book`. Unknown
    ``kind`` raises :class:`~sopack.book.BookError`; per-kind keyword options
    are documented on each extractor's ``extract`` function."""
    try:
        fn = _DISPATCH[kind]
    except KeyError:
        raise BookError(
            f"unknown extract kind {kind!r} (have: {', '.join(sorted(_DISPATCH))})") from None
    return fn(Path(source_path), **opts)

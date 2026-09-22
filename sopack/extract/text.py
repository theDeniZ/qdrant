"""Plain text → :class:`~sopack.book.Book`.

No page/paragraph structure to recover, so the whole file is split on blank
lines into paragraphs and numbered sequentially: ``page`` is always ``1``,
``para`` counts up. Long paragraphs are chunked the same way as the EPUB
extractor (``extract.chunk``); short/damaged/junk paragraphs are dropped and
recorded in ``stats["dropped_detail"]`` (R8).

Metadata (``book_code``, ``lang``, ``title``, ``author``, ``year``, …) is
never guessed from the filename (rule #9) — pass it as keyword options; what
is not given is left ``None`` for hand entry in the reviewed book.json.

**Stdlib only.**
"""

from __future__ import annotations

import hashlib
import re
from pathlib import Path

from ..book import Block, Book, BookError
from . import chunk

_BLANK_RE = re.compile(r'\n\s*\n+')


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def extract(
    path: Path,
    *,
    book_code: str | None = None,
    lang: str | None = None,
    title: str | None = None,
    author: str | None = None,
    year: int | None = None,
    corpus: str | None = None,
    slug: str | None = None,
    book_pair: str | None = None,
    acquired_from: str | None = None,
    rights: str | None = None,
) -> Book:
    path = Path(path)
    if not path.exists():
        raise BookError(f"no such file: {path}")
    raw = path.read_text(encoding="utf-8", errors="replace")

    out_blocks: list[Block] = []
    dropped_detail: list[dict] = []
    blocks_in = 0
    para_no = 0

    for raw_para in _BLANK_RE.split(raw):
        text = re.sub(r'[ \t]+', ' ', raw_para.strip())
        if not text:
            continue
        blocks_in += 1
        para_no += 1
        para_key = f"1.{para_no}"

        reason = chunk.quality_gate(text)
        if reason:
            dropped_detail.append({"para_key": para_key, "reason": reason, "text": text[:80]})
            continue

        pieces = chunk.split_long(text)
        for j, piece in enumerate(pieces):
            out_blocks.append(Block(
                para_key=para_key, page=1, para=para_no,
                seq=j, chunks=len(pieces), text=piece,
                words=len(piece.split()),
            ))

    joined = " ".join(b.text for b in out_blocks)
    stats = {
        "blocks_in": blocks_in,
        "blocks_out": len(out_blocks),
        "dropped": len(dropped_detail),
        "damage": chunk.damage_score(joined) if joined else 0.0,
        "words": len(joined.split()),
        "dropped_detail": dropped_detail,
    }

    book_meta = {
        "book_code": book_code,
        "lang": lang,
        "book_pair": book_pair if book_pair is not None else book_code,
        "title": title,
        "author": author,
        "year": year,
        "slug": slug,
        "corpus": corpus,
        "page_kind": "chapter",
    }

    from .. import contract

    return Book(
        schema=contract.SCHEMA_BOOK,
        profile="sop",
        source={
            "file": str(path),
            "sha256": _sha256_file(path),
            "kind": "text",
            "acquired_from": acquired_from,
            "rights": rights,
        },
        book=book_meta,
        id_rule="sop/seq",
        alignment=None,
        stats=stats,
        blocks=out_blocks,
    )

"""SDARM SoP JSON → :class:`~sopack.book.Book`.

Reads the ``generator/data/sop/<lang>/<CODE>.json`` shape already used by
``sdarm.tools.build_sop_vector_index`` — the live ``sop`` collection was
built from exactly these files::

    {
      "meta": {"de_code": "BH", "en_code": "SL", "de_title": "...",
                "en_title": "...", "publisher": "...", "year": "1973", ...},
      "de": {"5.1": {"text": "...", "en_ref": "7.1", "chapter": "..."}, ...},
      "en_reverse": {"7.1": ["5.1"], ...}
    }

German files are keyed by DE code and carry ``en_reverse`` (de_para_key is
the file's own key; en_reverse maps the *English* para_key it aligns to,
back to a list of this file's para_keys — the same shape the live indexer
reads). English and every other language (``ja``, ``ko``, …) files have no
``en_reverse`` — they carry only ``meta`` and their own language key.

No chunking happens here: the live collection has always indexed one point
per JSON paragraph key with no splitting (id_rule ``sop/plain``, which has no
``#<seq>`` suffix to disambiguate a split block), so re-extracting through
this module must reproduce the same one-block-per-paragraph shape or points
would silently collide.

``lang`` is inferred from the parent directory name when not given — that is
the documented shape of this source (the whole corpus is organised this way,
and the live indexer itself falls back to the file stem for the book code),
not a guess at unrelated metadata from an arbitrary filename (rule #9).
Author is intentionally left ``None`` unless given explicitly: every file
under ``data/sop/`` is an EGW work and the corpus convention is that EGW
points carry no ``corpus`` key, which is exactly what makes ``validate()``
exempt them from requiring ``author``/``year``.

**Stdlib only.**
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from ..book import Block, Book, BookError
from . import chunk


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def _parse_page_para(para_key: str) -> tuple[int, int]:
    try:
        page_str, para_str = para_key.split(".", 1)
        return int(page_str), int(para_str)
    except (ValueError, AttributeError):
        return 0, 0


def _year(raw) -> int | None:
    if isinstance(raw, int):
        return raw
    if isinstance(raw, str) and raw.strip().isdigit():
        return int(raw.strip())
    return None


def extract(
    path: Path,
    *,
    lang: str | None = None,
    corpus: str | None = None,
    author: str | None = None,
    slug: str | None = None,
    book_pair: str | None = None,
    acquired_from: str | None = None,
    rights: str | None = None,
) -> Book:
    path = Path(path)
    if not path.exists():
        raise BookError(f"no such file: {path}")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise BookError(f"{path}: not valid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise BookError(f"{path}: top level must be a JSON object")

    resolved_lang = lang or path.parent.name
    if resolved_lang not in data:
        available = [k for k in data if k not in ("meta", "en_reverse")]
        raise BookError(
            f"{path}: no {resolved_lang!r} key in this file (has: {', '.join(available) or 'none'})")

    meta = data.get("meta") or {}
    code = meta.get(f"{resolved_lang}_code") or meta.get("en_code") or path.stem
    resolved_title = meta.get(f"{resolved_lang}_title") or meta.get("en_title")
    resolved_year = _year(meta.get("year"))

    paras = data[resolved_lang]
    if not isinstance(paras, dict):
        raise BookError(f"{path}: {resolved_lang!r} must be an object of para_key -> entry")

    out_blocks: list[Block] = []
    dropped_detail: list[dict] = []
    blocks_in = 0

    for para_key, entry in paras.items():
        blocks_in += 1
        if not isinstance(entry, dict):
            dropped_detail.append({"para_key": para_key, "reason": "entry is not an object"})
            continue
        text = (entry.get("text") or "").strip()
        if not text:
            dropped_detail.append({"para_key": para_key, "reason": "empty text"})
            continue
        page, para = _parse_page_para(para_key)
        out_blocks.append(Block(
            para_key=para_key, page=page, para=para, seq=0, chunks=1,
            text=text, words=len(text.split()),
        ))

    alignment = None
    en_reverse = data.get("en_reverse")
    if en_reverse is not None:
        alignment = {"en_code": meta.get("en_code"), "en_reverse": en_reverse}

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
        "book_code": code,
        "lang": resolved_lang,
        "book_pair": book_pair,
        "title": resolved_title,
        "author": author,
        "year": resolved_year,
        "slug": slug,
        "corpus": corpus,
        "page_kind": None,
    }

    from .. import contract

    return Book(
        schema=contract.SCHEMA_BOOK,
        profile="sop",
        source={
            "file": str(path),
            "sha256": _sha256_file(path),
            "kind": "sop_json",
            "acquired_from": acquired_from,
            "rights": rights,
        },
        book=book_meta,
        id_rule="sop/plain",
        alignment=alignment,
        stats=stats,
        blocks=out_blocks,
    )

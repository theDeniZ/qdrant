"""``book.json`` — the reviewable intermediate between a source and a `.sopack`.

This module is a **fixed seam**: ``Block``, ``Book``, ``load``, ``dump``,
``validate``, ``to_payload``, ``uid`` and ``BookError`` are consumed by other
code (the ``pack`` step and, eventually, the extract CLI) and their names and
signatures must not change.

Deterministic output of ``sopack.extract``, and the thing a human reads, diffs
and keeps. Plain JSON, no vectors, safe to commit. See
``docs/IMPORT-PIPELINE-PLAN.md`` §3 for the exact shape.

**Stdlib only.**
"""

from __future__ import annotations

import json
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path

from . import contract

__all__ = ["Block", "Book", "BookError", "load", "dump", "validate", "to_payload", "uid"]


class BookError(Exception):
    """A book.json is malformed, structurally inconsistent, or refers to an
    unknown profile / id_rule. Raised by :func:`load`."""


@dataclass
class Block:
    para_key: str
    page: int
    para: int
    seq: int
    chunks: int
    text: str
    words: int


@dataclass
class Book:
    schema: str
    profile: str
    source: dict
    book: dict
    id_rule: str
    alignment: dict | None
    stats: dict
    blocks: list[Block] = field(default_factory=list)


# ── load / dump ──────────────────────────────────────────────────────────────

_REQUIRED_TOP = ("schema", "profile", "source", "book", "id_rule", "stats", "blocks")
_REQUIRED_BLOCK = ("para_key", "page", "para", "seq", "chunks", "text", "words")


def load(path) -> Book:
    """Parse *path* into a :class:`Book`, raising :class:`BookError` on any
    structurally bad input (not valid JSON, missing top-level keys, an
    unknown profile, a malformed block). Semantic completeness — missing
    metadata, duplicate blocks, etc. — is reported by :func:`validate`, not
    raised here, so a book.json with ``null`` metadata still loads."""
    path = Path(path)
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError as exc:
        raise BookError(f"cannot read {path}: {exc}") from exc
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise BookError(f"{path}: not valid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise BookError(f"{path}: top level must be a JSON object")

    missing = [k for k in _REQUIRED_TOP if k not in data]
    if missing:
        raise BookError(f"{path}: missing required key(s): {', '.join(missing)}")

    if data["schema"] != contract.SCHEMA_BOOK:
        raise BookError(
            f"{path}: unsupported schema {data['schema']!r} "
            f"(this module reads {contract.SCHEMA_BOOK!r})")

    try:
        contract.get_profile(data["profile"])
    except ValueError as exc:
        raise BookError(f"{path}: {exc}") from exc

    if not isinstance(data["source"], dict):
        raise BookError(f"{path}: 'source' must be an object")
    if not isinstance(data["book"], dict):
        raise BookError(f"{path}: 'book' must be an object")
    if not isinstance(data["stats"], dict):
        raise BookError(f"{path}: 'stats' must be an object")
    if not isinstance(data["id_rule"], str) or not data["id_rule"]:
        raise BookError(f"{path}: 'id_rule' must be a non-empty string")
    alignment = data.get("alignment")
    if alignment is not None and not isinstance(alignment, dict):
        raise BookError(f"{path}: 'alignment' must be an object or null")

    raw_blocks = data["blocks"]
    if not isinstance(raw_blocks, list):
        raise BookError(f"{path}: 'blocks' must be a list")

    blocks: list[Block] = []
    for i, b in enumerate(raw_blocks):
        if not isinstance(b, dict):
            raise BookError(f"{path}: blocks[{i}] is not an object")
        missing_b = [k for k in _REQUIRED_BLOCK if k not in b]
        if missing_b:
            raise BookError(f"{path}: blocks[{i}] missing key(s): {', '.join(missing_b)}")
        try:
            blocks.append(Block(
                para_key=str(b["para_key"]),
                page=int(b["page"]),
                para=int(b["para"]),
                seq=int(b["seq"]),
                chunks=int(b["chunks"]),
                text=str(b["text"]),
                words=int(b["words"]),
            ))
        except (TypeError, ValueError) as exc:
            raise BookError(f"{path}: blocks[{i}] has a badly typed field: {exc}") from exc

    return Book(
        schema=data["schema"],
        profile=data["profile"],
        source=data["source"],
        book=data["book"],
        id_rule=data["id_rule"],
        alignment=alignment,
        stats=data["stats"],
        blocks=blocks,
    )


def dump(book: Book, path) -> None:
    """Write *book* to *path* as book.json with a stable key order and
    ``indent=2``, so it diffs cleanly in review."""
    path = Path(path)
    doc = {
        "schema": book.schema,
        "profile": book.profile,
        "source": book.source,
        "book": book.book,
        "id_rule": book.id_rule,
        "alignment": book.alignment,
        "stats": book.stats,
        "blocks": [
            {
                "para_key": b.para_key,
                "page": b.page,
                "para": b.para,
                "seq": b.seq,
                "chunks": b.chunks,
                "text": b.text,
                "words": b.words,
            }
            for b in book.blocks
        ],
    }
    path.write_text(json.dumps(doc, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


# ── validate ─────────────────────────────────────────────────────────────────

def validate(book: Book) -> list[str]:
    """Semantic problems with *book*. Empty list means it is valid.

    Rejects: missing required metadata (``book_code``, ``lang``, ``title`` —
    plus ``author``/``year`` for non-EGW works, i.e. ``book.corpus`` is set;
    EGW works, which never carry a ``corpus`` key, are exempt, matching the
    live collection's convention), an ``id_rule`` not allowed by the book's
    profile, duplicate ``(para_key, seq)`` pairs, empty block text, and
    ``chunks``/``seq`` inconsistency within a ``para_key``.
    """
    errors: list[str] = []

    if book.schema != contract.SCHEMA_BOOK:
        errors.append(f"unsupported schema {book.schema!r}")
        return errors

    try:
        profile = contract.get_profile(book.profile)
    except ValueError as exc:
        errors.append(str(exc))
        return errors

    meta = book.book or {}
    for key in ("book_code", "lang", "title"):
        if not meta.get(key):
            errors.append(f"book.{key} is required")

    is_egw = meta.get("corpus") is None
    if not is_egw:
        for key in ("author", "year"):
            if not meta.get(key):
                errors.append(
                    f"book.{key} is required for non-EGW works (corpus={meta.get('corpus')!r})")

    if book.id_rule not in profile.id_rules:
        errors.append(
            f"id_rule {book.id_rule!r} is not valid for profile {profile.name!r} "
            f"(allowed: {', '.join(profile.id_rules)})")

    seen_pairs: set[tuple[str, int]] = set()
    by_para_key: dict[str, list[Block]] = defaultdict(list)
    for b in book.blocks:
        pair = (b.para_key, b.seq)
        if pair in seen_pairs:
            errors.append(f"duplicate block (para_key={b.para_key!r}, seq={b.seq})")
        seen_pairs.add(pair)
        if not b.text or not b.text.strip():
            errors.append(f"block (para_key={b.para_key!r}, seq={b.seq}) has empty text")
        by_para_key[b.para_key].append(b)

    for para_key, group in by_para_key.items():
        chunk_vals = {b.chunks for b in group}
        if len(chunk_vals) > 1:
            errors.append(
                f"para_key {para_key!r}: inconsistent 'chunks' values {sorted(chunk_vals)}")
            continue
        chunks = chunk_vals.pop()
        if chunks < 1:
            errors.append(f"para_key {para_key!r}: chunks must be >= 1, got {chunks}")
            continue
        seqs = sorted(b.seq for b in group)
        if seqs != list(range(chunks)):
            errors.append(
                f"para_key {para_key!r}: seq values {seqs} do not match chunks={chunks} "
                f"(expected {list(range(chunks))})")

    return errors


# ── payload / id ─────────────────────────────────────────────────────────────

def _aligned_for(book: Book, para_key: str):
    """The paragraph key(s) *para_key* aligns to in the other language.

    ``alignment["en_reverse"]`` is always keyed by **English** para_key,
    mapping to a list of the other language's para_keys (the shape
    ``sop_json`` files carry: ``en_reverse["7.1"] == ["5.1"]`` means EN 7.1
    aligns to DE 5.1). So a book whose own language *is* English looks it up
    directly; any other language inverts it once to go from its own
    para_key back to the aligned EN para_key(s)."""
    if not book.alignment:
        return None
    rev = book.alignment.get("en_reverse")
    if not rev:
        return None
    lang = (book.book or {}).get("lang")
    if lang == "en":
        return rev.get(para_key) or None
    hits = [en_pk for en_pk, own_pks in rev.items() if para_key in (own_pks or [])]
    return hits or None


def to_payload(book: Book, block: Block) -> dict:
    """The profile-aware Qdrant payload for one block of *book*.

    Produces exactly the payload keys ``contract.validate_payload`` accepts
    for the book's profile — required keys are always present (even as
    ``None``, which is legitimate for ``aligned``/``book_pair``), optional
    keys are included only when known.
    """
    profile = contract.get_profile(book.profile)
    meta = book.book or {}

    if profile.name == "sop":
        payload = {
            "lang": meta.get("lang"),
            "book_code": meta.get("book_code"),
            "book_pair": meta.get("book_pair"),
            "page": block.page,
            "para": block.para,
            "para_key": block.para_key,
            "raw_text": block.text,
            "aligned": _aligned_for(book, block.para_key),
        }
        for key in ("corpus", "author", "title", "year", "slug", "page_kind"):
            val = meta.get(key)
            if val is not None:
                payload[key] = val
        if block.chunks > 1:
            payload["chunk"] = block.seq
            payload["chunks"] = block.chunks
        return payload

    if profile.name == "bible":
        payload = {
            "bible": meta.get("book_code") or meta.get("bible"),
            "osis": block.para_key,
            "text": block.text,
        }
        for key in ("canonical_osis", "versification_offset"):
            val = meta.get(key)
            if val is not None:
                payload[key] = val
        return payload

    raise BookError(f"to_payload: no payload builder for profile {profile.name!r}")


def uid(book: Book, block: Block) -> str:
    """The pre-hash uid string for *block*, agreeing with
    ``contract.uid_for(book.id_rule, fields)``."""
    profile = contract.get_profile(book.profile)
    meta = book.book or {}
    if profile.name == "bible":
        fields = {
            "bible": meta.get("book_code") or meta.get("bible"),
            "osis": block.para_key,
        }
    else:
        fields = {
            "lang": meta.get("lang"),
            "book_code": meta.get("book_code"),
            "para_key": block.para_key,
            "seq": block.seq,
        }
    return contract.uid_for(book.id_rule, fields)

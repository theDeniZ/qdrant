"""EPUB → :class:`~sopack.book.Book`.

Ported from ``pd-books/qdrant/build_pioneers_corpus.py`` (zipfile + OPF
metadata, HTML→text, boilerplate/TOC stripping, the printed page.paragraph
citation lift). That script is the reference for what correct output looks
like — this module reproduces its logic, adapted to emit a :class:`Book`
instead of a flat JSONL.

**Stdlib only.**
"""

from __future__ import annotations

import collections
import hashlib
import html
import os
import re
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

from ..book import Block, Book, BookError
from . import chunk

# A trailing "CHR 24.1" is the printed page.paragraph reference many
# archive.org digital editions carry inline. It is the real citation key, so
# it is lifted into page/para and stripped from the embedded text.
REF_RE = re.compile(r'\s*([A-Z][A-Za-z0-9]{1,9})\s+(\d{1,4})\.(\d{1,3})\s*$')
PAGEMARK_RE = re.compile(r'^\[(\d{1,4})\]$')

# archive.org prepends a scanner disclaimer to every EPUB it generates, and
# appends a per-page accuracy banner. It is scanner metadata, not book text.
# Project Gutenberg prepends its own transcriber credits and licence header.
BOILERPLATE_RE = re.compile(
    r'produced in EPUB format by the Internet Archive'
    r'|relies on optical character recognition'
    r'|scanned and converted to EPUB format automatically'
    r'|this page is estimated to be'
    r'|The Internet Archive was founded in 1996'
    r'|archive\.org/details'
    r'|Online Distributed Proofreading Team'
    r'|pgdp\.net'
    r'|Project Gutenberg',
    re.I)

# A line that is mostly bare numbers is a table of contents, an index or a
# page-number column, not prose.
NUMERIC_RE = re.compile(r'^[\d.,;:—–-]+$')

_DC_NS = {"dc": "http://purl.org/dc/elements/1.1/"}


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def spine_docs(zf: zipfile.ZipFile) -> list[str]:
    """Content documents in reading order, resolved through the OPF spine."""
    container = zf.read("META-INF/container.xml").decode("utf-8", "replace")
    m = re.search(r'full-path="([^"]+)"', container)
    if not m:
        raise BookError("EPUB META-INF/container.xml has no rootfile full-path")
    opf_path = m.group(1)
    opf = zf.read(opf_path).decode("utf-8", "replace")
    base = os.path.dirname(opf_path)
    ids = dict(re.findall(r'<item\b[^>]*?id="([^"]+)"[^>]*?href="([^"]+)"', opf))
    for href, i in re.findall(r'<item\b[^>]*?href="([^"]+)"[^>]*?id="([^"]+)"', opf):
        ids.setdefault(i, href)
    names = set(zf.namelist())
    out = []
    for idref in re.findall(r'<itemref\b[^>]*?idref="([^"]+)"', opf):
        href = ids.get(idref)
        if not href:
            continue
        href = html.unescape(href).split("#")[0]
        path = os.path.normpath(os.path.join(base, href)) if base else href
        if path in names and re.search(r'\.x?html?$', path, re.I):
            out.append(path)
    return out, opf_path, opf


def _opf_metadata(opf_xml: str) -> dict:
    try:
        root = ET.fromstring(opf_xml)
    except ET.ParseError:
        return {}

    def first(tag):
        el = root.find(f".//dc:{tag}", _DC_NS)
        return el.text.strip() if el is not None and el.text else None

    return {
        "title": first("title"),
        "creator": first("creator"),
        "date": first("date"),
        "rights": first("rights"),
    }


def blocks(zf: zipfile.ZipFile, path: str) -> list[str]:
    raw = zf.read(path).decode("utf-8", "replace")
    raw = re.sub(r'<(script|style)\b.*?</\1>', ' ', raw, flags=re.S | re.I)
    body = re.search(r'<body[^>]*>(.*)</body>', raw, re.S | re.I)
    raw = body.group(1) if body else raw
    out = []
    for m in re.finditer(r'<(p|h[1-6]|li|blockquote)\b[^>]*>(.*?)</\1>', raw, re.S | re.I):
        inner = re.sub(r'<br\s*/?>', ' ', m.group(2), flags=re.I)
        text = html.unescape(re.sub(r'<[^>]+>', '', inner))
        text = re.sub(r'[\s\xa0]+', ' ', text).strip()
        if text:
            out.append(text)
    return out


def _year_from_date(date_str: str | None) -> int | None:
    if not date_str:
        return None
    m = re.search(r'(1[5-9]\d{2}|20\d{2})', date_str)
    return int(m.group(1)) if m else None


def extract(
    path: Path,
    *,
    book_code: str | None = None,
    lang: str = "en",
    title: str | None = None,
    author: str | None = None,
    year: int | None = None,
    corpus: str | None = None,
    slug: str | None = None,
    book_pair: str | None = None,
    acquired_from: str | None = None,
    rights: str | None = None,
) -> Book:
    """Extract one EPUB into a :class:`Book`.

    ``book_code`` is taken, in order: the explicit argument; an inline
    citation code detected in >=40% of blocks (the Jones/Waggoner-style
    printed-reference scheme); otherwise left ``None`` for hand entry in the
    reviewed book.json (never guessed from the filename — rule #9).
    Likewise ``title``/``author``/``year`` prefer the explicit argument, then
    the EPUB's own OPF ``<dc:*>`` metadata, then ``None``.
    """
    path = Path(path)
    if not path.exists():
        raise BookError(f"no such file: {path}")

    with zipfile.ZipFile(path) as zf:
        docs, opf_path, opf_xml = spine_docs(zf)
        opf_meta = _opf_metadata(opf_xml)
        raw_blocks = [(i + 1, b) for i, d in enumerate(docs) for b in blocks(zf, d)]

    hits: collections.Counter = collections.Counter()
    for _, b in raw_blocks:
        m = REF_RE.search(b)
        if m:
            hits[m.group(1)] += 1
    inline_code, inline_n = (hits.most_common(1)[0] if hits else (None, 0))
    coded = inline_code is not None and inline_n >= 0.4 * max(len(raw_blocks), 1)

    resolved_code = book_code or (inline_code if coded else None)
    resolved_title = title or opf_meta.get("title")
    resolved_author = author or opf_meta.get("creator")
    resolved_year = year if year is not None else _year_from_date(opf_meta.get("date"))
    resolved_rights = rights or opf_meta.get("rights")

    out_blocks: list[Block] = []
    dropped_detail: list[dict] = []
    seen: collections.Counter = collections.Counter()
    page, para_in_page, saw_page_marker = 0, 0, False
    blocks_in = 0

    for doc_no, block_text in raw_blocks:
        pm = PAGEMARK_RE.match(block_text)
        if pm:                                    # bare "[89]" printed-page marker
            page, para_in_page, saw_page_marker = int(pm.group(1)), 0, True
            continue
        blocks_in += 1

        m = REF_RE.search(block_text)
        if coded and m and m.group(1) == inline_code:
            text = block_text[:m.start()].strip()
            key_page, key_para = int(m.group(2)), int(m.group(3))
        else:
            text = block_text
            if saw_page_marker:                   # printed-page markers in this file
                para_in_page += 1
                key_page, key_para = page, para_in_page
            else:                                 # fall back to chapter.ordinal
                if doc_no != page:
                    page, para_in_page = doc_no, 0
                para_in_page += 1
                key_page, key_para = doc_no, para_in_page

        para_key = f"{key_page}.{key_para}"

        if BOILERPLATE_RE.search(text):
            dropped_detail.append({"para_key": para_key, "reason": "boilerplate",
                                    "text": text[:80]})
            continue
        tokens = text.split()
        if len(tokens) > 20 and sum(bool(NUMERIC_RE.match(t)) for t in tokens) > 0.35 * len(tokens):
            dropped_detail.append({"para_key": para_key, "reason": "numeric/TOC line",
                                    "text": text[:80]})
            continue

        reason = chunk.quality_gate(text)
        if reason:
            dropped_detail.append({"para_key": para_key, "reason": reason, "text": text[:80]})
            continue

        pieces = chunk.split_long(text)
        base_seq = seen[para_key]
        for j, piece in enumerate(pieces):
            out_blocks.append(Block(
                para_key=para_key, page=key_page, para=key_para,
                seq=base_seq + j, chunks=len(pieces), text=piece,
                words=len(piece.split()),
            ))
        seen[para_key] += len(pieces)

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
        "book_code": resolved_code,
        "lang": lang,
        "book_pair": book_pair if book_pair is not None else resolved_code,
        "title": resolved_title,
        "author": resolved_author,
        "year": resolved_year,
        "slug": slug,
        "corpus": corpus,
        "page_kind": "print" if (coded or saw_page_marker) else "chapter",
    }

    from .. import contract  # local import: keep the module's import graph flat

    return Book(
        schema=contract.SCHEMA_BOOK,
        profile="sop",
        source={
            "file": str(path),
            "sha256": _sha256_file(path),
            "kind": "epub",
            "acquired_from": acquired_from,
            "rights": resolved_rights,
        },
        book=book_meta,
        id_rule="sop/seq",
        alignment=None,
        stats=stats,
        blocks=out_blocks,
    )

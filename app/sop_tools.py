"""SoP translation tools — MCP server.

MCP exposing the Spirit-of-Prophecy lookup operations the translator agent
needs. The server queries the ``sop`` Qdrant collection directly (built by
``build_sop_vector_index.py``, which also creates the ``lang`` / ``book_code``
/ ``page`` payload indexes these filters rely on); it embeds queries locally
with fastembed and talks to Qdrant over its REST API. Runs over stdio when
executed directly, or inside the networked ``qdrant/`` server.
This mirrors ``bible_tools_mcp.py``.

Tools::

    sop_lookup(query | queries, codes?)          — semantic Pass 1 + Pass 2 cascade (batchable)
    sop_book_paragraphs(code, p1, p2?, lang?)    — explicit book page-range fetch (paged)
    sop_list_books(lang?, search?)               — languages, or books (codes + titles)
    sop_context(code, para_key, lang?, ...)      — a few paragraphs either side of an anchor
    sop_parallel(code, para_key, lang?, ...)     — paragraph-level de<->en alignment
    sop_by_bible_ref(osis, lang?, limit?)        — paragraphs quoting a given verse (needs backfill)

Configuration: set ``QDRANT_URL`` to override the default ``http://localhost:6333``.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

import requests
from mcp.server.fastmcp import FastMCP
from fastembed import TextEmbedding


_QDRANT_URL  = os.environ.get("QDRANT_URL", "http://localhost:6333")
_TIMEOUT_S   = float(os.environ.get("SOP_TIMEOUT_S", "30"))
_COLLECTION  = "sop"
_EMBEDDING_MODEL = "intfloat/multilingual-e5-large"
_VECTOR_NAME     = "fast-multilingual-e5-large"
# e5 requires queries to be embedded with the "query: " prefix (passages are
# indexed with "passage: "). Must match build_sop_vector_index.py.
_QUERY_PREFIX = "query: "
_PASS2_THRESHOLD = 0.50
_MAX_BATCH = 50
_MAX_PARAGRAPHS = 400

mcp = FastMCP("sop-tools")

# Lazy-loaded embedder — an idle MCP pays no model-load cost.
_embedder = None


def _get_embedder() -> TextEmbedding:
    global _embedder
    if _embedder is None:
        _embedder = TextEmbedding(_EMBEDDING_MODEL)
    return _embedder


def _qdrant(path: str, body: dict) -> Any:
    r = requests.post(f"{_QDRANT_URL}/collections/{_COLLECTION}/{path}",
                      json=body, timeout=_TIMEOUT_S)
    r.raise_for_status()
    return r.json().get("result")


def _format_hit(point: dict) -> dict:
    pl = point.get("payload", {})
    return {
        "book_code": pl.get("book_code"),
        "page":      pl.get("page"),
        "para_key":  pl.get("para_key"),
        "score":     round(point.get("score", 0.0), 3),
        "text":      pl.get("raw_text", ""),
    }


def _search_body(vector: list[float], *, codes: list[str], lang: str,
                 limit: int, min_score: float) -> dict:
    """One filtered vector search against the ``sop`` collection."""
    must = [{"key": "lang", "match": {"value": lang}}]
    if codes:
        must.append({"key": "book_code", "match": {"any": codes}})
    return {
        "vector": {"name": _VECTOR_NAME, "vector": vector},
        "limit": limit,
        "with_payload": True,
        "score_threshold": min_score,
        "filter": {"must": must},
    }


def _search_batch(bodies: list[dict]) -> list[list[dict]]:
    if not bodies:
        return []
    results = _qdrant("points/search/batch", {"searches": bodies})
    return [[_format_hit(p) for p in hits] for hits in results]


@mcp.tool()
def sop_lookup(query: str | None = None, codes: list[str] | None = None,
               lang: str = "de", min_score: float = 0.55, limit: int = 1,
               queries: list[str | dict] | None = None) -> dict:
    """Semantic lookup against the SoP Qdrant index — one query or a batch.

    Runs Pass 1 (constrained by ``codes`` if non-empty) at ``min_score``,
    returning the top ``limit`` results. If Pass 1 finds anything, ``hits``
    is populated and ``fallbacks`` is ``[]``. Otherwise Pass 2 runs
    corpus-wide at threshold 0.50, populating ``fallbacks`` (up to ``limit``).
    Both empty → no usable paragraph in that language, route to Tier 4 (DeepL).

    **Batch many paragraphs in one call** with ``queries`` — all texts are
    embedded together and searched in one Qdrant round trip.

    **Query in the language you are asking for.** The index is multilingual
    (one embedding space), but an English query against ``lang="ko"`` scores
    far worse than a Korean one.

    Args:
        query:     Source text whose canonical rendering in *lang* you want.
                   Strip the trailing citation suffix for best embedding fit.
                   Use either ``query`` or ``queries``.
        codes:     Pre-resolved book codes for the cited work. German is named
                   by **DE** code (``["BW", "WZC"]`` for Steps to Christ);
                   every other language is named by **English** code
                   (``["SC"]``, ``["DA"]``). Pass ``[]`` or omit for a
                   corpus-wide search. See ``sop_list_books``. In batch mode
                   this is the default for items that give no codes of their own.
        lang:      Index language to query: "de", "en", "es", "fr", "it", "ja",
                   "ko", "pt", "ro", "ru", "uk", "zh" — coverage varies widely
                   (``sop_list_books()`` lists paragraph counts). Default "de".
        min_score: Pass-1 threshold. Default 0.55 (per AGENT-translator.md).
        limit:     Top-N results to return per pass. Default 1; clamp 1..20.
                   Use 3 when you need to compare alternative DE editions
                   (e.g. BW vs WZC for the same SC passage).
        queries:   Batch mode — up to 50 items, each a string or
                   ``{"query": "...", "codes": ["BW"]}``.

    Returns:
        Single: ``{"hits": [...], "fallbacks": [...]}``.
        Batch:  ``{"results": [{"query", "hits", "fallbacks"}, ...]}`` in input order.
        Each result carries ``book_code``, ``page``, ``para_key``, ``score``, ``text``.
    """
    limit = max(1, min(20, int(limit)))
    batch = queries is not None
    if batch:
        items = [{"query": q, "codes": codes} if isinstance(q, str) else
                 {"query": q.get("query", ""), "codes": q.get("codes", codes)} for q in queries]
    elif query is not None:
        items = [{"query": query, "codes": codes}]
    else:
        return {"error": "pass `query` or `queries`"}
    if len(items) > _MAX_BATCH:
        return {"error": f"at most {_MAX_BATCH} queries per call"}
    if not items:
        return {"results": []}

    vectors = [[float(x) for x in v]
               for v in _get_embedder().embed([_QUERY_PREFIX + it["query"] for it in items])]

    # Pass 1: constrained by `codes` if provided.
    pass1 = _search_batch([
        _search_body(v, codes=list(it["codes"] or []), lang=lang, limit=limit, min_score=min_score)
        for v, it in zip(vectors, items)])
    # Pass 2: corpus-wide, only for the items Pass 1 left empty.
    empty = [i for i, hits in enumerate(pass1) if not hits]
    pass2 = _search_batch([
        _search_body(vectors[i], codes=[], lang=lang, limit=limit, min_score=_PASS2_THRESHOLD)
        for i in empty])
    fallbacks = dict(zip(empty, pass2))

    results = [{"query": it["query"], "hits": pass1[i], "fallbacks": fallbacks.get(i, [])}
               for i, it in enumerate(items)]
    if not batch:
        return {"hits": results[0]["hits"], "fallbacks": results[0]["fallbacks"]}
    return {"results": results}


@mcp.tool()
def sop_book_paragraphs(book_code: str, page_from: int, page_to: int | None = None,
                        lang: str = "de") -> dict:
    """Fetch paragraphs for an explicit book + page range in one language.

    Use when you know exactly which edition + page you need — for example when
    reading the rendering around a verified Qdrant hit. Returns paragraphs
    ordered by page then paragraph.

    Args:
        book_code:  Book code in the *lang* corpus: a **DE** code for German
                    (``"BW"``, ``"WZC"``, ``"DM"``), the **English** code for
                    every other language (``"SC"``, ``"DA"``, ``"GC"``).
        page_from:  First page to include.
        page_to:    Last page to include (inclusive). Defaults to ``page_from``.
        lang:       Index language ("de", "en", "es", "ja", "ko", "ro", "ru",
                    "uk", …). Default "de".

    Returns:
        ``{"paragraphs": [{book_code, page, para, para_key, text}, ...]}``.
        Large ranges are cut at a page boundary after ~400 paragraphs; the
        response then carries ``"truncated": true`` and ``"next_page_from"``
        — call again from that page to continue.
    """
    if page_to is None:
        page_to = page_from

    must = [
        {"key": "lang",      "match": {"value": lang}},
        {"key": "book_code", "match": {"value": book_code}},
        {"key": "page",      "range": {"gte": page_from, "lte": page_to}},
    ]
    # Ordered by page (integer index) so a cap lands on the lowest pages.
    body = {"limit": _MAX_PARAGRAPHS + 1, "with_payload": True, "filter": {"must": must},
            "order_by": {"key": "page", "direction": "asc"}}
    points = _qdrant("points/scroll", body).get("points", [])
    paragraphs = [
        {
            "book_code": p["payload"]["book_code"],
            "page":      p["payload"]["page"],
            "para":      p["payload"]["para"],
            "para_key":  p["payload"]["para_key"],
            "text":      p["payload"]["raw_text"],
        }
        for p in points
    ]
    paragraphs.sort(key=lambda x: (x["page"], x["para"]))

    out: dict[str, Any] = {"paragraphs": paragraphs}
    if len(paragraphs) > _MAX_PARAGRAPHS:
        last_page = paragraphs[_MAX_PARAGRAPHS]["page"]
        kept = [p for p in paragraphs if p["page"] < last_page]
        if not kept:  # one page alone exceeds the cap — return it whole
            body["limit"] = 10_000
            body["filter"]["must"][2] = {"key": "page", "match": {"value": last_page}}
            kept = sorted(({"book_code": p["payload"]["book_code"], "page": p["payload"]["page"],
                            "para": p["payload"]["para"], "para_key": p["payload"]["para_key"],
                            "text": p["payload"]["raw_text"]}
                           for p in _qdrant("points/scroll", body).get("points", [])),
                          key=lambda x: x["para"])
            last_page += 1
        out = {"paragraphs": kept}
        if last_page <= page_to:
            out.update(truncated=True, next_page_from=last_page)
    return out


def _book_titles() -> dict:
    """Code → titles tables (``data/sop_books.json``, see scripts/export_book_titles.py)."""
    global _titles
    if _titles is None:
        here = Path(__file__).resolve().parent
        candidates = [os.environ.get("SOP_BOOKS_JSON", ""), here / "data" / "sop_books.json",
                      here.parent / "qdrant" / "app" / "data" / "sop_books.json"]
        _titles = {}
        for c in candidates:
            if c and Path(c).is_file():
                _titles = json.loads(Path(c).read_text(encoding="utf-8"))
                break
    return _titles


_titles: dict | None = None


@mcp.tool()
def sop_list_books(lang: str | None = None, search: str | None = None) -> dict:
    """List the SoP corpus: languages, or the books of one language with titles.

    Use it to find the right ``codes`` / ``book_code`` before a lookup instead
    of guessing — German uses its own **DE** codes (``BW`` and ``WZC`` are both
    Steps to Christ), every other language uses **English** codes (``SC``, ``DA``).

    Args:
        lang:    Omit to list languages with paragraph counts; give a language
                 code ("de", "ja", …) to list its books.
        search:  Optional case-insensitive filter on code or title, in any
                 language (``"Steps to Christ"``, ``"Messias"``, ``"GC"``). With
                 ``lang`` omitted it searches every language.

    Returns:
        ``{"languages": [{lang, paragraphs}, ...]}`` or
        ``{"books": [{lang, book_code, paragraphs, titles, en_code?, en_titles?}, ...]}``
        sorted by language then code. ``titles`` are the edition's own titles
        where known; ``en_titles`` give the English work for translated editions.
    """
    titles = _book_titles()
    if not lang and not search:
        hits = _qdrant("facet", {"key": "lang", "limit": 1000, "exact": True})["hits"]
        return {"languages": [{"lang": h["value"], "paragraphs": h["count"]} for h in hits]}

    langs = [lang] if lang else [h["value"] for h in
                                 _qdrant("facet", {"key": "lang", "limit": 1000, "exact": True})["hits"]]
    needle = (search or "").casefold()
    books = []
    for lg in langs:
        hits = _qdrant("facet", {"key": "book_code", "limit": 10_000, "exact": True,
                                 "filter": {"must": [{"key": "lang", "match": {"value": lg}}]}})["hits"]
        for h in hits:
            code = h["value"]
            own = titles.get(lg, {}).get(code, {})
            en_code = own.get("en_code") or (code if lg != "de" else None)
            book = {"lang": lg, "book_code": code, "paragraphs": h["count"],
                    "titles": own.get("titles", [])}
            if lg != "en" and en_code:
                book["en_code"] = en_code
                book["en_titles"] = titles.get("en", {}).get(en_code, {}).get("titles", [])
            if lg == "en":
                book["titles"] = titles.get("en", {}).get(code, {}).get("titles", [])
            haystack = " ".join([code, *book["titles"], *book.get("en_titles", [])]).casefold()
            if needle and needle not in haystack:
                continue
            books.append(book)
    books.sort(key=lambda b: (b["lang"], b["book_code"]))
    out: dict[str, Any] = {"books": books}
    if not books:
        out["error"] = f"no SoP books match lang={lang!r} search={search!r}"
    return out



def _de_codes_for_en(en_code: str) -> list[str]:
    """DE book codes whose ``en_code`` matches (can be >1 — e.g. BW and WZC
    both map to SC)."""
    titles = _book_titles()
    return sorted(code for code, meta in titles.get("de", {}).items()
                  if meta.get("en_code") == en_code)


def _en_code_for_de(de_code: str) -> str | None:
    titles = _book_titles()
    return titles.get("de", {}).get(de_code, {}).get("en_code")


@mcp.tool()
def sop_context(book_code: str, para_key: str, lang: str = "en",
                before: int = 2, after: int = 2) -> dict:
    """Neighbouring paragraphs around one SoP paragraph.

    Cheaper than ``sop_book_paragraphs`` when you only need a few paragraphs
    either side of a paragraph you already have — for example to read more
    context around a verified Qdrant hit — rather than fetching a whole
    explicit page range.

    Args:
        book_code: Book code in the *lang* corpus (DE code for lang="de", the
                   English code for every other language).
        para_key:  The anchor paragraph's "PAGE.PARA" key.
        lang:      Index language. Default "en".
        before:    Paragraphs to include before the anchor. Default 2; clamp 0..20.
        after:     Paragraphs to include after the anchor. Default 2; clamp 0..20.

    Returns:
        ``{"context": [{book_code, page, para, para_key, text, is_target}, ...]}``
        ordered by page then paragraph, with exactly one entry carrying
        ``is_target: true``. May return fewer than requested near a book's
        first or last page. ``{"error": "..."}`` if `para_key` isn't found.
    """
    before = max(0, min(int(before), 20))
    after = max(0, min(int(after), 20))
    try:
        page = int(para_key.split(".", 1)[0])
    except (ValueError, AttributeError):
        return {"error": f"not a PAGE.PARA para_key: {para_key!r}"}

    # Generous page-window fetch — a page can hold as few as one paragraph,
    # so `before`/`after` paragraphs can span more pages than that many. Still
    # far cheaper than a whole-book fetch, which is what this tool avoids.
    margin = before + after + 3
    page_from, page_to = max(1, page - margin), page + margin
    must = [
        {"key": "lang",      "match": {"value": lang}},
        {"key": "book_code", "match": {"value": book_code}},
        {"key": "page",      "range": {"gte": page_from, "lte": page_to}},
    ]
    body = {"limit": _MAX_PARAGRAPHS, "with_payload": True, "filter": {"must": must}}
    points = _qdrant("points/scroll", body).get("points", [])
    paragraphs = sorted(
        ({"book_code": p["payload"]["book_code"], "page": p["payload"]["page"],
          "para": p["payload"]["para"], "para_key": p["payload"]["para_key"],
          "text": p["payload"]["raw_text"]}
         for p in points),
        key=lambda x: (x["page"], x["para"]),
    )

    idx = next((i for i, p in enumerate(paragraphs) if p["para_key"] == para_key), None)
    if idx is None:
        return {"error": f"para_key {para_key!r} not found for lang={lang!r} "
                          f"book_code={book_code!r} (or outside the fetch window)"}

    window = paragraphs[max(0, idx - before): idx + after + 1]
    for p in window:
        p["is_target"] = (p["para_key"] == para_key)
    return {"context": window}


@mcp.tool()
def sop_parallel(book_code: str, para_key: str, lang: str = "de",
                 target_lang: str = "en") -> dict:
    """Paragraph-level de<->en alignment for one SoP paragraph.

    Uses the ``aligned`` field Qdrant stores on every point (the paragraph-
    level de<->en cross-reference the German index carries — see
    docs/SOP-INDEX.md's ``en_ref`` / ``en_reverse``, which ``aligned`` mirrors
    into the vector index) plus the book-code cross-reference table
    (``sop_list_books``'s ``en_code``). Only the de<->en direction is
    populated in the corpus — ja/ko carry no alignment at all (their Qdrant
    points have ``aligned: null``), so any other (lang, target_lang) pair
    returns an error rather than a silently-empty result. This is
    **paragraph-level** alignment, not the book/page-level lookup
    ``book_map.json`` alone would give — it is the more precise of the two,
    where the corpus has it.

    Args:
        book_code:   Book code in the *lang* corpus (DE code for lang="de",
                     EN code for lang="en").
        para_key:    The paragraph's "PAGE.PARA" key in the *lang* edition.
        lang:        Source language. Default "de".
        target_lang: Language to align into. Default "en".

    Returns:
        ``{"source": {...}, "target": [{...}, ...]}`` — `target` may hold more
        than one paragraph (a single paragraph sometimes maps to several on
        the other side). ``{"error": "..."}`` if the language pair is
        unsupported or the source paragraph doesn't resolve.
    """
    if lang == target_lang:
        return {"error": "lang and target_lang must differ"}
    if {lang, target_lang} != {"de", "en"}:
        return {"error": "only de<->en alignment is available in this corpus "
                          "(ja/ko carry no paragraph-level alignment)"}

    must = [
        {"key": "lang",      "match": {"value": lang}},
        {"key": "book_code", "match": {"value": book_code}},
        {"key": "para_key",  "match": {"value": para_key}},
    ]
    points = _qdrant("points/scroll",
                     {"limit": 1, "with_payload": True, "filter": {"must": must}}).get("points", [])
    if not points:
        return {"error": f"no point found for lang={lang!r} book_code={book_code!r} "
                          f"para_key={para_key!r}"}
    src = points[0]["payload"]
    source = {"book_code": src.get("book_code"), "lang": lang, "page": src.get("page"),
              "para": src.get("para"), "para_key": src.get("para_key"),
              "text": src.get("raw_text", "")}

    if lang == "de":  # de -> en: read the forward `aligned` list directly
        aligned_keys = src.get("aligned") or []
        if not aligned_keys:
            return {"source": source, "target": [],
                    "note": "this paragraph carries no 'aligned' EN reference"}
        en_code = _en_code_for_de(book_code)
        if not en_code:
            return {"error": f"no EN counterpart known for DE book_code={book_code!r}"}
        must_t = [
            {"key": "lang",      "match": {"value": "en"}},
            {"key": "book_code", "match": {"value": en_code}},
            {"key": "para_key",  "match": {"any": aligned_keys}},
        ]
        pts = _qdrant("points/scroll", {"limit": len(aligned_keys) + 5, "with_payload": True,
                                        "filter": {"must": must_t}}).get("points", [])
    else:  # en -> de: no reverse field on the EN point — find DE points whose
           # `aligned` list contains this EN para_key (array-membership match).
        de_codes = _de_codes_for_en(book_code)
        if not de_codes:
            return {"error": f"no DE counterpart known for EN book_code={book_code!r}"}
        must_t = [
            {"key": "lang",      "match": {"value": "de"}},
            {"key": "book_code", "match": {"any": de_codes}},
            {"key": "aligned",   "match": {"value": para_key}},
        ]
        pts = _qdrant("points/scroll", {"limit": _MAX_PARAGRAPHS, "with_payload": True,
                                        "filter": {"must": must_t}}).get("points", [])

    target = sorted(
        ({"book_code": p["payload"].get("book_code"), "lang": target_lang,
          "page": p["payload"].get("page"), "para": p["payload"].get("para"),
          "para_key": p["payload"].get("para_key"), "text": p["payload"].get("raw_text", "")}
         for p in pts),
        key=lambda x: (x["page"] or 0, x["para"] or 0),
    )
    return {"source": source, "target": target}


@mcp.tool()
def sop_by_bible_ref(osis: str, lang: str = "en", limit: int = 20) -> dict:
    """Find SoP paragraphs that quote a given Bible reference.

    Needs the ``bible_refs`` payload field created by the offline backfill
    (``sdarm.tools.extract_sop_refs --qdrant-backfill``) — most of the corpus
    does not carry it yet, so this returns a clear, actionable error instead
    of a silently-empty result when the field isn't indexed at all.

    Args:
        osis:  A single OSIS reference, e.g. ``"John.3.16"`` or ``"1Cor.15.3"``
               (exact match — no range expansion; a paragraph citing
               ``"John.3.16-18"`` is only found if the backfill recorded that
               exact key).
        lang:  Index language to search. Default "en".
        limit: Max rows to return. Default 20; clamp 1..200.

    Returns:
        ``{"results": [{book_code, page, para, para_key, text, bible_refs}, ...]}``.
        ``{"error": "..."}`` if ``bible_refs`` is not indexed yet.
    """
    info = requests.get(f"{_QDRANT_URL}/collections/{_COLLECTION}", timeout=_TIMEOUT_S)
    info.raise_for_status()
    schema = info.json().get("result", {}).get("payload_schema", {})
    if "bible_refs" not in schema:
        return {"error": "The 'bible_refs' payload field is not indexed on the 'sop' "
                          "collection yet — sop_by_bible_ref needs the offline backfill. "
                          "Run `python -m sdarm.tools.extract_sop_refs --lang de,en "
                          "--qdrant-backfill --no-dry-run` (see "
                          "generator/src/sdarm/tools/extract_sop_refs.py) first."}

    must = [
        {"key": "lang",       "match": {"value": lang}},
        {"key": "bible_refs", "match": {"value": osis}},
    ]
    body = {"limit": max(1, min(int(limit), 200)), "with_payload": True, "filter": {"must": must}}
    points = _qdrant("points/scroll", body).get("points", [])
    results = [
        {"book_code": p["payload"].get("book_code"), "page": p["payload"].get("page"),
         "para": p["payload"].get("para"), "para_key": p["payload"].get("para_key"),
         "text": p["payload"].get("raw_text", ""), "bible_refs": p["payload"].get("bible_refs", [])}
        for p in points
    ]
    return {"results": results}


def main() -> None:
    """Stdio entry point — claude-code spawns this as a subprocess."""
    mcp.run()


if __name__ == "__main__":
    main()

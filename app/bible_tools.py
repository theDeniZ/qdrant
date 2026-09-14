"""Bible lookup tools — MCP server.

MCP exposing Bible verse query operations. The server queries the ``bibles``
Qdrant collection built by ``build_bible_vector_index.py`` (which also creates
the ``bible`` / ``osis`` payload indexes these filters rely on). Runs over stdio
when executed directly, or inside the networked ``qdrant/`` server.

Tools::

    bible_search(query, bible?, limit?)              — semantic search across verses
    bible_lookup(ref, bible?, numbering?)            — verses by OSIS ref/range/chapter/list
    bible_list_translations()                        — indexed translations + verse counts

Configuration: set ``QDRANT_URL`` to override the default ``http://localhost:6333``.
"""

from __future__ import annotations

import os
import re
from typing import Any

import requests
from mcp.server.fastmcp import FastMCP
from fastembed import TextEmbedding

try:  # inside the qdrant/ server package
    from .versification import has_mapping, remap_verse
except ImportError:
    try:  # run as a script from app/
        from versification import has_mapping, remap_verse
    except ImportError:  # sdarm translator/ copy, next to the generator package
        from sdarm.core.bible.versification import has_mapping, remap_verse

_QDRANT_URL = os.environ.get("QDRANT_URL", "http://localhost:6333")
_TIMEOUT_S = float(os.environ.get("BIBLE_TIMEOUT_S", "30"))
_EMBEDDING_MODEL = "intfloat/multilingual-e5-large"
_VECTOR_NAME = "fast-multilingual-e5-large"
# e5 requires queries to be embedded with the "query: " prefix (verses are
# indexed with "passage: "). Must match build_bible_vector_index.py.
_QUERY_PREFIX = "query: "

mcp = FastMCP("bible-tools")

# Lazy-loaded embedder
_embedder = None


def _get_embedder() -> TextEmbedding:
    global _embedder
    if _embedder is None:
        _embedder = TextEmbedding(_EMBEDDING_MODEL)
    return _embedder


@mcp.tool()
def bible_search(
    query: str,
    bible: str | None = None,
    limit: int = 5,
    min_score: float = 0.5,
) -> dict:
    """Semantic search across Bible verses.

    Embeds the query and returns the most similar verses from the specified
    Bible translation (or all translations if not specified).

    Args:
        query:     Search query (e.g., "hope in darkness", "forgiveness").
        bible:     Bible translation to search — "kjv", "nkjv", "net",
                   "luther1912", "schlachter", "elberfelder1905", "synodal",
                   "ukrogienko", "spanish", "japkougo" (口語訳), "korean"
                   (개역한글). Omit to search all translations; call
                   ``bible_list_translations`` to see what is actually indexed.
        limit:     Number of results to return. Default 5; clamp 1..20.
        min_score: Similarity threshold (0.0-1.0). Default 0.5.

    Returns:
        ``{"results": [...]}`` where each result carries ``osis``, ``bible``,
        ``text``, ``score``. ``osis`` is in the translation's own numbering.
    """
    embedder = _get_embedder()
    embedding = list(embedder.embed([_QUERY_PREFIX + query]))[0]

    must_filters = []
    if bible:
        must_filters.append({"key": "bible", "match": {"value": bible}})

    search_body = {
        "vector": {"name": _VECTOR_NAME, "vector": list(embedding)},
        "limit": min(max(limit, 1), 20),
        "with_payload": True,
        "score_threshold": min_score,
    }
    if must_filters:
        search_body["filter"] = {"must": must_filters}

    r = requests.post(
        f"{_QDRANT_URL}/collections/bibles/points/search",
        json=search_body,
        timeout=_TIMEOUT_S,
    )
    r.raise_for_status()
    points = r.json().get("result", [])

    results = []
    for p in points:
        payload = p.get("payload", {})
        result = {
            "osis": payload.get("osis"),
            "bible": payload.get("bible"),
            "text": payload.get("text", ""),
            "score": p.get("score", 0),
        }
        results.append(result)

    return {"results": results}


_MAX_VERSES_PER_CHAPTER = 176  # Ps.119 — upper bound for open chapter spans
_MAX_KEYS = 2000
_OSIS_RE = re.compile(r"^([1-4]?[A-Za-z]+)\.(\d+)(?:\.(\d+))?$")


def _expand_ref(ref: str) -> list[str]:
    """Expand one OSIS ref / range into candidate verse keys, in order.

    ``John.3.16`` · ``John.3.16-18`` · ``John.3.16-John.4.2`` · ``Ps.23``
    (whole chapter) · ``Ps.23-Ps.24``. Keys past a chapter's real end simply
    match nothing.
    """
    start, _, end = ref.strip().partition("-")
    m1 = _OSIS_RE.match(start)
    if not m1:
        raise ValueError(f"not an OSIS reference: {ref!r}")
    book, c1, v1 = m1.group(1), int(m1.group(2)), m1.group(3)
    if not end:
        if v1:
            return [f"{book}.{c1}.{v1}"]
        c2, v2 = c1, None
    elif end.isdigit():  # John.3.16-18 (same chapter) or Ps.23-24 (chapters)
        c2, v2 = (c1, int(end)) if v1 else (int(end), None)
    else:
        m2 = _OSIS_RE.match(end)
        if not m2 or m2.group(1) != book:
            raise ValueError(f"range must stay within one book: {ref!r}")
        c2, v2 = int(m2.group(2)), (int(m2.group(3)) if m2.group(3) else None)
    first = int(v1) if v1 else 1
    keys = []
    for ch in range(c1, c2 + 1):
        lo = first if ch == c1 else 1
        hi = v2 if (ch == c2 and v2 is not None) else _MAX_VERSES_PER_CHAPTER
        keys.extend(f"{book}.{ch}.{v}" for v in range(lo, hi + 1))
    return keys


def _scroll_all(must: list[dict] | None = None, should: list[dict] | None = None,
                payload: bool | list[str] = True) -> list[dict]:
    flt: dict[str, Any] = {}
    if must:
        flt["must"] = must
    if should:
        flt["should"] = should
    points, offset = [], None
    while True:
        body: dict[str, Any] = {"limit": 1000, "with_payload": payload, "filter": flt}
        if offset is not None:
            body["offset"] = offset
        r = requests.post(f"{_QDRANT_URL}/collections/bibles/points/scroll",
                          json=body, timeout=_TIMEOUT_S)
        r.raise_for_status()
        result = r.json().get("result", {})
        points.extend(result.get("points", []))
        offset = result.get("next_page_offset")
        if offset is None:
            return points


def _facet_bibles() -> list[dict]:
    r = requests.post(f"{_QDRANT_URL}/collections/bibles/facet",
                      json={"key": "bible", "limit": 1000, "exact": True}, timeout=_TIMEOUT_S)
    r.raise_for_status()
    return r.json().get("result", {}).get("hits", [])


def _to_edition(key: str, bible: str) -> str:
    b, c, v = key.split(".")
    return "%s.%d.%d" % remap_verse(b, int(c), int(v), bible)


@mcp.tool()
def bible_lookup(ref: str, bible: str | list[str] | None = None,
                 numbering: str = "edition") -> dict:
    """Fetch verse text by OSIS reference — single verses, ranges, chapters, lists.

    One call replaces a verse-by-verse loop.

    Args:
        ref:       One or more OSIS references separated by ``;`` or ``,``:
                   ``"John.3.16"``, ``"John.3.16-18"``, ``"John.3.36-John.4.2"``,
                   ``"Ps.23"`` (whole chapter), ``"Gen.1.1; Rom.8.28-30"``.
        bible:     A translation ("kjv", "luther1912", "schlachter", "synodal",
                   "japkougo", "korean", …) or a list of them. Omit for every
                   indexed translation (see ``bible_list_translations``).
        numbering: How to read the verse numbers in ``ref``:

                   * ``"edition"`` (default) — each translation's **own**
                     numbering: ``Ps.51.3`` in luther1912 is "Gott, sei mir
                     gnädig", ``Ps.51.1`` is the superscription.
                   * ``"kjv"`` — KJV/English numbering (lesson ``sOsis`` refs),
                     remapped per translation where versification differs
                     (luther1912 Hebrew OT numbering, synodal, ukrogienko).
                     ``Ps.51.1`` then returns Luther ``Ps.51.3``. Every verse
                     carries ``kjv_osis``; ``osis`` is the edition's own key —
                     print that number when quoting the edition. Exception:
                     the ``synodal`` index stores **Daniel** under KJV numbers,
                     so the printed Synodal Daniel verse is often ``osis`` + 1
                     (``Dan.6.10`` → print 6:11); probe the chapter's last
                     verse when in doubt. And ``ukrogienko`` **Psalms** are
                     stored in Septuagint numbering, which the remap table
                     does not model yet — look those up with
                     ``numbering="edition"`` (KJV Ps 23 = Ps 22) and check the words.

    Returns:
        ``{"verses": [{osis, bible, text, kjv_osis?}, ...]}`` ordered by
        reference, then by the order of ``bible``; plus
        ``"not_found": [ref, ...]`` for references that matched nothing.
        ``{"error": "..."}`` for malformed references.
    """
    if numbering not in ("edition", "kjv"):
        return {"error": "numbering must be 'edition' or 'kjv'"}
    refs = [r.strip() for r in re.split(r"[;,]", ref) if r.strip()]
    try:
        per_ref = [_expand_ref(r) for r in refs]
    except ValueError as exc:
        return {"error": str(exc)}
    keys = list(dict.fromkeys(k for ks in per_ref for k in ks))
    if not keys:
        return {"error": "no reference given"}
    if len(keys) > _MAX_KEYS:
        return {"error": f"reference spans too many verses (>{_MAX_KEYS}); split it"}

    bibles = [bible] if isinstance(bible, str) else list(bible or [])
    # edition key → requested key, per bible (identity unless numbering="kjv").
    back: dict[str, dict[str, str]] = {}
    if numbering == "edition":
        must = [{"key": "osis", "match": {"any": keys}}]
        if bibles:
            must.append({"key": "bible", "match": {"any": bibles}})
        points = _scroll_all(must=must)
    else:
        # Keep only verses that exist in KJV, so an open chapter span can't
        # remap past a chapter end onto the next chapter's real verses.
        existing = {p["payload"]["osis"] for p in _scroll_all(
            must=[{"key": "bible", "match": {"value": "kjv"}},
                  {"key": "osis", "match": {"any": keys}}], payload=["osis"])}
        keys = [k for k in keys if k in existing]
        targets = bibles or [h["value"] for h in _facet_bibles()]
        plain = [b for b in targets if not has_mapping(b)]
        should = []
        if plain and keys:
            should.append({"must": [{"key": "bible", "match": {"any": plain}},
                                    {"key": "osis", "match": {"any": keys}}]})
        for b in targets:
            if has_mapping(b) and keys:
                back[b] = {_to_edition(k, b): k for k in keys}
                should.append({"must": [{"key": "bible", "match": {"value": b}},
                                        {"key": "osis", "match": {"any": list(back[b])}}]})
        points = _scroll_all(should=should) if should else []

    key_pos = {k: i for i, k in enumerate(keys)}
    bible_pos = {b: i for i, b in enumerate(bibles)}
    verses, seen = [], set()
    for p in points:
        payload = p.get("payload", {})
        b, osis = payload.get("bible"), payload.get("osis")
        if (b, osis) in seen:
            continue
        seen.add((b, osis))
        verse = {"osis": osis, "bible": b, "text": payload.get("text", "")}
        if numbering == "kjv":
            verse["kjv_osis"] = back.get(b, {}).get(osis, osis)
        verses.append(verse)
    verses.sort(key=lambda v: (key_pos.get(v.get("kjv_osis", v["osis"]), 0),
                               bible_pos.get(v["bible"], len(bible_pos)), v["bible"] or ""))

    found = {v.get("kjv_osis", v["osis"]) for v in verses}
    not_found = [r for r, ks in zip(refs, per_ref) if not found.intersection(ks)]
    out: dict[str, Any] = {"verses": verses}
    if not_found:
        out["not_found"] = not_found
    return out


@mcp.tool()
def bible_list_translations() -> dict:
    """List the indexed Bible translations with their verse counts.

    Call this before assuming a translation exists; pass the returned names as
    ``bible`` to ``bible_lookup`` / ``bible_search``.

    Returns:
        ``{"translations": [{bible, verses, kjv_remapped}, ...]}`` sorted by
        name — ``kjv_remapped`` is true where ``bible_lookup(numbering="kjv")``
        applies a versification mapping.
    """
    return {"translations": sorted(
        ({"bible": h["value"], "verses": h["count"], "kjv_remapped": has_mapping(h["value"])}
         for h in _facet_bibles()), key=lambda t: t["bible"])}


def main() -> None:
    """Stdio entry point — claude-code spawns this as a subprocess."""
    mcp.run()


if __name__ == "__main__":
    main()

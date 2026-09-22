"""Sentence-boundary chunker and OCR-damage/junk gates.

Ported from ``pd-books/qdrant/build_pioneers_corpus.py``, battle-tested
against 51 real pioneer-era OCR'd books. Kept faithful to that reference —
same constants, same regexes, same arithmetic — so a re-extraction of the
pioneer corpus through ``sopack`` reproduces the same chunking decisions.

**Stdlib only.**
"""

from __future__ import annotations

import re

# multilingual-e5-large truncates at 512 tokens ≈ 380 English words. Blocks
# longer than MAX_WORDS are split on sentence boundaries so nothing is
# silently lost; several pioneer books are page-per-block rebuilds with no
# paragraph structure (Miller's Views p95 = 828 words, Bates 666, Smith's
# Daniel and the Revelation 475).
MAX_WORDS = 300
TARGET_WORDS = 220
MIN_WORDS = 8          # drops page numbers, running heads, short headings
# Per-block gate. A work can score clean overall and still open on a
# scrambled title page ("^^i'^M - er^ aniel anb IRrt^flati"). Those blocks
# are dropped on their own.
MAX_BLOCK_DAMAGE = 0.25
MAX_JUNK_CHARS = 0.02

SENT_RE = re.compile(r'(?<=[.!?;:])["”’\']?\s+')

# Characters a normal book actually uses. Anything else — box-drawing
# leftovers, stray diacritics, scanner artefacts — is damage. A scrambled
# title page reads as a handful of one-letter "words" and so scores clean on
# damage_score(); it is the character mix that gives it away.
CLEAN_CHARS_RE = re.compile(r"[A-Za-z0-9\s.,;:!?'’‘\"“”()\[\]{}—–\-/&%$#*@+=°£§¶†‡]")


def split_long(text: str) -> list[str]:
    """Sentence-boundary split of a block that would overflow the encoder."""
    if len(text.split()) <= MAX_WORDS:
        return [text]
    parts, cur, n = [], [], 0
    for sent in SENT_RE.split(text):
        w = len(sent.split())
        if cur and n + w > TARGET_WORDS:
            parts.append(" ".join(cur))
            cur, n = [], 0
        cur.append(sent)
        n += w
    if cur:
        parts.append(" ".join(cur))
    # A single sentence longer than MAX_WORDS still has to be cut somewhere.
    final = []
    for p in parts:
        words = p.split()
        if len(words) <= MAX_WORDS:
            final.append(p)
        else:
            for i in range(0, len(words), TARGET_WORDS):
                final.append(" ".join(words[i:i + TARGET_WORDS]))
    return [p for p in final if p.strip()]


def damage_score(text: str) -> float:
    """Share of tokens that look like OCR wreckage. Crude, but it separates
    'ButastoJesus' and ';^^^UR country's' from ordinary prose."""
    words = re.findall(r"[A-Za-z’']+", text)
    if not words:
        return 1.0
    bad = 0
    for w in words:
        if len(w) > 2 and not w.isupper() and re.search(r'[A-Z]', w[1:]):
            bad += 1
        elif len(w) > 3 and not re.search(r'[aeiouyAEIOUY]', w):
            bad += 1
    junk = len(re.findall(r'[\^~`|\\{}<>]', text))
    return (bad + junk) / len(words)


def junk_char_ratio(text: str) -> float:
    if not text:
        return 1.0
    return 1 - len(CLEAN_CHARS_RE.findall(text)) / len(text)


def quality_gate(text: str, *, min_words: int = MIN_WORDS,
                  max_damage: float = MAX_BLOCK_DAMAGE,
                  max_junk: float = MAX_JUNK_CHARS) -> str | None:
    """``None`` if *text* passes the per-block gates; else a short drop
    reason suitable for a ``stats["dropped_detail"]`` entry (R8 — nothing
    silently discarded)."""
    words = text.split()
    if len(words) < min_words:
        return f"too short ({len(words)} word(s) < {min_words})"
    d = damage_score(text)
    if d > max_damage:
        return f"OCR damage {d:.1%} over {max_damage:.0%} gate"
    j = junk_char_ratio(text)
    if j > max_junk:
        return f"junk-char ratio {j:.1%} over {max_junk:.1%} gate"
    return None

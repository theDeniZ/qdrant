"""KJV → target-versification remapping for scripture references.

Lesson content is authored in English and its `sOsis` references use **KJV
(English) versification**. The German edition resolves verse text from Luther
1912, whose Old Testament follows the **Hebrew/Masoretic** tradition. Where the
two systems number verses differently, an English-numbered reference silently
pulls the *wrong* Luther verse (e.g. KJV ``Ps.51.1`` → Luther 51:1 = the
superscription "Ein Psalm Davids", not "Gott, sei mir gnädig").

This module encodes the documented KJV↔Luther differences from
``docs/VERSIFICATION.md`` as a programmatic mapping. It is intentionally
**advisory only** — the core Bible resolver (:mod:`sdarm.core.bible.osis`) still
looks up `sOsis` keys verbatim. The mapping is consumed exclusively by the
editor's "Find misaligned verses" review tool, which surfaces every reference
sitting in a versification-difference zone so a human can correct the stored
`sOsis` (and its visible label) by hand.

The mapping is keyed by **target translation**. Only ``luther1912`` is encoded;
for any other target :func:`remap_ref` returns the reference unchanged (we have
no verified mapping for it).

Stdlib only — this stays in the pure domain kernel.
"""

from __future__ import annotations

import re

# ── Rule model ──────────────────────────────────────────────────────────────
# A rule says: for KJV ``book chapter:v`` with ``v_lo <= v <= v_hi``, the Luther
# verse is ``(book, tgt_chapter, v + delta)``. Rules are grouped per book; the
# first matching rule wins. Books / chapters with no rule are identical in both
# systems (identity remap).
#
# Each rule tuple is ``(kjv_chapter, v_lo, v_hi, tgt_chapter, delta)``.

_BIG = 200  # upper bound covering any chapter's verse count

# Psalm superscription offsets (see VERSIFICATION.md §1). The offset is uniform
# across the whole psalm: KJV Ps N:v → Luther Ps N:(v + offset).
_PSALM_OFFSET_1 = [
    3, 4, 5, 6, 7, 8, 9, 12, 13, 18, 19, 20, 21, 22, 30, 31, 34, 36, 38, 39, 40,
    41, 42, 44, 45, 46, 47, 48, 49, 53, 55, 56, 57, 58, 59, 61, 62, 63, 64, 65,
    67, 68, 69, 70, 75, 76, 77, 80, 81, 83, 84, 85, 88, 89, 92, 102, 108, 140, 142,
]
_PSALM_OFFSET_2 = [51, 52, 54, 60]


def _build_luther_rules() -> dict[str, list[tuple[int, int, int, int, int]]]:
    rules: dict[str, list[tuple[int, int, int, int, int]]] = {}

    def add(book: str, kjv_ch: int, v_lo: int, v_hi: int, tgt_ch: int, delta: int) -> None:
        rules.setdefault(book, []).append((kjv_ch, v_lo, v_hi, tgt_ch, delta))

    # §1 Psalms — superscription offset (whole psalm).
    for n in _PSALM_OFFSET_1:
        add("Ps", n, 1, _BIG, n, 1)
    for n in _PSALM_OFFSET_2:
        add("Ps", n, 1, _BIG, n, 2)

    # §2 Old Testament — chapter-boundary & block shifts.
    add("Gen", 31, 55, 55, 32, -54)
    add("Gen", 32, 1, 32, 32, 1)
    add("Exod", 8, 1, 4, 7, 25)
    add("Exod", 8, 5, 32, 8, -4)
    add("Exod", 22, 1, 1, 21, 36)
    add("Exod", 22, 2, 31, 22, -1)
    add("Lev", 6, 1, 7, 5, 19)
    add("Lev", 6, 8, 30, 6, -7)
    add("Num", 16, 36, 50, 17, -35)
    add("Num", 17, 1, 13, 17, 15)
    add("Num", 29, 40, 40, 30, -39)
    add("Num", 30, 1, 16, 30, 1)
    add("Deut", 12, 32, 32, 13, -31)
    add("Deut", 13, 1, 18, 13, 1)
    add("Deut", 22, 30, 30, 23, -29)
    add("Deut", 23, 1, 25, 23, 1)
    add("Deut", 29, 1, 1, 28, 68)
    add("Deut", 29, 2, 29, 29, -1)
    add("1Sam", 21, 1, 15, 21, 1)
    add("1Sam", 23, 29, 29, 24, -28)
    add("1Sam", 24, 1, 22, 24, 1)
    add("2Sam", 18, 33, 33, 19, -32)
    add("2Sam", 19, 1, 43, 19, 1)
    add("1Kgs", 4, 21, 34, 5, -20)
    add("1Kgs", 5, 1, 18, 5, 14)
    add("1Kgs", 22, 44, 60, 22, 1)  # internal split: 22:43 stays, 22:n→22:n+1 for n≥44
    add("2Kgs", 11, 21, 21, 12, -20)
    add("2Kgs", 12, 1, 21, 12, 1)
    add("1Chr", 6, 1, 15, 5, 26)
    add("1Chr", 6, 16, 81, 6, -15)
    add("2Chr", 2, 1, 1, 1, 17)
    add("2Chr", 2, 2, 18, 2, -1)
    add("2Chr", 14, 1, 1, 13, 22)
    add("2Chr", 14, 2, 15, 14, -1)
    add("Neh", 4, 1, 6, 3, 32)
    add("Neh", 4, 7, 23, 4, -6)
    add("Neh", 9, 38, 38, 10, -37)
    add("Neh", 10, 1, 39, 10, 1)
    add("Job", 41, 1, 8, 40, 24)
    add("Job", 41, 9, 34, 41, -8)
    add("Eccl", 5, 1, 1, 4, 16)
    add("Eccl", 5, 2, 20, 5, -1)
    add("Song", 6, 13, 13, 7, -12)
    add("Song", 7, 1, 13, 7, 1)
    add("Isa", 9, 1, 1, 8, 22)
    add("Isa", 9, 2, 21, 9, -1)
    add("Isa", 64, 1, 1, 63, 18)  # KJV 64:1 → Luther 63:19 (merge approximation)
    add("Isa", 64, 2, 12, 64, -1)
    add("Jer", 9, 1, 1, 8, 22)
    add("Jer", 9, 2, 26, 9, -1)
    add("Ezek", 20, 45, 49, 21, -44)
    add("Ezek", 21, 1, 32, 21, 5)
    add("Hos", 1, 10, 11, 2, -9)
    add("Hos", 2, 1, 23, 2, 2)
    add("Hos", 11, 12, 12, 12, -11)
    add("Hos", 12, 1, 14, 12, 1)
    add("Hos", 13, 16, 16, 14, -15)
    add("Hos", 14, 1, 9, 14, 1)
    add("Joel", 2, 28, 32, 3, -27)
    add("Joel", 3, 1, 21, 4, 0)
    add("Jonah", 1, 17, 17, 2, -16)
    add("Jonah", 2, 1, 10, 2, 1)
    add("Mic", 5, 1, 1, 4, 13)
    add("Mic", 5, 2, 15, 5, -1)
    add("Nah", 1, 15, 15, 2, -14)
    add("Nah", 2, 1, 13, 2, 1)
    add("Zech", 1, 18, 21, 2, -17)
    add("Zech", 2, 1, 13, 2, 4)
    add("Mal", 4, 1, 6, 3, 18)
    add("Dan", 4, 1, 3, 3, 30)
    add("Dan", 4, 4, 37, 4, -3)
    add("Dan", 5, 31, 31, 6, -30)
    add("Dan", 6, 1, 28, 6, 1)

    # §3 New Testament.
    add("Acts", 19, 41, 41, 19, -1)  # KJV 19:41 merged into Luther 19:40
    add("2Cor", 13, 13, 14, 13, -1)  # 13:13→12 (merge), 13:14→13

    # Not encoded (sub-verse merges/splits that cannot be expressed as a
    # whole-verse shift; verify by hand if referenced):
    #   2Kgs 15 (later split), Rev 12/13 (13:1a = Luther 12:18), 3John 1:14.

    return rules


_LUTHER_RULES = _build_luther_rules()


# ── Ohienko (Ukrainian) ─────────────────────────────────────────────────────
# The Ohienko edition follows the Hebrew/Masoretic tradition, the same system
# Luther 1912 uses, so it inherits the Luther rules. The exception was derived
# empirically by comparing per-chapter verse counts of ukrogienko.json against
# kjv.json and luther1912.json: every ruled book matches the Hebrew structure
# except **Malachi**, where Ohienko keeps the English four-chapter division.
def _build_ukrogienko_rules() -> dict[str, list[tuple[int, int, int, int, int]]]:
    rules = {b: list(rs) for b, rs in _LUTHER_RULES.items() if b != "Mal"}
    return rules


_UKROGIENKO_RULES = _build_ukrogienko_rules()


# ── Synodal (Russian) ───────────────────────────────────────────────────────
# The Synodal text is a hybrid: its Psalter follows the **Septuagint** chapter
# division (unlike Luther/Ohienko, which keep the Hebrew chapters), while for
# the rest of the Old Testament it mostly keeps the English chapter boundaries.
# The book list below was derived the same way — per-chapter verse counts of
# synodal.json compared against kjv.json and luther1912.json.
_SYNODAL_HEBREW_BOOKS = {"1Sam", "2Cor", "Acts", "Eccl", "Job", "Jonah", "Song"}


def _build_synodal_rules() -> dict[str, list[tuple[int, int, int, int, int]]]:
    rules: dict[str, list[tuple[int, int, int, int, int]]] = {
        b: list(rs) for b, rs in _LUTHER_RULES.items()
        if b in _SYNODAL_HEBREW_BOOKS
    }

    # Psalms: Septuagint chapter division *plus* the Hebrew superscription
    # offsets (the Synodal text counts superscriptions as verses, exactly as
    # Luther does). Both shifts therefore compose.
    ps: list[tuple[int, int, int, int, int]] = []

    def offset(n: int) -> int:
        return 2 if n in _PSALM_OFFSET_2 else (1 if n in _PSALM_OFFSET_1 else 0)

    for n in range(1, 151):
        o = offset(n)
        if n <= 9:                      # 1-9 keep their number
            ps.append((n, 1, _BIG, n, o))
        elif n == 10:                   # KJV 10 -> Syn 9:22ff
            ps.append((n, 1, _BIG, 9, 21 + o))
        elif 11 <= n <= 113:            # -1
            ps.append((n, 1, _BIG, n - 1, o))
        elif n == 114:                  # KJV 114+115 -> Syn 113
            ps.append((n, 1, _BIG, 113, o))
        elif n == 115:
            ps.append((n, 1, _BIG, 113, 8 + o))
        elif n == 116:                  # KJV 116 -> Syn 114 + 115
            ps.append((n, 1, 9, 114, o))
            ps.append((n, 10, _BIG, 115, -9 + o))
        elif 117 <= n <= 146:           # -1
            ps.append((n, 1, _BIG, n - 1, o))
        elif n == 147:                  # KJV 147 -> Syn 146 + 147
            ps.append((n, 1, 11, 146, o))
            ps.append((n, 12, _BIG, 147, -11 + o))
        else:                           # 148-150 identical
            ps.append((n, 1, _BIG, n, o))

    rules["Ps"] = ps

    # Romans 16:25-27 (the doxology) is printed as 14:24-26 in the Synodal text.
    rules["Rom"] = [(16, 25, 27, 14, -1)]
    return rules


_SYNODAL_RULES = _build_synodal_rules()


# Target translation → rule table. Translations that share KJV versification
# (kjv, nkjv, net) or that we have not mapped (schlachter, …) are absent →
# identity remap.
_TARGET_RULES: dict[str, dict[str, list[tuple[int, int, int, int, int]]]] = {
    "luther1912": _LUTHER_RULES,
    "ukrogienko": _UKROGIENKO_RULES,
    "synodal": _SYNODAL_RULES,
}


def has_mapping(target: str) -> bool:
    """True if a versification mapping is known for ``target``."""
    return target in _TARGET_RULES


def _parse(ref: str):
    parts = ref.split(".")
    if len(parts) != 3:
        return None
    try:
        return (parts[0], int(parts[1]), int(parts[2]))
    except ValueError:
        return None


def remap_verse(book: str, chapter: int, verse: int, target: str) -> tuple[str, int, int]:
    """Remap a single KJV (book, chapter, verse) to ``target`` versification."""
    for kjv_ch, v_lo, v_hi, tgt_ch, delta in _TARGET_RULES.get(target, {}).get(book, ()):
        if chapter == kjv_ch and v_lo <= verse <= v_hi:
            return (book, tgt_ch, verse + delta)
    return (book, chapter, verse)


def _split_suffix(part: str) -> tuple[str, str]:
    """Split off a sub-verse suffix: 'Isa.46.9a' → ('Isa.46.9', 'a'),
    'Isa.46.9.a' → ('Isa.46.9', '.a'). No suffix → (part, '')."""
    parts = part.split(".")
    if len(parts) == 4 and parts[-1].isalpha():
        return ".".join(parts[:3]), "." + parts[-1]
    if len(parts) == 3:
        m = re.search(r"[a-zA-Z]+$", parts[2])
        if m:
            return f"{parts[0]}.{parts[1]}.{parts[2][:m.start()]}", m.group(0)
    return part, ""


def remap_ref(osis_ref: str, target: str) -> str:
    """Remap a (possibly ranged) OSIS reference from KJV to ``target``.

    Returns the input **unchanged** when ``target`` has no mapping, the
    reference can't be parsed, or no versification rule applies — so an
    untouched reference is byte-for-byte identical to its input (including any
    ``a``/``b`` sub-verse suffix). Only when a rule actually shifts the verse is
    a new reference returned, with the original sub-verse suffix re-appended.
    """
    if not osis_ref or target not in _TARGET_RULES:
        return osis_ref

    def _one(part: str) -> str:
        core, suffix = _split_suffix(part)
        t = _parse(core)
        if not t:
            return part  # unparseable — keep verbatim
        b, c, v = remap_verse(*t, target)
        if (b, c, v) == t:
            return part  # no rule applied — preserve exactly (suffix and all)
        return f"{b}.{c}.{v}{suffix}"

    if "-" not in osis_ref:
        return _one(osis_ref)

    start, end = osis_ref.split("-", 1)
    return f"{_one(start)}-{_one(end)}"


def is_misaligned(osis_ref: str, target: str) -> bool:
    """True only when a versification rule actually shifts ``osis_ref``.

    A reference that merely carries a sub-verse suffix (``Eph.5.33.a``) but is
    not in any difference zone is **not** misaligned — the suffix is preserved.
    """
    return remap_ref(osis_ref, target) != osis_ref


__all__ = ["has_mapping", "is_misaligned", "remap_ref", "remap_verse"]

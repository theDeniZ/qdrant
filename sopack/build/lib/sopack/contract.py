"""Single source of truth for the corpus import pipeline.

Imported by **both** the `sopack` CLI on the Mac and the import service on the
server, so client and server cannot drift — which is the whole family of
failure #15 in ``docs/IMPORT-PIPELINE.md``.

**Stdlib only.** The server must never pull fastembed, onnxruntime or numpy in
through this module; only ``sopack.pack`` may import an embedding library.

Every constant below was *measured* against the live collections on 2026-09-22,
not assumed — see ``docs/IMPORT-PIPELINE-PLAN.md`` §2:

* fastembed 0.8.0 (mean pooling) reproduces stored vectors at cosine 1.00000 on
  ``sop`` (pioneers, EGW/en, EGW/de) and on ``bibles``. There is no split
  vector space, and the deployed query path is not mismatched.
* The id rules in ``ID_RULES`` each reproduced 8/8 live point ids.
"""

from __future__ import annotations

from uuid import NAMESPACE_DNS, uuid5

SCHEMA_PACK = "sopack/1"
SCHEMA_BOOK = "sopack.book/1"

# ── the embedding contract ───────────────────────────────────────────────────
# Changing any of this invalidates every vector already written. It is asserted
# on every pack, and the canary probe (§4.4) proves it empirically per import.
VECTOR_NAME = "fast-multilingual-e5-large"
VECTOR_SIZE = 1024
DISTANCE = "Cosine"

EMBEDDING = {
    "model": "intfloat/multilingual-e5-large",
    "library": "fastembed",
    "library_version": "0.8.0",
    "pooling": "mean",          # fastembed >=0.8.0; 0.5.1 and earlier used CLS
    "normalized": True,
    "passage_prefix": "passage: ",
}
QUERY_PREFIX = "query: "        # the read side; here so both sides agree

# Minimum cosine between a pack's probe vector and the collection's stored
# vector for the same point. Below this the import aborts before writing.
PROBE_MIN_COSINE = 0.95
# A healthy pack scores ~1.0; anything under this is reported as suspicious
# even though it passes.
PROBE_EXPECT_COSINE = 0.99


# ── point id rules ───────────────────────────────────────────────────────────
# A book's rule is fixed at its first import and recorded in its book.json.
# Changing it orphans every point already written, so `pack` refuses to change
# the rule of a book that already carries one.

def _id(uid: str) -> str:
    return str(uuid5(NAMESPACE_DNS, uid))


def uid_sop_plain(f: dict) -> str:
    """EGW points, de and en. Verified 8/8 against live `sop`."""
    return f"{f['lang']}:{f['book_code']}:{f['para_key']}"


def uid_sop_seq(f: dict) -> str:
    """Pioneer points. The `#<seq>` suffix is ALWAYS present, including on
    unsplit blocks (seq 0) — verified 8/8 against live `sop`. It is what makes
    a chunked block's several points distinct, since they share a para_key."""
    return f"{f['lang']}:{f['book_code']}:{f['para_key']}#{f.get('seq', 0)}"


def uid_bible(f: dict) -> str:
    """Verse points. Verified 8/8 against live `bibles`."""
    return f"bible:{f['bible']}:{f['osis']}"


ID_RULES = {
    "sop/plain": uid_sop_plain,
    "sop/seq": uid_sop_seq,
    "bible/v1": uid_bible,
}

ID_RULE_DOC = {
    "sop/plain": "uuid5(dns, '<lang>:<book_code>:<para_key>')",
    "sop/seq": "uuid5(dns, '<lang>:<book_code>:<para_key>#<seq>')",
    "bible/v1": "uuid5(dns, 'bible:<bible>:<osis>')",
}


def point_id(rule: str, fields: dict) -> str:
    """Deterministic point id for *fields* under the named *rule*."""
    try:
        make_uid = ID_RULES[rule]
    except KeyError:
        raise ValueError(
            f"unknown id_rule {rule!r} (have: {', '.join(sorted(ID_RULES))})") from None
    return _id(make_uid(fields))


def uid_for(rule: str, fields: dict) -> str:
    """The pre-hash uid string — stored in points.jsonl so the server can
    recompute the id and detect a mangled pack."""
    return ID_RULES[rule](fields)


# ── profiles ─────────────────────────────────────────────────────────────────
# A profile fixes the payload schema, the id rule, the payload indexes and the
# post-import verification. Everything else in the pipeline is profile-agnostic.

class Profile:
    def __init__(self, name, collection, required, optional, indexes,
                 id_rules, default_id_rule, identity, text_field,
                 reimport_is_normal):
        self.name = name
        self.collection = collection
        self.required = tuple(required)
        self.optional = tuple(optional)
        self.indexes = dict(indexes)
        self.id_rules = tuple(id_rules)
        self.default_id_rule = default_id_rule
        self.identity = identity          # payload key naming the unit of import
        self.text_field = text_field      # which payload key holds the text
        self.reimport_is_normal = reimport_is_normal


PROFILES = {
    "sop": Profile(
        name="sop",
        collection="sop",
        # Exactly what translator/sop_tools_mcp.py and app/sop_tools.py read.
        required=("lang", "book_code", "book_pair", "page", "para",
                  "para_key", "raw_text", "aligned"),
        # Additive; existing readers ignore what they do not know. `corpus` is
        # the escape hatch that keeps non-EGW works out of EGW-only searches —
        # EGW points have NO `corpus` key, so must_not works with no backfill.
        optional=("corpus", "author", "title", "year", "slug", "page_kind",
                  "chunk", "chunks", "bible_refs"),
        indexes={"lang": "keyword", "book_code": "keyword", "page": "integer"},
        id_rules=("sop/seq", "sop/plain"),
        default_id_rule="sop/seq",
        identity="book_code",
        text_field="raw_text",
        # A book_code already in use by a DIFFERENT slug is a collision, not a
        # re-import. v1 refuses overwrites unless the job asks for them.
        reimport_is_normal=False,
    ),
    "bible": Profile(
        name="bible",
        collection="bibles",
        required=("bible", "osis", "text"),
        optional=("canonical_osis", "versification_offset"),
        indexes={"bible": "keyword", "osis": "keyword"},
        id_rules=("bible/v1",),
        default_id_rule="bible/v1",
        identity="bible",
        text_field="text",
        # The unit is a whole translation, and re-importing a corrected edition
        # is the normal case rather than the exception.
        reimport_is_normal=True,
    ),
}


def get_profile(name: str) -> Profile:
    try:
        return PROFILES[name]
    except KeyError:
        raise ValueError(
            f"unknown profile {name!r} (have: {', '.join(sorted(PROFILES))})") from None


# ── validation helpers ───────────────────────────────────────────────────────

def validate_payload(profile: Profile, payload: dict) -> list[str]:
    """Problems with one point's payload. Empty list means it is well formed.

    Required keys must be *present* (``aligned`` and ``book_pair`` are
    legitimately ``None``), and no unknown key may appear — an unknown key is
    usually a typo that would be silently written and never read.
    """
    errors = []
    for key in profile.required:
        if key not in payload:
            errors.append(f"missing required payload key {key!r}")
    known = set(profile.required) | set(profile.optional)
    for key in payload:
        if key not in known:
            errors.append(f"unknown payload key {key!r}")
    text = payload.get(profile.text_field)
    if not isinstance(text, str) or not text.strip():
        errors.append(f"{profile.text_field!r} is empty")
    return errors


def check_embedding(declared: dict) -> list[str]:
    """Differences between a pack's declared embedding block and this contract.

    Any difference is fatal: it means the pack's vectors live in a different
    geometric space than the collection, which no amount of retrying fixes.
    """
    errors = []
    for key, want in EMBEDDING.items():
        got = declared.get(key)
        if got != want:
            errors.append(f"embedding.{key}: pack says {got!r}, contract requires {want!r}")
    return errors


def check_target(declared: dict, profile: Profile) -> list[str]:
    """Differences between a pack's declared target and this contract."""
    errors = []
    expected = {
        "collection": profile.collection,
        "vector_name": VECTOR_NAME,
        "vector_size": VECTOR_SIZE,
        "distance": DISTANCE,
    }
    for key, want in expected.items():
        got = declared.get(key)
        if got != want:
            errors.append(f"target.{key}: pack says {got!r}, expected {want!r}")
    return errors


def cosine(a, b) -> float:
    """Plain-Python cosine. Used on a handful of probe vectors per import, so
    the server needs no numpy for it."""
    num = sum(x * y for x, y in zip(a, b))
    na = sum(x * x for x in a) ** 0.5
    nb = sum(y * y for y in b) ** 0.5
    return num / (na * nb) if na and nb else 0.0

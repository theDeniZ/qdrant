"""Single source of truth for the corpus import pipeline — the **neutral**
half (SOPACK-AUTONOMY.md §3.2): everything true for any backend. Imported by
**both** the ``sopack`` CLI and the server's import service, so client and
server cannot drift.

**Stdlib only.** The server must never pull fastembed, onnxruntime or numpy in
through this module; only ``sopack.pack`` may import an embedding library.

What is deliberately **not** here any more: collection names, Qdrant
named-vector names, Qdrant payload-index types, and the Qdrant spelling of
"Cosine". Those describe one backend's storage, not the vector space a pack
was built in, and now live in the importer's adapter config
(``app/store_adapter.py``). See ``docs/SOPACK-AUTONOMY.md`` §3.2 and
``docs/SOPACK-2-FORMAT.md`` (normative).

**The contract is data**, not code: everything below is *derived* from
``contracts/<id>/contract.toml`` (loaded with ``tomllib``), which is shared
byte-for-byte with the Rust ``sopack`` binary (``docs/SOPACK-1.0-PLAN.md``
§3.6). Module-level constants (``EMBEDDING``, ``PROFILES``, ``ID_RULES``, …)
are the *default* contract (``e5-large-v1``), loaded once at import time so
every existing call site (``pack.py``, ``format.py``, ``doctor.py``,
``cli.py``, the server's ``import_service.py``) keeps working unchanged.
Call :func:`load_contract` directly to load a different one.
"""

from __future__ import annotations

import hashlib
import json
import os
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from uuid import NAMESPACE_DNS, uuid5

SCHEMA_PACK = "sopack/2"        # what THIS writer produces
SCHEMA_PACK_V1 = "sopack/1"     # still accepted by the reader (transition)
SUPPORTED_PACK_SCHEMAS = (SCHEMA_PACK_V1, SCHEMA_PACK)
SCHEMA_BOOK = "sopack.book/1"   # unrelated to the embedding contract
SCHEMA_CALIBRATION = "sopack.calibration/1"

DEFAULT_CONTRACT_ID = "e5-large-v1"

# This Python reference implementation's own pinned embedding library. It is
# NOT part of the neutral contract (a Rust build has no fastembed at all) —
# acceptance of a pack's vectors is decided by the calibration probe, not by
# which library produced them (SOPACK-2-FORMAT.md §2). Kept here, once, so
# `sopack.pack` (the preflight assertion) and `sopack.doctor` (the
# environment check) agree without either one hard-coding its own copy.
PYTHON_FASTEMBED_VERSION = "0.8.0"

__all__ = [
    "SCHEMA_PACK", "SCHEMA_PACK_V1", "SUPPORTED_PACK_SCHEMAS", "SCHEMA_BOOK", "SCHEMA_CALIBRATION",
    "DEFAULT_CONTRACT_ID", "PYTHON_FASTEMBED_VERSION",
    "Contract", "Profile", "ContractError",
    "load_contract", "contracts_dir", "contract_dir",
    "EMBEDDING", "QUERY_PREFIX", "VECTOR_SIZE",
    "CONTRACT_ID", "CONTRACT_PATH", "CONTRACT_SHA256",
    "CALIBRATION_PATH", "CALIBRATION_SHA256",
    "PACK_MIN_COSINE", "PROBE_MIN_COSINE", "PROBE_EXPECT_COSINE",
    "ID_NAMESPACE", "ID_RULES", "ID_RULE_DOC", "PROFILES",
    "get_profile", "validate_payload", "check_embedding", "check_target",
    "point_id", "uid_for", "cosine", "load_calibration", "CalibrationError",
    "calibration_fixture_sha256",
]


class ContractError(Exception):
    """``contract.toml`` (or its calibration fixture) is missing or malformed."""


class CalibrationError(Exception):
    """The calibration fixture is missing, malformed, or does not match the
    sha256 the contract declares for it."""


# ── locating contracts/ ──────────────────────────────────────────────────────
#
# Single source of truth: qdrant/sopack-rs/contracts/. The Rust binary embeds
# these files (include_str!) and the Python server reads them straight off
# disk with tomllib — no packaged, possibly-stale copy inside `sopack/`
# itself. `sopack/` and `sopack-rs/` are siblings under `qdrant/`, and the
# Docker image (qdrant/Dockerfile) COPYs `sopack-rs/contracts` alongside
# `sopack` and `app` so the container reproduces the same relative layout.
# SOPACK_CONTRACTS_DIR overrides this for tests or an unusual deployment.

def contracts_dir() -> Path:
    override = os.environ.get("SOPACK_CONTRACTS_DIR")
    if override:
        return Path(override)
    return Path(__file__).resolve().parent.parent / "sopack-rs" / "contracts"


def contract_dir(contract_id: str = DEFAULT_CONTRACT_ID) -> Path:
    return contracts_dir() / contract_id


# ── profiles ─────────────────────────────────────────────────────────────────

class Profile:
    """A profile fixes the payload schema, the id rule and post-import
    verification. It carries **hints** about which payload fields a store
    might want to index (``filterable``) — never a concrete backend index
    type; that mapping is the adapter's job (SOPACK-AUTONOMY.md §3.2)."""

    def __init__(self, name, required, optional, id_rules, default_id_rule,
                 identity, text_field, reimport_is_normal, filterable):
        self.name = name
        self.required = tuple(required)
        self.optional = tuple(optional)
        self.id_rules = tuple(id_rules)
        self.default_id_rule = default_id_rule
        self.identity = identity          # payload key naming the unit of import
        self.text_field = text_field      # which payload key holds the text
        self.reimport_is_normal = reimport_is_normal
        self.filterable = dict(filterable)  # {payload_key: "string"|"int"|...}


@dataclass
class Contract:
    """One loaded ``contract.toml`` (+ its calibration fixture's sha256)."""

    id: str
    path: Path
    raw: bytes
    sha256: str
    embedding: dict = field(default_factory=dict)
    query_prefix: str = ""
    calibration_file: str = "calibration.json"
    calibration_sha256: str = ""
    pack_min_cosine: float = 0.9999
    probe_min_cosine: float = 0.95
    probe_expect_cosine: float = 0.99
    id_rule_templates: dict = field(default_factory=dict)
    profiles: dict = field(default_factory=dict)

    @property
    def dim(self) -> int:
        return int(self.embedding["dim"])

    @property
    def calibration_path(self) -> Path:
        return self.path.parent / self.calibration_file

    def id_rule_doc(self, rule: str) -> str:
        template = self.id_rule_templates[rule]
        return "uuid5(dns, '" + template.replace("{", "<").replace("}", ">") + "')"

    def get_profile(self, name: str) -> Profile:
        try:
            return self.profiles[name]
        except KeyError:
            raise ValueError(
                f"unknown profile {name!r} (have: {', '.join(sorted(self.profiles))})"
            ) from None


def _build_profile(name: str, data: dict) -> Profile:
    return Profile(
        name=name,
        required=data["required"],
        optional=data["optional"],
        id_rules=data["id_rules"],
        default_id_rule=data["default_id_rule"],
        identity=data["identity"],
        text_field=data["text_field"],
        reimport_is_normal=bool(data["reimport_is_normal"]),
        filterable=data.get("filterable") or {},
    )


def load_contract(contract_id: str = DEFAULT_CONTRACT_ID) -> Contract:
    """Load and validate ``contracts/<contract_id>/contract.toml``."""
    path = contract_dir(contract_id) / "contract.toml"
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise ContractError(f"cannot read {path}: {exc}") from exc
    try:
        data = tomllib.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
        raise ContractError(f"{path}: not valid TOML: {exc}") from exc

    try:
        embedding = dict(data["embedding"])
        calibration = data["calibration"]
        ids = data["ids"]
        profiles_data = data["profiles"]
    except KeyError as exc:
        raise ContractError(f"{path}: missing required section {exc}") from exc

    profiles = {name: _build_profile(name, pdata) for name, pdata in profiles_data.items()}

    return Contract(
        id=str(data.get("id", contract_id)),
        path=path,
        raw=raw,
        sha256=hashlib.sha256(raw).hexdigest(),
        embedding=embedding,
        query_prefix=str(embedding.get("query_prefix", "")),
        calibration_file=str(calibration.get("file", "calibration.json")),
        calibration_sha256=str(calibration.get("sha256", "")),
        pack_min_cosine=float(calibration.get("pack_min_cosine", 0.9999)),
        probe_min_cosine=float(calibration.get("probe_min_cosine", 0.95)),
        probe_expect_cosine=float(calibration.get("probe_expect_cosine", 0.99)),
        id_rule_templates=dict(ids.get("rules", {})),
        profiles=profiles,
    )


def load_calibration(c: "Contract | None" = None) -> dict:
    """Load and verify ``calibration.json`` next to *c*'s ``contract.toml``
    (default: the module's default contract). Raises :class:`CalibrationError`
    if the file is missing, malformed, or its sha256 does not match
    ``contract.toml``'s ``[calibration].sha256`` — a pack must never be built
    (or checked) against a fixture the contract does not vouch for."""
    c = c or _DEFAULT
    path = c.calibration_path
    try:
        raw = path.read_bytes()
    except OSError as exc:
        raise CalibrationError(f"cannot read calibration fixture {path}: {exc}") from exc
    got_sha = hashlib.sha256(raw).hexdigest()
    if c.calibration_sha256 and got_sha != c.calibration_sha256:
        raise CalibrationError(
            f"{path}: sha256 {got_sha[:12]}… does not match contract.toml's "
            f"[calibration].sha256 {c.calibration_sha256[:12]}… — the fixture "
            f"on disk does not match the one this contract was calibrated "
            f"against")
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise CalibrationError(f"{path}: not valid JSON: {exc}") from exc
    if data.get("schema") != SCHEMA_CALIBRATION:
        raise CalibrationError(
            f"{path}: unsupported schema {data.get('schema')!r} "
            f"(expected {SCHEMA_CALIBRATION!r})")
    entries = data.get("entries")
    if not isinstance(entries, list) or not entries:
        raise CalibrationError(f"{path}: 'entries' must be a non-empty list")
    return data


# ── the default (module-level) contract ──────────────────────────────────────
#
# Loaded once, at import time, exactly like the constants it replaces — every
# existing call site keeps working. A caller that needs a *different*
# contract calls load_contract() directly (M2+; sopack ships one contract for
# 1.0).

_DEFAULT = load_contract(DEFAULT_CONTRACT_ID)

CONTRACT_ID = _DEFAULT.id
CONTRACT_PATH = _DEFAULT.path
CONTRACT_SHA256 = _DEFAULT.sha256

EMBEDDING = dict(_DEFAULT.embedding)
QUERY_PREFIX = _DEFAULT.query_prefix
VECTOR_SIZE = _DEFAULT.dim

CALIBRATION_PATH = _DEFAULT.calibration_path
CALIBRATION_SHA256 = _DEFAULT.calibration_sha256
PACK_MIN_COSINE = _DEFAULT.pack_min_cosine
PROBE_MIN_COSINE = _DEFAULT.probe_min_cosine
PROBE_EXPECT_COSINE = _DEFAULT.probe_expect_cosine

ID_NAMESPACE = NAMESPACE_DNS
PROFILES = _DEFAULT.profiles


def _fmt_uid(template: str, fields: dict) -> str:
    try:
        return template.format(**fields)
    except KeyError as exc:
        raise ValueError(f"id_rule template {template!r} needs field {exc}") from None


ID_RULES = {
    rule: (lambda fields, _t=template: _fmt_uid(_t, fields))
    for rule, template in _DEFAULT.id_rule_templates.items()
}
ID_RULE_DOC = {rule: _DEFAULT.id_rule_doc(rule) for rule in _DEFAULT.id_rule_templates}


def get_profile(name: str) -> Profile:
    return _DEFAULT.get_profile(name)


# ── id rules ─────────────────────────────────────────────────────────────────

def _id(uid: str) -> str:
    return str(uuid5(ID_NAMESPACE, uid))


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


# The exact keys SOPACK-2-FORMAT.md §2 says the importer checks. `runtime`,
# `device`, `threads`, `batch_tokens` are provenance (R10) and are never
# compared. There is no `library`/`library_version` check any more —
# acceptance is decided by the calibration probe, not by which library
# produced the vectors.
_EMBED_CHECK_KEYS_V2 = ("model", "pooling", "normalized", "dim", "distance",
                        "max_tokens", "passage_prefix")
# sopack/1 packs never carried `dim`, `distance` or `max_tokens` inside their
# embedding block (dim lived in counts.dim; distance was a Qdrant-only
# target field; max_tokens did not exist as a tracked concept). Checking
# those keys against a /1 manifest would always spuriously fail, so the
# legacy check compares only what /1 packs actually declared.
_EMBED_CHECK_KEYS_V1 = ("model", "pooling", "normalized", "passage_prefix")


def check_embedding(declared: dict, schema: str = SCHEMA_PACK) -> list[str]:
    """Differences between a pack's declared embedding block and this
    contract. Any difference is fatal: it means the pack's vectors live in a
    different geometric space than the collection, which no amount of
    retrying fixes."""
    keys = _EMBED_CHECK_KEYS_V1 if schema == SCHEMA_PACK_V1 else _EMBED_CHECK_KEYS_V2
    errors = []
    for key in keys:
        want = EMBEDDING.get(key)
        got = declared.get(key)
        if got != want:
            errors.append(f"embedding.{key}: pack says {got!r}, contract requires {want!r}")
    return errors


def check_target(declared: dict, profile: Profile, schema: str = SCHEMA_PACK) -> list[str]:
    """Differences between a pack's declared ``target`` block and this
    contract. For ``sopack/2`` the target is store-neutral
    (``{"profile", "contract"}``). A legacy ``sopack/1`` pack's target named a
    backend-specific collection/vector space too — this module intentionally
    no longer knows what those values should be (moving that knowledge out is
    the point of M1), so a /1 target gets no check here at all; the importer,
    which DOES know the backend, independently verifies a /1 pack's declared
    backend fields against its own adapter config (``app/store_adapter.py``)."""
    if schema == SCHEMA_PACK_V1:
        return []
    errors = []
    if declared.get("profile") != profile.name:
        errors.append(f"target.profile: pack says {declared.get('profile')!r}, "
                      f"expected {profile.name!r}")
    if declared.get("contract") != CONTRACT_ID:
        errors.append(f"target.contract: pack says {declared.get('contract')!r}, "
                      f"expected {CONTRACT_ID!r}")
    return errors


def calibration_fixture_sha256(fixture_doc: dict) -> str:
    """sha256 of a calibration fixture doc's canonical JSON bytes.

    The committed fixture's sha is simply the sha256 of its file
    (``CALIBRATION_SHA256``, verified by :func:`load_calibration`). A caller
    that overrides the fixture (tests; never production, where ``pack()``'s
    ``calibration`` parameter is left at its default) has no file to hash, so
    both sides of a probe — the writer (``sopack.pack``) and a reader
    checking against that SAME override (``run_calibration_probe``'s
    ``fixture=`` parameter) — hash the override's canonical JSON instead, so
    they still agree on what "this fixture" is."""
    blob = json.dumps(fixture_doc, ensure_ascii=False, sort_keys=True).encode("utf-8")
    return hashlib.sha256(blob).hexdigest()


def cosine(a, b) -> float:
    """Plain-Python cosine. Used on a handful of probe vectors per import, so
    the server needs no numpy for it."""
    num = sum(x * y for x, y in zip(a, b))
    na = sum(x * x for x in a) ** 0.5
    nb = sum(y * y for y in b) ** 0.5
    return num / (na * nb) if na and nb else 0.0

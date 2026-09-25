"""The corpus import stage machine (docs/IMPORT-PIPELINE-PLAN.md §6).

Stages, exactly in this order::

    open · contract · probe · preflight · snapshot · undo · upsert ·
    indexes · titles · verify · report

Each stage asserts its own output and refuses to hand a half-result to the
next (R8). Public entry points for the admin UI::

    start_job(pack_id, mode, allow_overwrite, operator) -> job_id
    get_job(job_id) -> dict
    list_jobs() -> list[dict]
    read_log(job_id, after) -> {"events": [...], "next": int}
    cancel(job_id)
    resume(job_id)
    rollback(job_id)
    restore_snapshot(job_id)

Jobs run in a worker thread under the one global import lock
(``jobs.try_acquire_import_lock``). Every stage talks to the backend only
through ``ctx.adapter`` (``app/store_adapter.py``, M1: SOPACK-AUTONOMY.md
§3.3) — this module itself makes no HTTP calls and no longer imports
``requests``. Stdlib only, and must never import fastembed, onnxruntime,
numpy or qdrant_client (rule #5 of the brief); only ``sopack.pack`` on the
Mac may.
"""

from __future__ import annotations

import copy
import json
import os
import sys
import threading
import time
from pathlib import Path

from sopack import contract
from sopack.format import PackError, PackReader

from . import jobs, snapshots, store_adapter

_TIMEOUT_S = float(os.environ.get("IMPORT_TIMEOUT_S", "60"))
_UPSERT_TIMEOUT_S = float(os.environ.get("IMPORT_UPSERT_TIMEOUT_S", "120"))
_MAX_RETRIES = 5
_BACKOFF_CAP_S = 60.0
_PREFLIGHT_BATCH = 512
_UPSERT_BATCH = 128
_UNDO_BATCH = 128

# Stages that always re-run, even on resume — they are read-only against
# Qdrant and they rebuild transient state (`_Ctx.reader`, live vector config)
# that a fresh worker thread has no other way to recover after a process
# restart.
#
# `preflight` is deliberately NOT here: once it has reached 'ok' it has
# already written overwrite_ids.txt, which is what `allow_overwrite` was
# checked against. Re-deriving it on resume would compare the live
# collection against itself *after* this job's own (possibly-interrupted)
# upsert had already written some of those points, making a job's own prior
# writes look like a foreign overwrite and wrongly refuse a resume that
# should just continue (R5: "re-running a completed job is a no-op").
_ALWAYS_RERUN = {"open", "contract", "probe"}


def _qdrant_url() -> str:
    return os.environ.get("QDRANT_URL", "http://localhost:6333").rstrip("/")


def _packs_dir() -> Path:
    return Path(os.environ.get("PACKS_DIR", "/data/packs"))


class StageFailure(Exception):
    """A stage refuses to hand a half-result to the next one. Fails the job."""

    def __init__(self, stage: str, msg: str):
        super().__init__(msg)
        self.stage = stage


class Busy(Exception):
    """Another import job is already running (one global import lock)."""


class Cancelled(Exception):
    """The job was asked to cancel mid-stage."""


# ── job context (transient, per worker-thread run; never persisted) ─────────

def _make_adapter() -> store_adapter.QdrantAdapter:
    """The adapter every real job uses. A separate function (rather than
    inlining ``store_adapter.QdrantAdapter(_qdrant_url())`` at each call site)
    so a future multi-backend deployment has one place to choose an adapter
    from job/profile config; for M1 there is exactly one backend."""
    return store_adapter.QdrantAdapter(_qdrant_url(), timeout_s=_TIMEOUT_S,
                                       upsert_timeout_s=_UPSERT_TIMEOUT_S)


class _Ctx:
    def __init__(self, job_id: str, adapter: store_adapter.StoreAdapter | None = None):
        job = jobs.load(job_id)
        self.job_id = job_id
        self.pack_id = job["pack_id"]
        self.mode = job["mode"]
        self.collection = job["collection"]
        self.allow_overwrite = bool(job["allow_overwrite"])
        self.profile = contract.get_profile(job["profile"])
        self.pack_path = _packs_dir() / f"{self.pack_id}.sopack"
        self.target_collection = (self.collection if self.mode == "apply"
                                  else f"{self.collection}__dryrun")
        self.reader: PackReader | None = None
        self.merged_titles: dict | None = None
        self.adapter = adapter or _make_adapter()

    def close(self) -> None:
        if self.reader is not None:
            self.reader.close()


def _check_cancel(job_id: str) -> None:
    if jobs.load(job_id).get("status") == "cancelling":
        raise Cancelled()


# ── stages ───────────────────────────────────────────────────────────────────

def _stage_open(ctx: _Ctx, job_id: str) -> dict:
    if not ctx.pack_path.is_file():
        raise StageFailure("open", f"pack not found: {ctx.pack_path}")
    try:
        reader = PackReader(ctx.pack_path)
    except PackError as exc:
        raise StageFailure("open", str(exc)) from exc
    errors = reader.check()
    if errors:
        reader.close()
        raise StageFailure("open", "; ".join(errors))
    ctx.reader = reader
    return {"detail": f"{reader.count} point(s), profile {reader.profile.name}"}


def _stage_contract(ctx: _Ctx, job_id: str) -> dict:
    try:
        ctx.adapter.ensure_collection(ctx.profile.name, contract.VECTOR_SIZE)
    except store_adapter.AdapterError as exc:
        raise StageFailure("contract", str(exc)) from exc
    return {"detail": f"live collection {ctx.collection!r} matches "
                      f"{store_adapter.QDRANT_VECTOR_NAME}/{contract.VECTOR_SIZE}/"
                      f"{store_adapter.QDRANT_DISTANCE}"}


def run_calibration_probe(reader: PackReader, adapter: store_adapter.StoreAdapter,
                          profile, collection: str, *, fixture: dict | None = None) -> str:
    """The ``sopack/2`` probe (SOPACK-2-FORMAT.md §4, steps 1-3), model-free.

    Store-neutral by construction — everything it touches goes through
    *adapter*, so this same function is what ``_stage_probe_v2`` runs against
    a live Qdrant AND what the round-trip acceptance test
    (app/tests/test_store_adapter.py) runs against an ``InMemoryAdapter``,
    proving the same ``.sopack`` verifies the same way against a second
    backend (SOPACK-AUTONOMY.md §5.5).

    *fixture* defaults to ``None``, which loads this importer's own committed
    calibration fixture (production, always). Tests pass the SAME fixture
    doc a test pack was calibrated against (``sopack.pack.pack``'s
    ``calibration=`` override), so the two sides agree on what "this
    fixture" hashes to without a real committed file
    (``contract.calibration_fixture_sha256``).

    Raises :class:`StageFailure` (stage ``"probe"``) on any violation;
    returns a human-readable detail string on success.
    """
    manifest_probe = reader.manifest.get("probe") or {}
    entries = manifest_probe.get("entries") or []
    if not entries:
        raise StageFailure("probe", "pack carries no calibration probe — refusing (not "
                                    "skippable, no flag)")
    pack_vectors = reader.probe_vectors()
    if len(pack_vectors) != len(entries):
        raise StageFailure("probe", f"probe vector count ({len(pack_vectors)}) != entry count "
                                    f"({len(entries)})")

    if fixture is None:
        try:
            fixture = contract.load_calibration()
        except contract.CalibrationError as exc:
            raise StageFailure("probe", f"this importer's calibration fixture is unusable: "
                                        f"{exc}") from exc
        importer_fixture_sha = contract.CALIBRATION_SHA256
    else:
        importer_fixture_sha = contract.calibration_fixture_sha256(fixture)

    # Step 1: pack <-> fixture.
    fixture_sha = manifest_probe.get("fixture_sha256")
    if fixture_sha != importer_fixture_sha:
        raise StageFailure(
            "probe", f"pack was calibrated against a different fixture "
                     f"(fixture_sha256 {fixture_sha!r} != this importer's "
                     f"{importer_fixture_sha!r})")
    fixture_by_id = {e["id"]: e for e in fixture["entries"]}
    problems = []
    worst1 = 1.0
    for i, e in enumerate(entries):
        fx = fixture_by_id.get(e.get("id"))
        if fx is None:
            problems.append(f"fixture entry {e.get('id')} not found in this importer's "
                            "calibration.json")
            continue
        cos = contract.cosine(pack_vectors[i], fx["vector"])
        worst1 = min(worst1, cos)
        if cos < contract.PROBE_MIN_COSINE:
            problems.append(f"pack<->fixture {e.get('id')}: cosine {cos:.5f} < "
                            f"{contract.PROBE_MIN_COSINE}")
    if problems:
        raise StageFailure("probe", "; ".join(problems))

    # Step 2: fixture <-> store (only the entries relevant to THIS profile).
    relevant = [e for e in fixture["entries"] if e.get("profile") == profile.name]
    ids = [e["id"] for e in relevant]
    stored_by_id = {}
    if ids:
        for row in adapter.retrieve(collection, ids, with_payload=False, with_vector=True):
            if row.get("vector"):
                stored_by_id[row["id"]] = row["vector"]
    present = [e for e in relevant if e["id"] in stored_by_id]

    if not present:
        # Step 3: empty store (or none of the fixture's ids are in it yet) —
        # the fixture DEFINES the store's space; record the contract
        # fingerprint on first import, and refuse a later import under a
        # different one.
        existing_fp = adapter.get_fingerprint(collection)
        if existing_fp is None:
            adapter.set_fingerprint(collection, contract.CONTRACT_SHA256)
            fp_detail = (f"empty store: recorded contract fingerprint "
                        f"{contract.CONTRACT_SHA256[:12]}…")
        elif existing_fp != contract.CONTRACT_SHA256:
            raise StageFailure(
                "probe", f"store {collection!r} was previously imported under a different "
                         f"contract (recorded fingerprint {existing_fp[:12]}… != this pack's "
                         f"{contract.CONTRACT_SHA256[:12]}…) — refusing")
        else:
            fp_detail = (f"empty store: contract fingerprint {contract.CONTRACT_SHA256[:12]}… "
                        "already recorded, matches")
    else:
        step2_problems = []
        worst2 = 1.0
        for e in present:
            cos = contract.cosine(stored_by_id[e["id"]], e["vector"])
            worst2 = min(worst2, cos)
            if cos < contract.PROBE_MIN_COSINE:
                step2_problems.append(f"fixture<->store {e['id']}: cosine {cos:.5f} < "
                                      f"{contract.PROBE_MIN_COSINE}")
        if step2_problems:
            raise StageFailure("probe", "; ".join(step2_problems))
        fp_detail = f"fixture<->store: {len(present)} fixture point(s), worst cosine {worst2:.5f}"

    return (f"{len(entries)} calibration entries: pack<->fixture worst cosine {worst1:.5f}; "
           f"{fp_detail}")


def _stage_probe(ctx: _Ctx, job_id: str) -> dict:
    """Schema-aware (SOPACK-2-FORMAT.md §4): a ``sopack/1`` pack keeps the
    live-canary probe (unchanged since before M1); a ``sopack/2`` pack runs
    the three-step calibration probe instead."""
    schema = ctx.reader.manifest.get("schema")
    if schema == contract.SCHEMA_PACK_V1:
        return _stage_probe_v1(ctx, job_id)
    return _stage_probe_v2(ctx, job_id)


def _stage_probe_v1(ctx: _Ctx, job_id: str) -> dict:
    probe = ctx.reader.manifest.get("probe") or {}
    canaries = probe.get("canaries") or []
    if not canaries:
        raise StageFailure("probe", "pack carries no canary probe — refusing (not skippable, "
                                    "no flag)")
    vectors = ctx.reader.probe_vectors()
    if len(vectors) != len(canaries):
        raise StageFailure("probe", f"probe vector count ({len(vectors)}) != canary count "
                                    f"({len(canaries)})")

    by_collection: dict[str, list[int]] = {}
    for i, c in enumerate(canaries):
        by_collection.setdefault(c.get("collection") or ctx.collection, []).append(i)

    found: dict[str, dict] = {}
    for coll, idxs in by_collection.items():
        ids = [canaries[i]["id"] for i in idxs]
        for row in ctx.adapter.retrieve(coll, ids, with_payload=False, with_vector=True):
            found[row["id"]] = row

    worst = 1.0
    problems = []
    for i, c in enumerate(canaries):
        pid = str(c["id"])
        point = found.get(pid)
        if point is None:
            problems.append(f"canary {pid} missing from {c.get('collection') or ctx.collection}")
            continue
        stored = point.get("vector")
        if not stored:
            problems.append(f"canary {pid} carries no vector")
            continue
        cos = contract.cosine(vectors[i], stored)
        worst = min(worst, cos)
        min_required = max(contract.PROBE_MIN_COSINE, float(c.get("cosine_expected_min") or 0))
        if cos < min_required:
            problems.append(f"canary {pid}: cosine {cos:.5f} < {min_required}")
    if problems:
        raise StageFailure("probe", "; ".join(problems))
    return {"detail": f"{len(canaries)} canaries, worst cosine {worst:.5f}"}


def _stage_probe_v2(ctx: _Ctx, job_id: str) -> dict:
    detail = run_calibration_probe(ctx.reader, ctx.adapter, ctx.profile, ctx.collection)
    return {"detail": detail}


def _identity_probe(adapter: store_adapter.StoreAdapter, collection: str, profile, code,
                    lang) -> tuple[bool, str | None]:
    """Does (code[, lang]) already exist? For the sop profile also returns a
    sample existing point's ``slug``, which is what tells a re-index apart
    from a genuine collision (failure #10): same slug -> re-index, different
    (non-empty) slug -> a different work claiming a taken code.

    Existence + sample come from one ``scroll(limit=1)`` call rather than a
    ``facet`` + a second ``scroll`` (the two calls the pre-adapter code made)
    — a store-neutral adapter need not expose Qdrant's faceting endpoint for
    this, and one call is strictly less work for the same answer."""
    must = [{"key": profile.identity, "match": {"value": code}}]
    if lang and profile.name == "sop":
        must.append({"key": "lang", "match": {"value": lang}})
    rows = adapter.scroll(collection, {"must": must}, 1, with_payload=True)
    if not rows:
        return False, None
    if profile.name != "sop":
        return True, None
    slug = rows[0].get("payload", {}).get("slug")
    return True, slug


def _retrieve_existing_ids(adapter: store_adapter.StoreAdapter, collection: str,
                           ids: list[str]) -> set[str]:
    rows = adapter.retrieve(collection, ids, with_payload=False, with_vector=False)
    return {row["id"] for row in rows}


def _stage_preflight(ctx: _Ctx, job_id: str) -> dict:
    profile = ctx.profile
    books = ctx.reader.manifest.get("books") or []
    collisions, reindexed, new_books = [], [], []

    for b in books:
        code = b.get(profile.identity)
        lang = b.get("lang")
        exists, existing_slug = _identity_probe(ctx.adapter, ctx.collection, profile, code, lang)
        if not exists:
            new_books.append(f"{lang + ':' if lang else ''}{code}")
            continue
        if profile.name == "sop":
            incoming_slug = b.get("slug")
            if existing_slug and incoming_slug and existing_slug != incoming_slug:
                collisions.append(f"{lang}:{code} (existing slug {existing_slug!r} != "
                                  f"incoming {incoming_slug!r})")
            else:
                reindexed.append(f"{lang}:{code}")
        else:
            reindexed.append(str(code))

    if collisions:
        # A different work claiming a taken code is always a hard stop,
        # regardless of allow_overwrite (failure #10).
        raise StageFailure("preflight", f"{len(collisions)} book_code collision(s), refusing: "
                                        + "; ".join(collisions))

    overwrite_ids: list[str] = []
    new_count = 0
    for points, _vectors in ctx.reader.batches(size=_PREFLIGHT_BATCH):
        _check_cancel(job_id)
        ids = [p["id"] for p in points]
        existing = _retrieve_existing_ids(ctx.adapter, ctx.collection, ids)
        for pid in ids:
            if pid in existing:
                overwrite_ids.append(pid)
            else:
                new_count += 1

    if overwrite_ids and not ctx.allow_overwrite:
        raise StageFailure("preflight", f"{len(overwrite_ids)} point(s) already exist; "
                                        "refusing without allow_overwrite")

    d = jobs.job_dir(job_id)
    d.mkdir(parents=True, exist_ok=True)
    (d / "overwrite_ids.txt").write_text("\n".join(overwrite_ids), encoding="utf-8")

    jobs.update(job_id, counts={**jobs.load(job_id)["counts"], "books": len(books),
                                "overwritten": len(overwrite_ids)})
    return {"detail": f"{len(new_books)} new book(s), {len(reindexed)} re-index book(s), "
                      f"{new_count} new point(s), {len(overwrite_ids)} overwrite point(s)"}


def _overwrite_ids(job_id: str) -> list[str]:
    p = jobs.job_dir(job_id) / "overwrite_ids.txt"
    if not p.is_file():
        return []
    text = p.read_text(encoding="utf-8")
    return [line for line in text.splitlines() if line]


def _stage_snapshot(ctx: _Ctx, job_id: str) -> dict:
    if ctx.mode == "dry-run":
        ctx.adapter.create_scratch_collection(ctx.target_collection, contract.VECTOR_SIZE)
        return {"skipped": True,
                "detail": f"dry-run: scratch collection {ctx.target_collection} created "
                          "instead of a snapshot"}
    try:
        snap = snapshots.create(ctx.collection)
    except snapshots.SnapshotError as exc:
        # Nothing is written if this fails — that is the whole point.
        raise StageFailure("snapshot", str(exc)) from exc
    jobs.update(job_id, snapshot={"name": snap.get("name"), "size": snap.get("size")})
    return {"detail": snap.get("name")}


def _stage_undo(ctx: _Ctx, job_id: str) -> dict:
    if ctx.mode == "dry-run":
        return {"skipped": True,
                "detail": "dry-run: scratch collection starts empty, nothing to preserve"}
    overwrite_ids = _overwrite_ids(job_id)
    if not overwrite_ids:
        return {"skipped": True, "detail": "no overwrites"}

    path = jobs.job_dir(job_id) / "undo.jsonl"
    captured = 0
    with path.open("w", encoding="utf-8") as fh:
        for i in range(0, len(overwrite_ids), _UNDO_BATCH):
            _check_cancel(job_id)
            chunk = overwrite_ids[i:i + _UNDO_BATCH]
            result = ctx.adapter.retrieve(ctx.collection, chunk, with_payload=True, with_vector=True)
            for p in result:
                fh.write(json.dumps({"id": p["id"], "payload": p.get("payload") or {},
                                     "vector": p.get("vector")}, ensure_ascii=False) + "\n")
                captured += 1

    if captured != len(overwrite_ids):
        raise StageFailure("undo", f"captured prior state for {captured}/{len(overwrite_ids)} "
                                   "overwritten point(s) — refusing an incomplete undo ledger")
    return {"detail": f"captured {captured} point(s) for rollback"}


def _upsert_with_retry(adapter: store_adapter.StoreAdapter, collection: str,
                       points: list[dict], job_id: str) -> None:
    delay = 1.0
    last_exc: Exception | None = None
    for attempt in range(1, _MAX_RETRIES + 1):
        try:
            adapter.upsert(collection, points)
            return
        except store_adapter.AdapterError as exc:
            last_exc = exc
            if attempt == _MAX_RETRIES:
                break
            jobs.append_log(job_id, "upsert", "warn",
                            f"batch failed (attempt {attempt}/{_MAX_RETRIES}): {exc}; "
                            f"retrying in {delay:.0f}s")
            time.sleep(min(delay, _BACKOFF_CAP_S))
            delay = min(delay * 2, _BACKOFF_CAP_S)
    raise StageFailure("upsert", f"batch failed after {_MAX_RETRIES} attempts: {last_exc}")


def _stage_upsert(ctx: _Ctx, job_id: str) -> dict:
    overwrite_ids = set(_overwrite_ids(job_id))
    total = ctx.reader.count
    jobs.update(job_id, progress={"points_total": total, "points_written": 0, "batches": 0})

    written = 0
    batches_done = 0
    created_path = jobs.job_dir(job_id) / "created_ids.txt"
    with created_path.open("w", encoding="utf-8") as created_fh:
        for points, vectors in ctx.reader.batches(size=_UPSERT_BATCH):
            _check_cancel(job_id)
            body_points = [
                {"id": p["id"], "vector": v, "payload": p["payload"]}
                for p, v in zip(points, vectors)
            ]
            _upsert_with_retry(ctx.adapter, ctx.target_collection, body_points, job_id)
            for p in points:
                if p["id"] not in overwrite_ids:
                    created_fh.write(p["id"] + "\n")
            written += len(points)
            batches_done += 1
            jobs.update(job_id, progress={"points_total": total, "points_written": written,
                                          "batches": batches_done})
            jobs.append_log(job_id, "upsert", "info", f"batch {batches_done} ok",
                            data={"written": written})

    if written != total:
        raise StageFailure("upsert", f"wrote {written} point(s), pack declares {total}")

    created_n = sum(1 for _ in created_path.read_text(encoding="utf-8").splitlines()
                    if _.strip()) if created_path.exists() else 0
    jobs.update(job_id, counts={**jobs.load(job_id)["counts"], "created": created_n,
                                "overwritten": len(overwrite_ids)})
    return {"detail": f"{written} point(s) written to {ctx.target_collection}"}


def _stage_indexes(ctx: _Ctx, job_id: str) -> dict:
    created = []
    for field, schema in store_adapter.indexes_for(ctx.profile.name).items():
        try:
            ctx.adapter.ensure_index(ctx.target_collection, field, schema)
        except store_adapter.AdapterError as exc:
            raise StageFailure("indexes", f"could not ensure index {field!r}: {exc}") from exc
        created.append(field)
    return {"detail": f"ensured payload index(es): {', '.join(created)}"}


def _assert_no_shrink(existing: dict, merged: dict) -> None:
    """The shrink guard (R6 / failures #11-#13): refuse if any language table
    would come out of a merge smaller than it went in. Kept as its own
    function — independent of how ``merged`` was produced — so it is a real
    fail-safe against a future bug in the merge path, not just an assertion
    that repeats what the additive walk already guarantees by construction."""
    for lang, table in existing.items():
        if len(merged.get(lang, {})) < len(table):
            raise StageFailure("titles", f"merge would shrink {lang} from {len(table)} to "
                                         f"{len(merged.get(lang, {}))} entries — refusing")


def _merge_titles(existing: dict, fragment: dict) -> tuple[dict, list[str]]:
    """Additive merge of a pack's titles.json fragment into the existing
    table — the shape of scripts/merge_corpus_titles.py's default (non
    --enrich) mode: a code already known is left completely untouched, only
    new (lang, code) pairs are added. Never removes or shrinks anything (R6)."""
    merged = copy.deepcopy(existing)
    added = []
    for lang, books in (fragment or {}).items():
        table = merged.setdefault(lang, {})
        for code, entry in (books or {}).items():
            if code in table:
                continue
            table[code] = copy.deepcopy(entry)
            added.append(f"{lang}:{code}")

    _assert_no_shrink(existing, merged)
    return merged, added


def _sop_books_path() -> Path:
    return Path(os.environ.get("SOP_BOOKS_JSON", "/data/sop_books.json"))


def _load_titles_table() -> dict:
    dest = _sop_books_path()
    if dest.is_file():
        return json.loads(dest.read_text(encoding="utf-8"))
    return {}


def _clear_titles_cache() -> None:
    """Clear ``sop_tools``'s module-global title cache — but only if that
    module is already loaded. ``sop_tools`` unconditionally imports the MCP
    server framework at module scope; this service has no business dragging
    that in (or being broken by a version mismatch in it) just to invalidate
    a cache. In the real server process ``server.py`` has already imported
    ``sop_tools`` by the time an import job ever runs, so this reaches it."""
    mod = sys.modules.get("app.sop_tools") or sys.modules.get("sop_tools")
    if mod is not None:
        mod._titles = None


def _stage_titles(ctx: _Ctx, job_id: str) -> dict:
    if ctx.profile.name != "sop":
        return {"skipped": True, "detail": "the title table applies to the sop profile only"}
    fragment = ctx.reader.titles()
    if not fragment:
        return {"skipped": True, "detail": "pack carries no titles.json fragment"}

    existing = _load_titles_table()
    merged, added = _merge_titles(existing, fragment)
    ctx.merged_titles = merged
    plural = "y" if len(added) == 1 else "ies"

    if ctx.mode == "dry-run":
        return {"detail": f"{len(added)} new title entr{plural} would be added (dry-run, "
                          "not written)"}

    d = jobs.job_dir(job_id)
    d.mkdir(parents=True, exist_ok=True)
    (d / "sop_books.before.json").write_text(
        json.dumps(existing, ensure_ascii=False, indent=1, sort_keys=True), encoding="utf-8")
    dest = _sop_books_path()
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".tmp")
    tmp.write_text(json.dumps(merged, ensure_ascii=False, indent=1, sort_keys=True) + "\n",
                   encoding="utf-8")
    os.replace(tmp, dest)
    _clear_titles_cache()
    return {"detail": f"{len(added)} new title entr{plural} written to {dest}"}


def _filter_for(profile, code, lang) -> dict:
    must = [{"key": profile.identity, "match": {"value": code}}]
    if lang and profile.name == "sop":
        must.append({"key": "lang", "match": {"value": lang}})
    return {"must": must}


def _points_count(adapter: store_adapter.StoreAdapter, collection: str, profile, code, lang) -> int:
    return adapter.count(collection, _filter_for(profile, code, lang))


def _retrievable(adapter: store_adapter.StoreAdapter, collection: str, profile, code, lang) -> bool:
    rows = adapter.scroll(collection, _filter_for(profile, code, lang), 1, with_payload=False)
    return bool(rows)


def _stage_verify(ctx: _Ctx, job_id: str) -> dict:
    books = ctx.reader.manifest.get("books") or []
    titles_table = ctx.merged_titles if ctx.merged_titles is not None else _load_titles_table()
    problems = []
    for b in books:
        code = b.get(ctx.profile.identity)
        lang = b.get("lang")
        want = int(b.get("points", 0))
        got = _points_count(ctx.adapter, ctx.target_collection, ctx.profile, code, lang)
        if got != want:
            problems.append(f"{lang or ''}:{code}: expected {want} point(s), found {got}")
        # The real retrieval check (failure #12): an imported book that a
        # filtered search cannot reach is a failed import, not a quiet success.
        if not _retrievable(ctx.adapter, ctx.target_collection, ctx.profile, code, lang):
            problems.append(f"{lang or ''}:{code}: not reachable by a filtered search")
        if ctx.profile.name == "sop" and not titles_table.get(lang, {}).get(code):
            problems.append(f"{lang}:{code}: no title-table entry")
    if problems:
        raise StageFailure("verify", "; ".join(problems))
    return {"detail": f"{len(books)} book(s) verified: point count, retrievability"
                      + (", title entry" if ctx.profile.name == "sop" else "") + " all ok"}


def _stage_report(ctx: _Ctx, job_id: str) -> dict:
    job = jobs.load(job_id)
    d = jobs.job_dir(job_id)
    lines = [
        f"# Import report — {job_id}",
        "",
        f"- pack: `{job['pack_id']}`",
        f"- profile / collection: `{job['profile']}` / `{ctx.target_collection}`",
        f"- mode: {job['mode']}",
        f"- operator: {job['operator']}",
        f"- allow_overwrite: {job['allow_overwrite']}",
        f"- started: {job.get('started_at')}",
        f"- counts: {json.dumps(job.get('counts', {}))}",
        f"- progress: {json.dumps(job.get('progress', {}))}",
        f"- snapshot: {json.dumps(job.get('snapshot'))}",
        "",
        "## Stages",
        "",
    ]
    for s in job["stages"]:
        line = f"- **{s['name']}**: {s['status']}"
        if s.get("detail"):
            line += f" — {s['detail']}"
        lines.append(line)
    (d / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return {"detail": "report.md written"}


_PIPELINE = [
    ("open", _stage_open),
    ("contract", _stage_contract),
    ("probe", _stage_probe),
    ("preflight", _stage_preflight),
    ("snapshot", _stage_snapshot),
    ("undo", _stage_undo),
    ("upsert", _stage_upsert),
    ("indexes", _stage_indexes),
    ("titles", _stage_titles),
    ("verify", _stage_verify),
    ("report", _stage_report),
]


# ── orchestration ────────────────────────────────────────────────────────────

def _cleanup(ctx: _Ctx, job_id: str) -> None:
    ctx.close()
    if ctx.mode == "dry-run":
        try:
            ctx.adapter.delete_collection(ctx.target_collection)
            jobs.append_log(job_id, "report", "info",
                            f"dropped scratch collection {ctx.target_collection}")
        except Exception as exc:  # best-effort; never masks the real outcome
            jobs.append_log(job_id, "report", "warn",
                            f"could not drop scratch collection {ctx.target_collection}: {exc}")


def _finish(job_id: str, ctx: _Ctx, *, status: str, error: str | None = None) -> None:
    jobs.update(job_id, status=status, finished_at=jobs.now_iso(), error=error)
    _cleanup(ctx, job_id)
    jobs.release_import_lock()


def _run(job_id: str) -> None:
    jobs.update(job_id, status="running", started_at=jobs.now_iso())
    ctx = _Ctx(job_id)
    try:
        for name, fn in _PIPELINE:
            job = jobs.load(job_id)
            entry = jobs.stage_entry(job, name)
            if entry["status"] == "ok" and name not in _ALWAYS_RERUN:
                continue  # resume: already done, and safe to trust (see _ALWAYS_RERUN)
            if job.get("status") == "cancelling":
                jobs.append_log(job_id, name, "warn", "cancel requested before this stage")
                _finish(job_id, ctx, status="cancelled")
                return

            jobs.update_stage(job_id, name, status="running", started_at=jobs.now_iso())
            jobs.update(job_id, stage=name)
            jobs.append_log(job_id, name, "info", f"stage {name} starting")
            try:
                result = fn(ctx, job_id)
            except StageFailure as exc:
                jobs.update_stage(job_id, name, status="failed", finished_at=jobs.now_iso(),
                                  detail=str(exc))
                jobs.append_log(job_id, name, "error", str(exc))
                _finish(job_id, ctx, status="failed", error=f"{name}: {exc}")
                return
            except Cancelled:
                jobs.update_stage(job_id, name, status="failed", finished_at=jobs.now_iso(),
                                  detail="cancelled")
                jobs.append_log(job_id, name, "warn", "cancelled mid-stage")
                _finish(job_id, ctx, status="cancelled")
                return
            except Exception as exc:  # an internal bug must fail the job, not vanish silently
                jobs.update_stage(job_id, name, status="failed", finished_at=jobs.now_iso(),
                                  detail=f"internal error: {exc}")
                jobs.append_log(job_id, name, "error", f"internal error: {exc}")
                _finish(job_id, ctx, status="failed", error=f"{name}: internal error: {exc}")
                return

            skipped = bool(result.get("skipped"))
            jobs.update_stage(job_id, name, status="skipped" if skipped else "ok",
                              finished_at=jobs.now_iso(), detail=result.get("detail", ""))
            jobs.append_log(job_id, name, "info",
                            f"stage {name} {'skipped' if skipped else 'ok'}: "
                            f"{result.get('detail', '')}")

        job = jobs.load(job_id)
        job["rollback"]["available"] = (job["mode"] == "apply")
        jobs.save(job_id, job)
        _finish(job_id, ctx, status="ok")
    except Exception as exc:  # belt-and-braces: never leave the lock held
        jobs.append_log(job_id, None, "error", f"unhandled error: {exc}")
        _finish(job_id, ctx, status="failed", error=str(exc))


# ── public API ───────────────────────────────────────────────────────────────

def start_job(pack_id: str, mode: str, allow_overwrite: bool, operator: str) -> str:
    if mode not in ("dry-run", "apply"):
        raise ValueError(f"mode must be 'dry-run' or 'apply', got {mode!r}")
    pack_path = _packs_dir() / f"{pack_id}.sopack"
    if not pack_path.is_file():
        raise FileNotFoundError(f"pack not found: {pack_path}")
    try:
        with PackReader(pack_path) as peek:
            manifest = peek.manifest
    except PackError as exc:
        raise ValueError(f"cannot open pack: {exc}") from exc
    profile = contract.get_profile(manifest.get("profile", ""))

    job_id = jobs.new_job_id()
    if not jobs.try_acquire_import_lock(job_id):
        raise Busy("another import job is already running")
    try:
        jobs.create(job_id, pack_id=pack_id, mode=mode, profile=profile.name,
                   collection=store_adapter.collection_for(profile.name), operator=operator,
                   allow_overwrite=allow_overwrite)
    except Exception:
        jobs.release_import_lock()
        raise

    t = threading.Thread(target=_run, args=(job_id,), name=f"import-{job_id}", daemon=True)
    t.start()
    return job_id


def get_job(job_id: str) -> dict:
    return jobs.load(job_id)


def list_jobs() -> list[dict]:
    return jobs.list_jobs()


def read_log(job_id: str, after: int = -1) -> dict:
    events, next_seq = jobs.read_log(job_id, after)
    return {"events": events, "next": next_seq}


def cancel(job_id: str) -> None:
    job = jobs.load(job_id)
    if job["status"] in ("queued", "running"):
        jobs.update(job_id, status="cancelling")
        jobs.append_log(job_id, job.get("stage"), "warn", "cancel requested")


def resume(job_id: str) -> None:
    job = jobs.load(job_id)
    if job["status"] != "interrupted":
        raise ValueError(f"job {job_id} is {job['status']!r}, not 'interrupted'")
    if not jobs.try_acquire_import_lock(job_id):
        raise Busy("another import job is already running")
    jobs.append_log(job_id, job.get("stage"), "info", "resuming interrupted job")
    t = threading.Thread(target=_run, args=(job_id,), name=f"import-{job_id}-resume", daemon=True)
    t.start()


def _do_rollback(job_id: str) -> None:
    job = jobs.load(job_id)
    collection = job["collection"]
    adapter = _make_adapter()
    d = jobs.job_dir(job_id)
    try:
        created_path = d / "created_ids.txt"
        created = [ln for ln in created_path.read_text(encoding="utf-8").splitlines() if ln] \
            if created_path.is_file() else []
        if created:
            adapter.delete_points(collection, created)
            jobs.append_log(job_id, "rollback", "info", f"deleted {len(created)} created point(s)")

        undo_path = d / "undo.jsonl"
        restored = 0
        if undo_path.is_file():
            batch: list[dict] = []
            for line in undo_path.read_text(encoding="utf-8").splitlines():
                line = line.strip()
                if not line:
                    continue
                rec = json.loads(line)
                batch.append({"id": rec["id"], "vector": rec["vector"], "payload": rec["payload"]})
                if len(batch) >= _UNDO_BATCH:
                    adapter.upsert(collection, batch)
                    restored += len(batch)
                    batch = []
            if batch:
                adapter.upsert(collection, batch)
                restored += len(batch)
        jobs.append_log(job_id, "rollback", "info", f"restored {restored} overwritten point(s)")

        before_path = d / "sop_books.before.json"
        if before_path.is_file():
            dest = _sop_books_path()
            dest.parent.mkdir(parents=True, exist_ok=True)
            tmp = dest.with_suffix(dest.suffix + ".tmp")
            tmp.write_text(before_path.read_text(encoding="utf-8"), encoding="utf-8")
            os.replace(tmp, dest)
            _clear_titles_cache()
            jobs.append_log(job_id, "rollback", "info", "restored title table")

        jobs.update(job_id, status="rolled_back", finished_at=jobs.now_iso(),
                   rollback={"available": False, "performed_at": jobs.now_iso()})
    except Exception as exc:
        jobs.update(job_id, status="failed", error=f"rollback: {exc}")
        jobs.append_log(job_id, "rollback", "error", str(exc))
        raise
    finally:
        jobs.release_import_lock()


def rollback(job_id: str) -> None:
    """Fine rollback (docs/IMPORT-PIPELINE-PLAN.md §7): delete created_ids.txt's
    ids, re-upsert undo.jsonl, restore sop_books.before.json, clear the title
    cache. Exact, seconds, no collateral damage to anything written since."""
    job = jobs.load(job_id)
    if not (job.get("rollback") or {}).get("available"):
        raise ValueError(f"job {job_id} has no rollback available")
    if not jobs.try_acquire_import_lock(f"rollback-{job_id}"):
        raise Busy("another import job is already running")
    jobs.update(job_id, status="rolling_back")
    t = threading.Thread(target=_do_rollback, args=(job_id,), name=f"rollback-{job_id}",
                        daemon=True)
    t.start()


def _do_restore_snapshot(job_id: str) -> None:
    job = jobs.load(job_id)
    try:
        name = job["snapshot"]["name"]
        jobs.append_log(job_id, "restore-snapshot", "info", f"restoring {name}")
        snapshots.restore(job["collection"], name)
        jobs.append_log(job_id, "restore-snapshot", "info", "restore complete")
    except Exception as exc:
        jobs.append_log(job_id, "restore-snapshot", "error", str(exc))
        raise
    finally:
        jobs.release_import_lock()


def restore_snapshot(job_id: str) -> None:
    """The coarse rollback: puts the whole collection back from the job's
    snapshot, losing anything else written since."""
    job = jobs.load(job_id)
    snap = job.get("snapshot")
    if not snap or not snap.get("name"):
        raise ValueError(f"job {job_id} has no snapshot")
    if not jobs.try_acquire_import_lock(f"restore-{job_id}"):
        raise Busy("another import job is already running")
    t = threading.Thread(target=_do_restore_snapshot, args=(job_id,),
                        name=f"restore-{job_id}", daemon=True)
    t.start()


# Boot recovery: a job still marked as mid-flight when this module is
# (re)imported means the process that was driving it is gone.
boot_recovered = jobs.boot_recover()

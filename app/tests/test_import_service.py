"""Tests for app/import_service.py against an in-process fake Qdrant
(app/tests/fake_qdrant.py) — no network, no fastembed/numpy/qdrant_client.

Focus, per the brief: probe rejection below threshold, snapshot failure
aborting before any write, book metadata gating, preflight
distinguishing collision from re-index, and rollback restoring exactly.
Plus a couple of end-to-end happy-path checks (apply + dry-run) tying the
whole stage machine together.

All of this exercises the **legacy ``sopack/1`` live-canary probe path**
(SOPACK-AUTONOMY.md §4: "the importer accepts sopack/1 ... during the
transition"), which M1 must keep working unchanged. ``sopack.format.PackWriter``
now only ever writes ``sopack/2`` (store-neutral, calibration-fixture probe),
so a /1 pack for these tests is assembled directly here
(``_write_v1_pack``), byte-for-byte the shape the OLD writer produced —
``sopack/2``-specific behaviour is covered separately in
``app/tests/test_store_adapter.py``.

Run: .venv/bin/python3.11 app/tests/test_import_service.py
"""

from __future__ import annotations

import array
import hashlib
import json
import os
import random
import shutil
import sys
import tempfile
import time
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from app.pack import contract  # noqa: E402

from app import import_service, jobs, store_adapter  # noqa: E402
from app.tests.fake_qdrant import FakeQdrant  # noqa: E402

DIM = contract.VECTOR_SIZE


def _vec(seed: int) -> list[float]:
    rnd = random.Random(seed)
    return [rnd.uniform(-1.0, 1.0) for _ in range(DIM)]


def _f32(vector: list[float]) -> bytes:
    return array.array("f", vector).tobytes()


def _write_v1_pack(path, *, profile="sop", pack_id="test-pack", book_code="TST", lang="en",
                   slug="test-book", n_points=3, canary_ids=None, canary_vectors=None,
                   titles=None, point_seed_offset=0, corpus="pioneers", book_pair=None,
                   author="Tester"):
    """A minimal, ``sopack/1``-shaped ``.sopack`` — the legacy live-canary
    manifest shape packs had before M1, assembled directly (the test writer
    in ``app/tests/packs.py`` only writes ``sopack/2``) so the /1 import path
    (probe, preflight, snapshot, upsert, …) keeps being exercised exactly as
    before.

    Book metadata travels on every point and in the manifest's book entry;
    ``corpus=None`` builds a pack that lacks it. ``titles`` attaches a legacy
    titles.json fragment, which the importer ignores.
    """
    prof = contract.get_profile(profile)
    collection = store_adapter.collection_for(profile)

    id_rule = prof.default_id_rule
    points_lines = []
    vectors_bytes = b""
    ids = []
    first_id = None
    for i in range(n_points):
        page, para = i + 1, 1
        para_key = f"{page}.{para}"
        if profile == "sop":
            payload = {
                "lang": lang, "book_code": book_code, "book_pair": book_pair,
                "page": page, "para": para, "para_key": para_key,
                "raw_text": f"paragraph {i} of {book_code}", "aligned": None,
                "slug": slug, "title": f"{book_code} Title", "author": author,
                "year": 2020,
            }
            if corpus:
                payload["corpus"] = corpus
            fields = {"lang": lang, "book_code": book_code, "para_key": para_key, "seq": 0}
        else:
            payload = {"bible": book_code, "osis": f"Gen.1.{i + 1}", "text": f"verse {i}"}
            fields = {"bible": book_code, "osis": f"Gen.1.{i + 1}"}
        uid = f"{lang}:{book_code}:{para_key}#0"
        pid = contract.point_id(id_rule, fields)
        ids.append(pid)
        if first_id is None:
            first_id = pid
        points_lines.append(json.dumps({"uid": uid, "id": pid, "payload": payload},
                                       ensure_ascii=False))
        vectors_bytes += _f32(_vec(point_seed_offset + i))

    book = {prof.identity: book_code, "points": n_points, "first_id": first_id,
           "title": f"{book_code} Title", "author": author, "year": 2020,
           "id_rule": id_rule}
    if profile == "sop":
        book["lang"] = lang
        book["slug"] = slug
        book["corpus"] = corpus
        book["book_pair"] = book_pair

    probe = None
    probe_bytes = b""
    if canary_ids:
        vecs = canary_vectors or [_vec(9000 + i) for i in range(len(canary_ids))]
        probe = {"canaries": [{"id": cid, "collection": collection, "vector_offset": i,
                               "cosine_expected_min": contract.PROBE_MIN_COSINE}
                              for i, cid in enumerate(canary_ids)],
                "vectors": "probe.f32"}
        for v in vecs:
            probe_bytes += _f32(v)

    points_blob = ("\n".join(points_lines) + "\n").encode("utf-8") if points_lines else b""
    titles_blob = (json.dumps(titles, ensure_ascii=False, indent=2).encode("utf-8")
                  if titles is not None else None)

    sha = {"points.jsonl": hashlib.sha256(points_blob).hexdigest(),
          "vectors.f32": hashlib.sha256(vectors_bytes).hexdigest()}
    if probe is not None:
        sha["probe.f32"] = hashlib.sha256(probe_bytes).hexdigest()
    if titles_blob is not None:
        sha["titles.json"] = hashlib.sha256(titles_blob).hexdigest()

    manifest = {
        "schema": "sopack/1",
        "profile": prof.name,
        "pack_id": pack_id,
        "created_by": "test",
        "target": {"collection": collection, "vector_name": store_adapter.QDRANT_VECTOR_NAME,
                   "vector_size": DIM, "distance": store_adapter.QDRANT_DISTANCE},
        "embedding": {"model": contract.EMBEDDING["model"], "library": "fastembed",
                     "library_version": "0.8.0",
                     "pooling": contract.EMBEDDING["pooling"],
                     "normalized": contract.EMBEDDING["normalized"],
                     "passage_prefix": contract.EMBEDDING["passage_prefix"]},
        "id_rule": id_rule,
        "id_rule_doc": contract.ID_RULE_DOC[id_rule],
        "counts": {"points": n_points, "books": (1 if n_points else 0), "dim": DIM,
                  "points_bytes": len(points_blob)},
        "sha256": sha,
        "books": [book] if n_points else [],
        "probe": probe,
    }

    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("manifest.json", json.dumps(manifest, ensure_ascii=False, indent=2))
        zf.writestr("points.jsonl", points_blob)
        info = zipfile.ZipInfo("vectors.f32")
        info.compress_type = zipfile.ZIP_STORED
        zf.writestr(info, vectors_bytes)
        if probe is not None:
            pinfo = zipfile.ZipInfo("probe.f32")
            pinfo.compress_type = zipfile.ZIP_STORED
            zf.writestr(pinfo, probe_bytes)
        if titles_blob is not None:
            zf.writestr("titles.json", titles_blob)
    return ids


# Kept as the name every existing test call site already uses.
_build_pack = _write_v1_pack


class ImportServiceTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.mkdtemp(prefix="import-svc-test-")
        os.environ["JOBS_DIR"] = str(Path(self._tmp) / "jobs")
        os.environ["PACKS_DIR"] = str(Path(self._tmp) / "packs")
        Path(os.environ["PACKS_DIR"]).mkdir(parents=True, exist_ok=True)
        self.qdrant = FakeQdrant()
        os.environ["QDRANT_URL"] = self.qdrant.url
        jobs.release_import_lock()
        # The `sop` collection is what every test targets, matching contract.py.
        self.qdrant.create_collection("sop", store_adapter.QDRANT_VECTOR_NAME, DIM,
                                      store_adapter.QDRANT_DISTANCE)
        self.qdrant.create_collection("bibles", store_adapter.QDRANT_VECTOR_NAME, DIM,
                                      store_adapter.QDRANT_DISTANCE)

    def tearDown(self):
        jobs.release_import_lock()
        self.qdrant.stop()
        shutil.rmtree(self._tmp, ignore_errors=True)
        for k in ("JOBS_DIR", "PACKS_DIR", "QDRANT_URL"):
            os.environ.pop(k, None)

    def _pack_path(self, pack_id="test-pack"):
        return Path(os.environ["PACKS_DIR"]) / f"{pack_id}.sopack"

    def _seed_canary(self, cid="canary-1", vector=None):
        vector = vector or _vec(1)
        self.qdrant.put_point("sop", cid, {"lang": "en", "book_code": "SC",
                                           "para_key": "1.1", "raw_text": "canary"}, vector)
        return cid, vector

    def _wait_terminal(self, job_id, timeout=15.0):
        deadline = time.time() + timeout
        terminal = {"ok", "failed", "cancelled", "rolled_back"}
        job = None
        while time.time() < deadline:
            job = import_service.get_job(job_id)
            if job["status"] in terminal:
                return job
            time.sleep(0.02)
        self.fail(f"job {job_id} did not reach a terminal status in {timeout}s "
                 f"(stuck at {job and job['status']!r})")


class TestProbeRejection(ImportServiceTestCase):
    def test_cosine_below_threshold_aborts_before_any_write(self):
        cid, stored_vec = self._seed_canary()
        # The pack's probe vector is a DIFFERENT random vector — far below 0.95.
        _build_pack(self._pack_path(), canary_ids=[cid], canary_vectors=[_vec(99999)])

        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        job = self._wait_terminal(job_id)

        self.assertEqual(job["status"], "failed")
        self.assertEqual(job["stage"], "probe")  # never advanced past probe
        probe_entry = jobs.stage_entry(job, "probe")
        self.assertEqual(probe_entry["status"], "failed")
        self.assertIn("cosine", probe_entry["detail"])
        # Nothing was written: snapshot never ran, no points changed.
        self.assertEqual(jobs.stage_entry(job, "snapshot")["status"], "pending")
        self.assertEqual(len(self.qdrant.points("sop")), 1)  # only the canary

    def test_missing_canary_id_fails(self):
        _build_pack(self._pack_path(), canary_ids=["does-not-exist"])
        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "failed")
        self.assertIn("missing", jobs.stage_entry(job, "probe")["detail"])

    def test_matching_canary_passes(self):
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), canary_ids=[cid], canary_vectors=[vec])
        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        job = self._wait_terminal(job_id)
        self.assertEqual(jobs.stage_entry(job, "probe")["status"], "ok")
        self.assertIn("1.00000", jobs.stage_entry(job, "probe")["detail"])


class TestSnapshotFailureAborts(ImportServiceTestCase):
    def test_snapshot_failure_leaves_collection_untouched(self):
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), canary_ids=[cid], canary_vectors=[vec])

        before = self.qdrant.snapshot_of("sop")

        # Sabotage snapshot creation without touching import_service's logic:
        # point QDRANT_URL's snapshot route at a collection name that doesn't
        # exist, by monkeypatching snapshots.create to always fail.
        from app import snapshots as snap_mod
        real_create = snap_mod.create

        def _boom(collection, qdrant_url=None, wait=True):
            raise snap_mod.SnapshotError("disk full (simulated)")

        snap_mod.create = _boom
        try:
            job_id = import_service.start_job("test-pack", "apply", False, "tester")
            job = self._wait_terminal(job_id)
        finally:
            snap_mod.create = real_create

        self.assertEqual(job["status"], "failed")
        self.assertEqual(jobs.stage_entry(job, "snapshot")["status"], "failed")
        self.assertIn("disk full", jobs.stage_entry(job, "snapshot")["detail"])
        # upsert never ran; the collection is exactly as it was.
        self.assertEqual(jobs.stage_entry(job, "upsert")["status"], "pending")
        self.assertEqual(self.qdrant.snapshot_of("sop"), before)
        self.assertIsNone(job["snapshot"])


class TestBookMetadata(ImportServiceTestCase):
    """Book metadata lives on the points — no title table anywhere."""

    def _run(self, **pack_kw):
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), canary_ids=[cid], canary_vectors=[vec], **pack_kw)
        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        return self._wait_terminal(job_id)

    def test_preflight_refuses_a_book_without_title(self):
        cid, vec = self._seed_canary()
        ids = _build_pack(self._pack_path(), canary_ids=[cid], canary_vectors=[vec])
        # Re-write the manifest's book entry without a title.
        import zipfile
        path = self._pack_path()
        with zipfile.ZipFile(path) as zf:
            files = {n: zf.read(n) for n in zf.namelist()}
        manifest = json.loads(files["manifest.json"])
        manifest["books"][0]["title"] = None
        files["manifest.json"] = json.dumps(manifest).encode("utf-8")
        with zipfile.ZipFile(path, "w") as zf:
            for n, data in files.items():
                zf.writestr(n, data)
        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "failed")
        self.assertIn("title", jobs.stage_entry(job, "preflight")["detail"])
        self.assertEqual(jobs.stage_entry(job, "snapshot")["status"], "pending")
        self.assertTrue(ids)

    def test_a_legacy_pack_without_corpus_is_stamped_egw(self):
        # Packs built before `corpus: "egw"` existed left Ellen White's books
        # with no corpus (and often no author): the legacy rule makes them EGW,
        # and preflight says so.
        job = self._run(corpus=None, author=None)
        self.assertEqual(job["status"], "ok", job.get("error"))
        self.assertIn("imported as corpus egw", jobs.stage_entry(job, "preflight")["detail"])
        imported = [pt["payload"] for pt in self.qdrant.points("sop").values()
                    if pt["payload"].get("book_code") == "TST"]
        self.assertTrue(imported)
        for payload in imported:
            self.assertEqual(payload["corpus"], "egw")
            self.assertEqual(payload["author"], "Ellen G. White")
            self.assertEqual(payload["book_pair"], "TST")  # English: its own code

    def test_preflight_refuses_a_compound_book_pair(self):
        job = self._run(book_pair="BW/SC")
        self.assertEqual(job["status"], "failed")
        self.assertIn("book_pair", jobs.stage_entry(job, "preflight")["detail"])

    def test_import_keeps_metadata_on_the_points_and_drops_the_book_cache(self):
        # Stand in for app.sop_tools without importing the real module (it
        # imports the MCP framework at module scope); import_service reaches
        # into sys.modules["app.sop_tools"] only if it is already loaded.
        import types
        fake_sop_tools = types.ModuleType("app.sop_tools")
        fake_sop_tools.calls = 0

        def _invalidate():
            fake_sop_tools.calls += 1
        fake_sop_tools.invalidate_books = _invalidate
        sys.modules["app.sop_tools"] = fake_sop_tools
        self.addCleanup(sys.modules.pop, "app.sop_tools", None)

        job = self._run(titles={"en": {"TST": {"titles": ["ignored"]}}})

        self.assertEqual(job["status"], "ok", job.get("error"))
        self.assertIn("book metadata", jobs.stage_entry(job, "verify")["detail"])
        imported = [pt["payload"] for pt in self.qdrant.points("sop").values()
                    if pt["payload"].get("book_code") == "TST"]
        self.assertEqual(len(imported), 3)
        for payload in imported:
            self.assertEqual(payload["corpus"], "pioneers")
            self.assertEqual(payload["title"], "TST Title")
            self.assertNotIn("titles", payload)  # the legacy fragment is ignored
        self.assertEqual(fake_sop_tools.calls, 1)
        self.assertFalse((jobs.job_dir(job["job_id"]) / "sop_books.before.json").exists())


class TestPreflightCollisionVsReindex(ImportServiceTestCase):
    def test_collision_blocks_even_with_allow_overwrite(self):
        # A DIFFERENT work already claims book_code "TST" with a different slug.
        self.qdrant.put_point("sop", "existing-1",
                              {"lang": "en", "book_code": "TST", "slug": "some-other-work",
                               "para_key": "1.1", "raw_text": "..."}, _vec(500))
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), book_code="TST", lang="en", slug="my-new-work",
                   canary_ids=[cid], canary_vectors=[vec])

        job_id = import_service.start_job("test-pack", "apply", True, "tester")  # allow_overwrite=True
        job = self._wait_terminal(job_id)

        self.assertEqual(job["status"], "failed")
        entry = jobs.stage_entry(job, "preflight")
        self.assertEqual(entry["status"], "failed")
        self.assertIn("collision", entry["detail"])
        # Collision is a hard stop regardless of allow_overwrite: nothing written.
        self.assertEqual(jobs.stage_entry(job, "snapshot")["status"], "pending")

    def test_reindex_of_the_same_slug_is_not_a_collision(self):
        # The SAME work, being re-imported (a corrected edition): same
        # book_code/slug, and — because it happens to reuse the exact same
        # point id — this is also a clean point-level overwrite rather than
        # leaving orphaned old content behind.
        cid, vec = self._seed_canary()
        ids = _build_pack(self._pack_path(), book_code="TST", lang="en", slug="test-book",
                          canary_ids=[cid], canary_vectors=[vec])
        self.qdrant.put_point("sop", ids[0],
                              {"lang": "en", "book_code": "TST", "slug": "test-book",
                               "para_key": "1.1", "raw_text": "old wording"}, _vec(501))

        job_id = import_service.start_job("test-pack", "apply", True, "tester")
        job = self._wait_terminal(job_id)

        self.assertNotEqual(jobs.stage_entry(job, "preflight")["status"], "failed")
        self.assertNotIn("collision", jobs.stage_entry(job, "preflight")["detail"])
        self.assertEqual(job["status"], "ok", job.get("error"))
        self.assertEqual(job["counts"]["overwritten"], 1)

    def test_new_point_overwrite_refused_without_allow_overwrite(self):
        # No book-level collision (fresh book_code), but one of the pack's
        # own point ids already exists in the collection under the same slug.
        cid, vec = self._seed_canary()
        ids = _build_pack(self._pack_path(), book_code="TST2", lang="en", slug="fresh-book",
                          canary_ids=[cid], canary_vectors=[vec], n_points=2)
        # Seed a point whose id IS one of the pack's own point ids (simulating
        # a prior partial import of the very same content), same slug.
        self.qdrant.put_point("sop", ids[0],
                              {"lang": "en", "book_code": "TST2", "slug": "fresh-book",
                               "para_key": "1.1", "raw_text": "old text"}, _vec(600))

        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "failed")
        entry = jobs.stage_entry(job, "preflight")
        self.assertIn("already exist", entry["detail"])

        # Same scenario, this time allowed.
        job_id2 = import_service.start_job("test-pack", "apply", True, "tester")
        job2 = self._wait_terminal(job_id2)
        self.assertEqual(job2["status"], "ok", job2.get("error"))
        self.assertEqual(job2["counts"]["overwritten"], 1)


class TestPreflightSameTitle(ImportServiceTestCase):
    """A known work arriving under a NEW book_code (the manifest's TATS for the
    live BP3) — decided from the store's own payloads, never by the client."""

    def _seed_live(self, code, title, author="Tester"):
        self.qdrant.put_point("sop", f"live-{code}",
                              {"lang": "en", "book_code": code, "slug": code.lower(),
                               "title": title, "author": author, "para_key": "1.1",
                               "raw_text": "..."}, _vec(700))

    def test_same_title_under_a_new_code_is_refused(self):
        self._seed_live("BP3", "NEWC Title")  # the pack's book NEWC carries "NEWC Title"
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), book_code="NEWC", slug="newc",
                    canary_ids=[cid], canary_vectors=[vec])

        job = self._wait_terminal(import_service.start_job("test-pack", "apply", True, "tester"))

        self.assertEqual(job["status"], "failed")
        entry = jobs.stage_entry(job, "preflight")
        self.assertIn("allow_same_title", entry["detail"])
        self.assertIn("BP3", entry["detail"])
        self.assertEqual(jobs.stage_entry(job, "snapshot")["status"], "pending")

    def test_allow_same_title_imports_it(self):
        self._seed_live("VOL1", "NEWC Title")  # e.g. a separate volume with an identical title
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), book_code="NEWC", slug="newc",
                    canary_ids=[cid], canary_vectors=[vec])

        job = self._wait_terminal(import_service.start_job("test-pack", "apply", False, "tester",
                                                           allow_same_title=True))

        self.assertEqual(job["status"], "ok", job.get("error"))
        self.assertIn("allowed same title", jobs.stage_entry(job, "preflight")["detail"])

    def test_same_title_by_another_author_is_a_different_work(self):
        self._seed_live("SANC", "NEWC Title", author="Someone Else")
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), book_code="NEWC", slug="newc",
                    canary_ids=[cid], canary_vectors=[vec])

        job = self._wait_terminal(import_service.start_job("test-pack", "apply", False, "tester"))

        self.assertEqual(job["status"], "ok", job.get("error"))


class TestRollbackRestoresExactly(ImportServiceTestCase):
    def test_rollback_restores_prior_state_byte_for_byte(self):
        # One point that will be overwritten, recorded before the import.
        overwritten_id = None
        cid, vec = self._seed_canary()
        ids = _build_pack(self._pack_path(), book_code="TST3", lang="en", slug="rollback-book",
                          canary_ids=[cid], canary_vectors=[vec], n_points=2)
        overwritten_id = ids[0]
        self.qdrant.put_point("sop", overwritten_id,
                              {"lang": "en", "book_code": "TST3", "slug": "rollback-book",
                               "para_key": "1.1", "raw_text": "ORIGINAL TEXT"}, _vec(700))

        before = self.qdrant.snapshot_of("sop")
        before_count = len(self.qdrant.points("sop"))

        job_id = import_service.start_job("test-pack", "apply", True, "tester")
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "ok", job.get("error"))
        self.assertNotEqual(self.qdrant.snapshot_of("sop"), before)
        self.assertEqual(len(self.qdrant.points("sop")), before_count + 1)  # +1 new point
        self.assertTrue(job["rollback"]["available"])

        import_service.rollback(job_id)
        deadline = time.time() + 10
        while time.time() < deadline:
            job = import_service.get_job(job_id)
            if job["status"] in ("rolled_back", "failed"):
                break
            time.sleep(0.02)

        self.assertEqual(job["status"], "rolled_back", job.get("error"))
        self.assertEqual(self.qdrant.snapshot_of("sop"), before,
                         "collection must be byte-identical to its pre-import state")
        self.assertEqual(len(self.qdrant.points("sop")), before_count)
        self.assertFalse(job["rollback"]["available"])


class TestDryRun(ImportServiceTestCase):
    def test_dry_run_uses_scratch_collection_and_drops_it(self):
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), book_code="TST4", lang="en", slug="dryrun-book",
                   canary_ids=[cid], canary_vectors=[vec], n_points=2)
        before = self.qdrant.snapshot_of("sop")

        job_id = import_service.start_job("test-pack", "dry-run", False, "tester")
        job = self._wait_terminal(job_id)

        self.assertEqual(job["status"], "ok", job.get("error"))
        self.assertEqual(jobs.stage_entry(job, "snapshot")["status"], "skipped")
        # The real `sop` collection was never touched...
        self.assertEqual(self.qdrant.snapshot_of("sop"), before)
        # ...and the scratch collection was dropped at the end.
        self.assertNotIn("sop__dryrun", self.qdrant.collections)
        self.assertFalse(job["rollback"]["available"])


class TestResume(ImportServiceTestCase):
    def test_resume_of_an_interrupted_job_completes_it(self):
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), book_code="TST5", lang="en", slug="resume-book",
                   canary_ids=[cid], canary_vectors=[vec], n_points=2)

        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "ok", job.get("error"))

        # Simulate "process died mid-upsert": what boot_recover would do to a
        # job caught running, applied directly here (a fresh worker thread
        # has no memory of the one that got interrupted).
        jobs.update(job_id, status="interrupted")
        jobs.update_stage(job_id, "verify", status="pending", finished_at=None, detail=None)
        jobs.update_stage(job_id, "report", status="pending", finished_at=None, detail=None)

        import_service.resume(job_id)
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "ok", job.get("error"))
        self.assertEqual(jobs.stage_entry(job, "verify")["status"], "ok")
        self.assertEqual(jobs.stage_entry(job, "report")["status"], "ok")
        # Re-running preflight/upsert on resume must not have duplicated points.
        self.assertEqual(len(self.qdrant.points("sop")), 3)  # canary + 2 pack points

    def test_resume_refuses_a_non_interrupted_job(self):
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), canary_ids=[cid], canary_vectors=[vec])
        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        self._wait_terminal(job_id)  # now 'ok', not 'interrupted'
        with self.assertRaises(ValueError):
            import_service.resume(job_id)


class TestCancel(ImportServiceTestCase):
    def test_cancel_before_worker_reaches_next_stage_marks_cancelled(self):
        cid, vec = self._seed_canary()
        _build_pack(self._pack_path(), canary_ids=[cid], canary_vectors=[vec])
        job_id = import_service.start_job("test-pack", "apply", False, "tester")
        import_service.cancel(job_id)
        job = self._wait_terminal(job_id)
        self.assertIn(job["status"], ("cancelled", "ok"))  # small race is acceptable
        if job["status"] == "cancelled":
            self.assertEqual(len(self.qdrant.points("sop")), 1)  # only the canary


class TestDeleteBooks(ImportServiceTestCase):
    """Book delete: a ``kind: "delete"`` job removes every point of a
    lang + book_code, after a snapshot and an undo ledger; rollback puts the
    book back byte for byte."""

    def _seed_book(self, code, n=3, lang="en", title="Old Copy", **extra):
        for i in range(n):
            self.qdrant.put_point("sop", f"{lang}-{code}-{i}",
                                  {"lang": lang, "book_code": code, "para_key": f"1.{i}",
                                   "raw_text": f"{code} {i}", "title": title,
                                   "author": "Tester", "corpus": "pioneers", **extra},
                                  _vec(900 + i))

    def _wait_status(self, job_id, statuses, timeout=10.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            job = import_service.get_job(job_id)
            if job["status"] in statuses:
                return job
            time.sleep(0.02)
        self.fail(f"job {job_id} stuck at {job['status']!r}")

    def test_delete_removes_only_that_book_and_rollback_restores_it(self):
        self._seed_book("OLD", 3)
        self._seed_book("NEW", 2, title="Old Copy")
        self._seed_book("OLD", 2, lang="de", book_pair="OLD")  # same code, other language
        before = self.qdrant.snapshot_of("sop")

        job_id = import_service.start_delete([{"lang": "en", "book_code": "OLD"}], "tester",
                                             allow_paired=True)
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "ok", job.get("error"))
        self.assertEqual(job["kind"], "delete")
        self.assertEqual([s["name"] for s in job["stages"]], jobs.DELETE_STAGES)
        self.assertEqual(job["counts"]["deleted"], 3)
        self.assertIsNotNone(job["snapshot"])
        left = {p["payload"]["lang"] + ":" + p["payload"]["book_code"]
                for p in self.qdrant.points("sop").values()}
        self.assertEqual(left, {"en:NEW", "de:OLD"})
        report = (jobs.job_dir(job_id) / "report.md").read_text(encoding="utf-8")
        self.assertIn("# Delete report", report)
        self.assertIn("en:OLD", report)

        import_service.rollback(job_id)
        job = self._wait_status(job_id, ("rolled_back", "failed"))
        self.assertEqual(job["status"], "rolled_back", job.get("error"))
        self.assertEqual(self.qdrant.snapshot_of("sop"), before)

    def test_unknown_book_refuses_and_deletes_nothing(self):
        self._seed_book("OLD", 2)
        before = self.qdrant.snapshot_of("sop")
        job_id = import_service.start_delete([{"lang": "en", "book_code": "OLD"},
                                              {"lang": "en", "book_code": "TYPO"}], "tester")
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "failed")
        self.assertIn("en:TYPO has no points", jobs.stage_entry(job, "preflight")["detail"])
        self.assertEqual(self.qdrant.snapshot_of("sop"), before)

    def test_original_of_a_live_translation_needs_allow_paired(self):
        self._seed_book("OLD", 2)
        self._seed_book("OLD", 1, lang="es", book_pair="OLD")
        self._seed_book("OLDR", 1, lang="ru", book_pair="OLD")
        job_id = import_service.start_delete([{"lang": "en", "book_code": "OLD"}], "tester")
        job = self._wait_terminal(job_id)
        self.assertEqual(job["status"], "failed")
        detail = jobs.stage_entry(job, "preflight")["detail"]
        self.assertIn("allow_paired", detail)
        self.assertIn("es:OLD", detail)
        self.assertIn("ru:OLDR", detail)
        self.assertEqual(len(self.qdrant.points("sop")), 4)

    def test_describe_books_reads_the_store(self):
        self._seed_book("OLD", 3, year=1868)
        self._seed_book("OLD", 1, lang="es", book_pair="OLD")
        got = import_service.describe_books([{"lang": "en", "book_code": "OLD"},
                                             {"lang": "en", "book_code": "NONE"}])
        self.assertEqual(got[0]["points"], 3)
        self.assertEqual(got[0]["title"], "Old Copy")
        self.assertEqual(got[0]["year"], 1868)
        self.assertEqual(got[0]["paired"], ["es:OLD"])
        self.assertEqual(got[1]["points"], 0)

    def test_bad_book_spec_is_a_value_error(self):
        with self.assertRaises(ValueError):
            import_service.start_delete([], "tester")
        with self.assertRaises(ValueError):
            import_service.start_delete([{"lang": "en"}], "tester")


if __name__ == "__main__":
    unittest.main()

"""Unit tests for app/jobs.py — the job store: create/load/save, atomic
writes, the append-only log, the global import lock and boot recovery.

Run: .venv/bin/python3.11 app/tests/test_jobs.py
"""

from __future__ import annotations

import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from app import jobs  # noqa: E402


class JobsTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.mkdtemp(prefix="jobs-test-")
        os.environ["JOBS_DIR"] = self._tmp
        # Make sure no import-order state leaks between tests.
        jobs.release_import_lock()

    def tearDown(self):
        jobs.release_import_lock()
        shutil.rmtree(self._tmp, ignore_errors=True)
        os.environ.pop("JOBS_DIR", None)


class TestCreateLoadSave(JobsTestCase):
    def test_create_then_load_round_trips(self):
        job_id = jobs.new_job_id()
        created = jobs.create(job_id, pack_id="p1", mode="apply", profile="sop",
                              collection="sop", operator="tester", allow_overwrite=False)
        loaded = jobs.load(job_id)
        self.assertEqual(loaded, created)
        self.assertEqual(loaded["status"], "queued")
        self.assertEqual([s["name"] for s in loaded["stages"]], jobs.STAGES)
        self.assertTrue(all(s["status"] == "pending" for s in loaded["stages"]))

    def test_save_is_atomic_no_tmp_left_behind(self):
        job_id = jobs.new_job_id()
        jobs.create(job_id, pack_id="p1", mode="apply", profile="sop", collection="sop",
                   operator="t", allow_overwrite=False)
        jobs.update(job_id, status="running")
        job_path = jobs.job_dir(job_id) / "job.json"
        tmp_path = job_path.with_suffix(job_path.suffix + ".tmp")
        self.assertTrue(job_path.is_file())
        self.assertFalse(tmp_path.exists())
        self.assertEqual(jobs.load(job_id)["status"], "running")

    def test_update_stage_merges_without_clobbering_other_fields(self):
        job_id = jobs.new_job_id()
        jobs.create(job_id, pack_id="p1", mode="apply", profile="sop", collection="sop",
                   operator="t", allow_overwrite=False)
        jobs.update_stage(job_id, "open", status="ok", detail="42 points")
        jobs.update_stage(job_id, "contract", status="running")
        job = jobs.load(job_id)
        self.assertEqual(jobs.stage_entry(job, "open")["status"], "ok")
        self.assertEqual(jobs.stage_entry(job, "open")["detail"], "42 points")
        self.assertEqual(jobs.stage_entry(job, "contract")["status"], "running")
        self.assertEqual(jobs.stage_entry(job, "probe")["status"], "pending")

    def test_list_jobs_newest_first(self):
        ids = []
        for i in range(3):
            job_id = f"2026-09-2{i}T00-00-00-aaaa"
            jobs.create(job_id, pack_id="p", mode="apply", profile="sop", collection="sop",
                       operator="t", allow_overwrite=False)
            # `create` stamps the real wall-clock time, which several calls in
            # the same test can tie on (second resolution) — pin created_at
            # explicitly so the ordering under test isn't a race.
            jobs.update(job_id, created_at=f"2026-09-2{i}T00:00:00Z")
            ids.append(job_id)
        listed = [j["job_id"] for j in jobs.list_jobs()]
        self.assertEqual(listed, list(reversed(ids)))


class TestLog(JobsTestCase):
    def test_append_log_seq_is_monotonic(self):
        job_id = jobs.new_job_id()
        jobs.create(job_id, pack_id="p", mode="apply", profile="sop", collection="sop",
                   operator="t", allow_overwrite=False)
        events = [jobs.append_log(job_id, "open", "info", f"event {i}") for i in range(5)]
        self.assertEqual([e["seq"] for e in events], [0, 1, 2, 3, 4])

    def test_read_log_after_filters_and_reports_next(self):
        job_id = jobs.new_job_id()
        jobs.create(job_id, pack_id="p", mode="apply", profile="sop", collection="sop",
                   operator="t", allow_overwrite=False)
        for i in range(5):
            jobs.append_log(job_id, "open", "info", f"event {i}")
        events, nxt = jobs.read_log(job_id, after=-1)
        self.assertEqual(len(events), 5)
        self.assertEqual(nxt, 5)
        events, nxt = jobs.read_log(job_id, after=2)
        self.assertEqual([e["seq"] for e in events], [3, 4])
        self.assertEqual(nxt, 5)
        events, nxt = jobs.read_log(job_id, after=4)
        self.assertEqual(events, [])
        self.assertEqual(nxt, 5)


class TestImportLock(JobsTestCase):
    def test_second_acquire_fails_while_first_holds_it(self):
        self.assertTrue(jobs.try_acquire_import_lock("job-a"))
        self.assertFalse(jobs.try_acquire_import_lock("job-b"))
        jobs.release_import_lock()
        self.assertTrue(jobs.try_acquire_import_lock("job-b"))

    def test_acquire_refused_if_a_job_on_disk_is_still_running(self):
        # Simulates a second process: no in-memory lock held here, but a job
        # file on disk claims to be running.
        job_id = jobs.new_job_id()
        jobs.create(job_id, pack_id="p", mode="apply", profile="sop", collection="sop",
                   operator="t", allow_overwrite=False)
        jobs.update(job_id, status="running")
        self.assertFalse(jobs.try_acquire_import_lock("some-other-job"))


class TestBootRecovery(JobsTestCase):
    def test_running_job_becomes_interrupted(self):
        job_id = jobs.new_job_id()
        jobs.create(job_id, pack_id="p", mode="apply", profile="sop", collection="sop",
                   operator="t", allow_overwrite=False)
        jobs.update(job_id, status="running", stage="upsert")
        recovered = jobs.boot_recover()
        self.assertEqual(recovered, [job_id])
        job = jobs.load(job_id)
        self.assertEqual(job["status"], "interrupted")
        self.assertIsNotNone(job["finished_at"])

    def test_terminal_job_is_left_alone(self):
        job_id = jobs.new_job_id()
        jobs.create(job_id, pack_id="p", mode="apply", profile="sop", collection="sop",
                   operator="t", allow_overwrite=False)
        jobs.update(job_id, status="ok")
        recovered = jobs.boot_recover()
        self.assertEqual(recovered, [])
        self.assertEqual(jobs.load(job_id)["status"], "ok")


if __name__ == "__main__":
    unittest.main()

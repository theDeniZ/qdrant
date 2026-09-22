"""Tests for the admin import routes (app/admin.py + app/uploads.py).

Standalone ``unittest`` — run with::

    cd /workspaces/sdarm/qdrant && .venv/bin/python3.11 -m app.tests.test_admin_import

``app/import_service.py`` (the job-execution seam, docs/IMPORT-API.md §3) is
owned by another agent and does not exist yet at the time this file was
written. ``app/admin.py`` imports it unconditionally (``from . import
import_service``), exactly as it will in production, so here we register a
minimal stand-in module in ``sys.modules`` *before* importing ``app.admin`` —
this is the "stub it only inside your own tests" allowance in the task brief.
Every route that only needs upload/pack storage (the part this file actually
owns) exercises the real ``app/uploads.py``, not a stub.
"""

from __future__ import annotations

import base64
import hashlib
import os
import shutil
import sys
import tempfile
import types
import unittest
from pathlib import Path

# ── environment must be set before importing app.admin / app.uploads ────────
_TMP = tempfile.mkdtemp(prefix="sdarm-import-test-")
os.environ["ADMIN_PASSWORD"] = "test-secret"
os.environ["KEYS_DB"] = str(Path(_TMP) / "keys.db")
os.environ["UPLOADS_DIR"] = str(Path(_TMP) / "uploads")
os.environ["PACKS_DIR"] = str(Path(_TMP) / "packs")
os.environ["JOBS_DIR"] = str(Path(_TMP) / "jobs")

# ── stub the import_service seam ─────────────────────────────────────────────
#
# A real app/import_service.py now exists alongside this file (written by
# another agent per the task brief), but its start_job() spins up a real
# background worker thread that talks to a live Qdrant over HTTP — not
# something a deterministic, offline unit-test suite should depend on for
# timing or network availability. So this file keeps its own stand-in,
# matching the *real* module's actual contract (verified by reading
# app/import_service.py and app/jobs.py directly): get_job/cancel/resume/
# rollback/restore_snapshot raise FileNotFoundError for an unknown job_id
# (they bottom out in app.jobs.load(), which does), not return None; a busy
# import lock raises import_service.Busy; read_log returns a dict, not a
# tuple.
_fake_jobs: dict = {}


class _Busy(Exception):
    pass


def _fake_start_job(pack_id, mode, allow_overwrite, operator):
    job_id = f"job-{len(_fake_jobs) + 1}"
    _fake_jobs[job_id] = {
        "job_id": job_id, "pack_id": pack_id, "mode": mode, "status": "queued",
        "stage": None, "operator": operator, "allow_overwrite": allow_overwrite,
        "created_at": "2026-09-22T00:00:00Z", "started_at": None, "finished_at": None,
        "stages": [], "progress": {}, "counts": {},
        "snapshot": {"name": "snap-test.snapshot"},
        "rollback": {"available": True, "performed_at": None}, "error": None,
    }
    return job_id


def _fake_get_job(job_id):
    if job_id not in _fake_jobs:
        raise FileNotFoundError(job_id)
    return _fake_jobs[job_id]


_fake_import_service = types.ModuleType("app.import_service")
_fake_import_service.Busy = _Busy
_fake_import_service.start_job = _fake_start_job
_fake_import_service.get_job = _fake_get_job
_fake_import_service.list_jobs = lambda: list(_fake_jobs.values())
_fake_import_service.read_log = lambda job_id, after: {"events": [], "next": after}
_fake_import_service.cancel = lambda job_id: _fake_get_job(job_id).__setitem__("status", "cancelled")
_fake_import_service.resume = lambda job_id: _fake_get_job(job_id).__setitem__("status", "running")
_fake_import_service.rollback = lambda job_id: _fake_get_job(job_id).__setitem__("status", "rolled_back")
_fake_import_service.restore_snapshot = (
    lambda job_id: _fake_get_job(job_id).__setitem__("status", "restoring"))
sys.modules["app.import_service"] = _fake_import_service

from starlette.testclient import TestClient  # noqa: E402

import app.admin as admin  # noqa: E402
from sopack.format import PackWriter  # noqa: E402


def _auth_header(password: str = "test-secret", user: str = "op") -> dict:
    token = base64.b64encode(f"{user}:{password}".encode()).decode()
    return {"Authorization": f"Basic {token}"}


AUTH = _auth_header()


def _make_pack_bytes() -> bytes:
    """A tiny but structurally real .sopack (profile 'bible', 1 point, dim 4 —
    small on purpose; nothing in the upload path cares about dim/contract,
    that is the import job's 'contract' stage, owned elsewhere)."""
    path = Path(_TMP) / "src.sopack"
    with PackWriter(path, profile="bible", pack_id="src", created_by="test", dim=4) as w:
        w.add("v1", {"bible": "kjv", "osis": "Gen.1.1", "text": "In the beginning..."},
              [0.1, 0.2, 0.3, 0.4])
    data = path.read_bytes()
    path.unlink()
    return data


class AdminImportTests(unittest.TestCase):
    def setUp(self):
        self.client = TestClient(admin.app)

    # ── auth / csrf ──────────────────────────────────────────────────────

    def test_auth_rejected_without_password(self):
        res = self.client.get("/")
        self.assertEqual(res.status_code, 401)

        res = self.client.get("/import")
        self.assertEqual(res.status_code, 401)

        res = self.client.get("/import/packs")
        self.assertEqual(res.status_code, 401)

    def test_csrf_rejected_on_bad_origin(self):
        res = self.client.post(
            "/import/uploads",
            json={"name": "x.sopack", "size": 10, "sha256": "0" * 64},
            headers={**AUTH, "Origin": "http://evil.example"},
        )
        self.assertEqual(res.status_code, 403)

    # ── chunked upload round trip (incl. a resumed upload) ──────────────

    def test_chunked_upload_round_trip_with_resume(self):
        data = _make_pack_bytes()
        sha256 = hashlib.sha256(data).hexdigest()
        mid = len(data) // 2
        chunk_a, chunk_b = data[:mid], data[mid:]

        created = self.client.post(
            "/import/uploads",
            json={"name": "corpus.sopack", "size": len(data), "sha256": sha256},
            headers=AUTH,
        )
        self.assertEqual(created.status_code, 200, created.text)
        body = created.json()
        upload_id = body["upload_id"]
        self.assertEqual(body["received"], 0)
        self.assertEqual(body["part_size"], 8 * 1024 * 1024)

        put0 = self.client.put(
            f"/import/uploads/{upload_id}/parts/0", content=chunk_a,
            headers={**AUTH, "Content-Type": "application/octet-stream"},
        )
        self.assertEqual(put0.status_code, 200, put0.text)
        self.assertEqual(put0.json(), {"received": 1, "bytes": len(chunk_a)})

        # Simulate a dropped connection: the client re-fetches status and
        # resumes from `received` rather than replaying part 0.
        st = self.client.get(f"/import/uploads/{upload_id}", headers=AUTH)
        self.assertEqual(st.status_code, 200)
        st_body = st.json()
        self.assertEqual(st_body["received"], 1)
        self.assertFalse(st_body["complete"])

        put1 = self.client.put(
            f"/import/uploads/{upload_id}/parts/{st_body['received']}", content=chunk_b,
            headers={**AUTH, "Content-Type": "application/octet-stream"},
        )
        self.assertEqual(put1.status_code, 200, put1.text)
        self.assertEqual(put1.json(), {"received": 2, "bytes": len(data)})

        # A retry of an already-received part is an idempotent no-op.
        retry = self.client.put(
            f"/import/uploads/{upload_id}/parts/1", content=chunk_b,
            headers={**AUTH, "Content-Type": "application/octet-stream"},
        )
        self.assertEqual(retry.status_code, 200)
        self.assertEqual(retry.json(), {"received": 2, "bytes": len(data)})

        done = self.client.post(f"/import/uploads/{upload_id}/complete", headers=AUTH)
        self.assertEqual(done.status_code, 200, done.text)
        done_body = done.json()
        self.assertTrue(done_body["pack_id"])
        self.assertEqual(done_body["manifest"]["profile"], "bible")
        self.assertEqual(done_body["manifest"]["counts"]["points"], 1)

        pack_path = Path(os.environ["PACKS_DIR"]) / f"{done_body['pack_id']}.sopack"
        self.assertTrue(pack_path.exists())

        # Staging is gone once the pack is finalized.
        gone = self.client.get(f"/import/uploads/{upload_id}", headers=AUTH)
        self.assertEqual(gone.status_code, 404)

        # And it shows up in the packs list.
        packs = self.client.get("/import/packs", headers=AUTH).json()["packs"]
        self.assertIn(done_body["pack_id"], [p["pack_id"] for p in packs])

    def test_checksum_mismatch_rejected_and_discarded(self):
        payload = b"not a real sopack, just some bytes"
        created = self.client.post(
            "/import/uploads",
            json={"name": "bad.sopack", "size": len(payload), "sha256": "0" * 64},
            headers=AUTH,
        ).json()
        upload_id = created["upload_id"]

        put = self.client.put(
            f"/import/uploads/{upload_id}/parts/0", content=payload,
            headers={**AUTH, "Content-Type": "application/octet-stream"},
        )
        self.assertEqual(put.status_code, 200)

        res = self.client.post(f"/import/uploads/{upload_id}/complete", headers=AUTH)
        self.assertEqual(res.status_code, 400)
        self.assertEqual(res.json()["error"], "checksum_mismatch")

        # discarded — the staging upload is gone
        res2 = self.client.get(f"/import/uploads/{upload_id}", headers=AUTH)
        self.assertEqual(res2.status_code, 404)

    def test_out_of_order_part_rejected(self):
        created = self.client.post(
            "/import/uploads",
            json={"name": "gap.sopack", "size": 32, "sha256": "0" * 64},
            headers=AUTH,
        ).json()
        upload_id = created["upload_id"]

        res = self.client.put(
            f"/import/uploads/{upload_id}/parts/1", content=b"x" * 16,
            headers={**AUTH, "Content-Type": "application/octet-stream"},
        )
        self.assertEqual(res.status_code, 400)
        self.assertEqual(res.json()["error"], "part_gap")

    # ── jobs: confirm mismatch ───────────────────────────────────────────

    def test_confirm_mismatch_on_rollback(self):
        created = self.client.post(
            "/import/jobs",
            json={"pack_id": "some-pack", "mode": "apply", "allow_overwrite": False},
            headers=AUTH,
        )
        self.assertEqual(created.status_code, 202, created.text)
        job_id = created.json()["job_id"]

        bad = self.client.post(
            f"/import/jobs/{job_id}/rollback", json={"confirm": "definitely-not-the-job-id"},
            headers=AUTH,
        )
        self.assertEqual(bad.status_code, 400)
        self.assertEqual(bad.json()["error"], "confirm_mismatch")

        good = self.client.post(
            f"/import/jobs/{job_id}/rollback", json={"confirm": job_id}, headers=AUTH,
        )
        self.assertEqual(good.status_code, 200, good.text)
        self.assertEqual(good.json(), {"status": "rolling_back"})

    def test_confirm_mismatch_on_restore_snapshot(self):
        created = self.client.post(
            "/import/jobs",
            json={"pack_id": "some-pack", "mode": "dry-run", "allow_overwrite": False},
            headers=AUTH,
        )
        job_id = created.json()["job_id"]

        bad = self.client.post(
            f"/import/jobs/{job_id}/restore-snapshot", json={"confirm": "wrong-name"},
            headers=AUTH,
        )
        self.assertEqual(bad.status_code, 400)
        self.assertEqual(bad.json()["error"], "confirm_mismatch")

        good = self.client.post(
            f"/import/jobs/{job_id}/restore-snapshot",
            json={"confirm": "snap-test.snapshot"}, headers=AUTH,
        )
        self.assertEqual(good.status_code, 200, good.text)
        self.assertEqual(good.json(), {"status": "restoring"})


def tearDownModule():
    shutil.rmtree(_TMP, ignore_errors=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)

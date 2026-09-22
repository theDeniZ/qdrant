"""Job store for the corpus import pipeline (see docs/IMPORT-API.md).

Owns everything about a job's life on disk: creation, atomic reads/writes,
the append-only event log, the global single-import lock and boot recovery.
The stage machine itself lives in ``import_service.py`` — this module knows
nothing about stages beyond their names and status vocabulary.

Jobs live in ``/data/jobs/{job_id}/`` (``JOBS_DIR`` env var overrides, for
tests). Every mutation goes through :func:`update` / :func:`update_stage` /
:func:`append_log` / :func:`save`, all of which serialize through one
in-process lock and write ``job.json`` atomically (``.tmp`` + ``os.replace``)
so a reader never sees a partial file (docs/IMPORT-API.md §4).

Stdlib only.
"""

from __future__ import annotations

import json
import os
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

STAGES = ["open", "contract", "probe", "preflight", "snapshot", "undo",
          "upsert", "indexes", "titles", "verify", "report"]

STAGE_STATUSES = {"pending", "running", "ok", "failed", "skipped"}
JOB_STATUSES = {"queued", "running", "ok", "failed", "cancelled", "cancelling",
                 "interrupted", "rolling_back", "rolled_back", "restoring"}

# Statuses a job can be found in at process start that mean "something was
# driving this and it is gone now" — boot recovery turns these into
# 'interrupted' (a job found running at startup, docs/IMPORT-PIPELINE-PLAN.md §6.2).
_ORPHANABLE = {"queued", "running", "cancelling", "rolling_back", "restoring"}

# One process-wide lock: never two concurrent imports (docs/IMPORT-API.md §3).
_import_lock = threading.Lock()
_running_job_id: str | None = None

# Serializes every job.json read-modify-write and log append so concurrent
# callers (the worker thread, an admin-triggered cancel/rollback) never race
# on the same file.
_lock = threading.RLock()


# ── paths ────────────────────────────────────────────────────────────────────

def jobs_dir() -> Path:
    return Path(os.environ.get("JOBS_DIR", "/data/jobs"))


def job_dir(job_id: str) -> Path:
    return jobs_dir() / job_id


def _job_path(job_id: str) -> Path:
    return job_dir(job_id) / "job.json"


def _log_path(job_id: str) -> Path:
    return job_dir(job_id) / "log.ndjson"


# ── time / ids ───────────────────────────────────────────────────────────────

def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def new_job_id() -> str:
    ts = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H-%M-%S")
    return f"{ts}-{os.urandom(2).hex()}"


# ── atomic file I/O ──────────────────────────────────────────────────────────

def _atomic_write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(text, encoding="utf-8")
    os.replace(tmp, path)


# ── job CRUD ─────────────────────────────────────────────────────────────────

def create(job_id: str, *, pack_id: str, mode: str, profile: str, collection: str,
           operator: str, allow_overwrite: bool) -> dict:
    """Create a new job directory + job.json in status 'queued'."""
    job = {
        "job_id": job_id,
        "pack_id": pack_id,
        "mode": mode,
        "profile": profile,
        "collection": collection,
        "status": "queued",
        "stage": None,
        "created_at": now_iso(),
        "started_at": None,
        "finished_at": None,
        "operator": operator,
        "allow_overwrite": bool(allow_overwrite),
        "stages": [{"name": n, "status": "pending", "started_at": None,
                    "finished_at": None, "detail": None} for n in STAGES],
        "progress": {},
        "snapshot": None,
        "counts": {"created": 0, "overwritten": 0, "books": 0},
        "error": None,
        "rollback": {"available": False, "performed_at": None},
    }
    save(job_id, job)
    _log_path(job_id).touch(exist_ok=True)
    return job


def load(job_id: str) -> dict:
    return json.loads(_job_path(job_id).read_text(encoding="utf-8"))


def exists(job_id: str) -> bool:
    return _job_path(job_id).is_file()


def save(job_id: str, job: dict) -> None:
    with _lock:
        _atomic_write(_job_path(job_id), json.dumps(job, ensure_ascii=False, indent=2))


def update(job_id: str, **fields) -> dict:
    """Load, shallow-merge top-level *fields*, save. Returns the updated job.

    Use this (never a bare load→mutate→save) so concurrent callers — the
    worker thread and an admin-triggered cancel/rollback — never clobber each
    other's writes.
    """
    with _lock:
        job = load(job_id)
        job.update(fields)
        save(job_id, job)
        return job


def stage_entry(job: dict, name: str) -> dict:
    for s in job["stages"]:
        if s["name"] == name:
            return s
    raise KeyError(f"no such stage {name!r}")


def update_stage(job_id: str, name: str, **fields) -> dict:
    with _lock:
        job = load(job_id)
        stage_entry(job, name).update(fields)
        save(job_id, job)
        return job


def list_jobs() -> list[dict]:
    root = jobs_dir()
    if not root.is_dir():
        return []
    out = []
    for d in root.iterdir():
        jp = d / "job.json"
        if not jp.is_file():
            continue
        try:
            out.append(json.loads(jp.read_text(encoding="utf-8")))
        except (json.JSONDecodeError, OSError):
            continue
    out.sort(key=lambda j: j.get("created_at") or "", reverse=True)
    return out


def prune(keep: int = 20) -> list[str]:
    """Delete the oldest job directories beyond *keep*, terminal jobs only."""
    import shutil
    jobs = [j for j in list_jobs() if j.get("status") in
            ("ok", "failed", "cancelled", "rolled_back")]
    doomed = jobs[keep:]
    removed = []
    for j in doomed:
        shutil.rmtree(job_dir(j["job_id"]), ignore_errors=True)
        removed.append(j["job_id"])
    return removed


# ── log ──────────────────────────────────────────────────────────────────────

def _next_seq(path: Path) -> int:
    if not path.exists() or path.stat().st_size == 0:
        return 0
    last = -1
    with path.open(encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                last = json.loads(line)["seq"]
            except (json.JSONDecodeError, KeyError):
                continue
    return last + 1


def append_log(job_id: str, stage: str | None, level: str, msg: str,
               data: dict | None = None) -> dict:
    with _lock:
        p = _log_path(job_id)
        seq = _next_seq(p)
        event = {"seq": seq, "ts": now_iso(), "stage": stage, "level": level,
                 "msg": msg, "data": data or {}}
        with p.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(event, ensure_ascii=False) + "\n")
        return event


def read_log(job_id: str, after: int = -1) -> tuple[list[dict], int]:
    """Events with seq > *after*, and the next 'after' value to poll with."""
    p = _log_path(job_id)
    events = []
    if p.is_file():
        with p.open(encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                ev = json.loads(line)
                if ev["seq"] > after:
                    events.append(ev)
    next_seq = events[-1]["seq"] + 1 if events else after + 1
    return events, next_seq


# ── the global single-import lock ───────────────────────────────────────────

def try_acquire_import_lock(job_id: str) -> bool:
    """True if *job_id* now owns the one import lock. Also refuses if any job
    on disk is still mid-flight (best-effort cross-process guard)."""
    global _running_job_id
    with _lock:
        if _import_lock.locked():
            return False
        for j in list_jobs():
            if j["job_id"] != job_id and j.get("status") in _ORPHANABLE:
                return False
        if _import_lock.acquire(blocking=False):
            _running_job_id = job_id
            return True
        return False


def release_import_lock() -> None:
    global _running_job_id
    with _lock:
        _running_job_id = None
        if _import_lock.locked():
            try:
                _import_lock.release()
            except RuntimeError:
                pass


def current_running_job() -> str | None:
    return _running_job_id


# ── boot recovery ────────────────────────────────────────────────────────────

def boot_recover() -> list[str]:
    """A job found mid-flight at process start had its driving thread killed
    with the old process — mark it 'interrupted' so it becomes resumable
    (docs/IMPORT-PIPELINE-PLAN.md §6.2) instead of forever claiming to run."""
    recovered = []
    for job in list_jobs():
        if job.get("status") in _ORPHANABLE:
            update(job["job_id"], status="interrupted", finished_at=now_iso())
            append_log(job["job_id"], job.get("stage"), "warn",
                      "process restarted while this job was in flight; marked interrupted")
            recovered.append(job["job_id"])
    return recovered

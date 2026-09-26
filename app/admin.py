"""Admin UI — key management + corpus import pipeline.

Served on a separate port (``ADMIN_PORT``, default 8081) that is meant to be
reachable from the hosting machine / LAN (not necessarily just localhost),
never via the public proxy. Protected by HTTP Basic auth with
``ADMIN_PASSWORD`` (any username). Every mutating route additionally requires
a same-origin ``Origin`` header against CSRF.

Two areas:

* ``/`` — API-key management (unchanged from before the import pipeline).
* ``/import`` — chunked pack upload, pack library, and import jobs. The wire
  contract for every ``/import/...`` route is docs/IMPORT-API.md; this module
  implements it exactly, calling ``app/uploads.py`` (owned here) for upload
  and pack storage and ``app/import_service.py`` (owned elsewhere — see
  docs/IMPORT-PIPELINE-PLAN.md §6) for job execution.
"""

from __future__ import annotations

import base64
import html
import os
import secrets
from datetime import datetime
from pathlib import Path

from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import HTMLResponse, JSONResponse, RedirectResponse, Response
from starlette.routing import Route

from . import import_service, keystore, uploads

_PASSWORD = os.environ.get("ADMIN_PASSWORD", "")
_PUBLIC_URL = os.environ.get("PUBLIC_URL", "https://qdrant.example.com").rstrip("/")
# Matches app/jobs.py's own ``jobs_dir()`` — both sides must agree on where a
# job's report.md lives. Read directly rather than importing app.jobs: the
# seam this module calls is import_service, and the report file's location is
# a documented disk convention (docs/IMPORT-API.md §4), not a seam function.
_JOBS_DIR = os.environ.get("JOBS_DIR", "/data/jobs")
_STATIC_DIR = Path(__file__).parent / "static"

_STAGES = ["open", "contract", "probe", "preflight", "snapshot", "undo",
           "upsert", "indexes", "titles", "verify", "report"]


def _authorized(request: Request) -> bool:
    if not _PASSWORD:
        return False
    raw = request.headers.get("authorization", "")
    if not raw.lower().startswith("basic "):
        return False
    try:
        _, _, pw = base64.b64decode(raw[6:]).decode().partition(":")
    except Exception:
        return False
    return secrets.compare_digest(pw, _PASSWORD)


def _same_origin(request: Request) -> bool:
    origin = request.headers.get("origin")
    if origin is None:  # non-browser clients (curl) send none
        return True
    return origin.split("://", 1)[-1] == request.headers.get("host")


def _challenge() -> Response:
    msg = "Unauthorized" if _PASSWORD else "ADMIN_PASSWORD is not set — admin UI disabled"
    return Response(msg, status_code=401, headers={"WWW-Authenticate": 'Basic realm="qdrant-mcp admin"'})


def _guard(request: Request, mutating: bool = False) -> Response | None:
    """Common auth/CSRF check for a route. Returns a Response to short-circuit
    with, or None to continue."""
    if not _authorized(request):
        return _challenge()
    if mutating and not _same_origin(request):
        return Response("Bad origin", status_code=403)
    return None


def _operator(request: Request) -> str:
    """Best-effort operator name for job records: the Basic-auth username
    (the password is shared, but nothing stops an operator entering their own
    name as the username), falling back to "admin"."""
    raw = request.headers.get("authorization", "")
    if raw.lower().startswith("basic "):
        try:
            user, _, _ = base64.b64decode(raw[6:]).decode().partition(":")
            if user.strip():
                return user.strip()
        except Exception:
            pass
    return "admin"


def _err(code: str, detail: str, status: int = 400) -> Response:
    return JSONResponse({"error": code, "detail": detail}, status_code=status)


async def _json_body(request: Request) -> dict:
    try:
        data = await request.json()
    except Exception:
        return {}
    return data if isinstance(data, dict) else {}


def _fmt(ts: float | None) -> str:
    return datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M") if ts else "—"


_CSS = """
body{font:15px system-ui,sans-serif;max-width:900px;margin:2rem auto;padding:0 1rem;color:#222}
table{border-collapse:collapse;width:100%}td,th{padding:.45rem;border-bottom:1px solid #ddd;text-align:left}
.revoked{color:#999;text-decoration:line-through}.new{background:#e8f6e8;border:1px solid #9c9;padding:1rem;margin:1rem 0}
code{background:#f3f3f3;padding:.15rem .3rem;word-break:break-all}button{cursor:pointer}form{display:inline}
"""

_IMPORT_CSS = """
.dropzone{border:2px dashed #999;border-radius:8px;padding:2rem;text-align:center;color:#666;cursor:pointer}
.dropzone.drag{border-color:#39c;background:#eef6ff;color:#222}
progress{width:100%;height:1.1rem}
.stagelist{list-style:none;padding:0;display:flex;flex-wrap:wrap;gap:.4rem;margin:.5rem 0}
.stage{padding:.25rem .6rem;border-radius:999px;background:#eee;border:1px solid #ccc;font-size:.85rem}
.stage.running{background:#fff6d6;border-color:#e0c200}
.stage.ok{background:#e8f6e8;border-color:#9c9}
.stage.failed{background:#fde3e3;border-color:#c66}
.stage.skipped{background:#f0f0f0;color:#999;text-decoration:line-through}
.warn{color:#b00;font-weight:600}
#log-view{background:#111;color:#ddd;font:12px/1.4 ui-monospace,SFMono-Regular,Menlo,monospace;
  padding:.75rem;height:220px;overflow:auto;white-space:pre-wrap;border-radius:6px}
.manifest{background:#f7f7f7;border:1px solid #ddd;border-radius:6px;padding:.75rem;margin:.5rem 0}
.manifest table{width:auto}.manifest td,.manifest th{border:none;padding:.15rem .75rem .15rem 0}
.row{display:flex;gap:.75rem;align-items:center;flex-wrap:wrap;margin:.5rem 0}
.hidden{display:none}
small.muted{color:#888}
"""


# ── key management (unchanged) ───────────────────────────────────────────────

def _page(new_key: tuple[str, str] | None = None, error: str = "") -> HTMLResponse:
    rows = []
    sessions = keystore.oauth_sessions()
    for k in keystore.list_keys():
        revoked = k["revoked_at"] is not None
        action = (
            f'<form method="post" action="/keys/{k["id"]}/delete" onsubmit="return confirm(\'Delete?\')">'
            f'<button>Delete</button></form>' if revoked else
            f'<form method="post" action="/keys/{k["id"]}/revoke" onsubmit="return confirm(\'Revoke?\')">'
            f'<button>Revoke</button></form>'
        )
        rows.append(
            f'<tr class="{"revoked" if revoked else ""}"><td>{html.escape(k["name"])}</td>'
            f'<td><code>{html.escape(k["prefix"])}…</code></td><td>{_fmt(k["created_at"])}</td>'
            f'<td>{_fmt(k["last_used_at"])}</td><td>{sessions.get(k["id"], 0) or ""}</td>'
            f'<td>{_fmt(k["revoked_at"])}</td><td>{action}</td></tr>'
        )
    banner = ""
    if new_key:
        name, key = (html.escape(x) for x in new_key)
        banner = (
            f'<div class="new"><b>Key "{name}" created — copy it now, it is not shown again.</b>'
            f'<p>Key: <code>{key}</code></p>'
            f'<p>Connector URL: <code>{_PUBLIC_URL}/mcp</code> (or <code>/sop/mcp</code>, <code>/bible/mcp</code>)</p>'
            f'<p>Header clients (Claude Code, …): <code>Authorization: Bearer {key}</code></p>'
            f'<p>claude.ai custom connector (OAuth): Advanced settings → OAuth Client ID '
            f'<code>bible-sop</code> (any value), OAuth Client Secret <code>{key}</code>. '
            f'Or leave both empty and paste the key on the sign-in page.</p></div>'
        )
    err = f'<p style="color:#b00">{html.escape(error)}</p>' if error else ""
    body = f"""<!doctype html><meta charset="utf-8"><title>Qdrant MCP keys</title><style>{_CSS}</style>
<h1>Qdrant MCP — API keys</h1>
<p><a href="/import">Import corpus →</a></p>
{banner}{err}
<form method="post" action="/keys"><input name="name" placeholder="Key name" required maxlength="80">
<button>Create key</button></form>
<table><tr><th>Name</th><th>Prefix</th><th>Created</th><th>Last used</th><th>OAuth sessions</th><th>Revoked</th><th></th></tr>
{''.join(rows) or '<tr><td colspan=6>No keys yet.</td></tr>'}</table>"""
    return HTMLResponse(body)


async def index(request: Request) -> Response:
    if not _authorized(request):
        return _challenge()
    return _page()


async def create_key(request: Request) -> Response:
    if not _authorized(request):
        return _challenge()
    if not _same_origin(request):
        return Response("Bad origin", status_code=403)
    name = str((await request.form()).get("name", ""))
    try:
        key = keystore.create(name)
    except Exception as exc:  # empty or duplicate name
        msg = "A key with that name already exists." if "UNIQUE" in str(exc) else str(exc)
        return _page(error=msg)
    return _page(new_key=(name.strip(), key))


async def revoke_key(request: Request) -> Response:
    if not _authorized(request):
        return _challenge()
    if not _same_origin(request):
        return Response("Bad origin", status_code=403)
    keystore.revoke(int(request.path_params["key_id"]))
    return RedirectResponse("/", status_code=303)


async def delete_key(request: Request) -> Response:
    if not _authorized(request):
        return _challenge()
    if not _same_origin(request):
        return Response("Bad origin", status_code=403)
    keystore.delete(int(request.path_params["key_id"]))
    return RedirectResponse("/", status_code=303)


# ── import: page + static asset ─────────────────────────────────────────────

def _import_html() -> HTMLResponse:
    stage_lis = "".join(f'<li id="stage-{s}" class="stage pending">{s}</li>' for s in _STAGES)
    body = f"""<!doctype html><meta charset="utf-8"><title>Qdrant MCP — Import</title>
<style>{_CSS}{_IMPORT_CSS}</style>
<h1>Corpus import</h1>
<p><a href="/">← API keys</a></p>

<h2>1. Upload a .sopack</h2>
<div id="dropzone" class="dropzone" tabindex="0">Drop a .sopack file here, or click to choose one.</div>
<input type="file" id="file-input" accept=".sopack" class="hidden">
<div class="row"><progress id="upload-bar" value="0" max="100"></progress>
<span id="upload-label"></span></div>

<div id="manifest-summary"></div>

<div class="row">
  <label><input type="checkbox" id="allow-overwrite"> Allow overwrite</label>
  <label title="Import a new book code even though a live book has the same title and author (a separate volume or edition)"><input type="checkbox" id="allow-same-title"> Allow same title</label>
  <span class="warn">Off by default. Turning this on lets the import replace points that
  already exist in the collection — only enable it if that is exactly what you intend.</span>
</div>
<div class="row">
  <button id="dry-run-btn" disabled>Dry run</button>
  <button id="import-btn" disabled>Import</button>
</div>

<h2>2. Job</h2>
<div id="job-panel">
  <p id="job-status"><small class="muted">No job running.</small></p>
  <progress id="job-bar" value="0" max="100"></progress>
  <ul id="stage-list" class="stagelist">{stage_lis}</ul>
  <div id="log-view"></div>
</div>

<h2>3. Packs</h2>
<table id="packs-table"><thead><tr><th>Pack</th><th>Name</th><th>Profile</th><th>Points</th>
<th>Books</th><th>Size</th><th>Uploaded</th><th>Imported by</th><th></th></tr></thead>
<tbody><tr><td colspan="9"><small class="muted">Loading…</small></td></tr></tbody></table>

<h2>4. Jobs</h2>
<table id="jobs-table"><thead><tr><th>Job</th><th>Pack</th><th>Mode</th><th>Status</th>
<th>Stage</th><th>Created</th><th>Operator</th><th></th></tr></thead>
<tbody><tr><td colspan="8"><small class="muted">Loading…</small></td></tr></tbody></table>

<script src="/static/import.js"></script>"""
    return HTMLResponse(body)


async def import_page(request: Request) -> Response:
    if not _authorized(request):
        return _challenge()
    return _import_html()


async def static_import_js(request: Request) -> Response:
    if not _authorized(request):
        return _challenge()
    path = _STATIC_DIR / "import.js"
    try:
        content = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return Response("not found", status_code=404)
    return Response(content, media_type="text/javascript")


# ── import: uploads ──────────────────────────────────────────────────────────

async def upload_create(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    body = await _json_body(request)
    try:
        rec = uploads.create(body.get("name"), body.get("size"), body.get("sha256"))
    except uploads.UploadError as exc:
        return _err(exc.code, exc.detail, exc.status)
    return JSONResponse({
        "upload_id": rec["upload_id"], "part_size": rec["part_size"], "received": rec["received"],
    })


async def upload_put_part(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    upload_id = request.path_params["upload_id"]
    n = request.path_params["n"]
    try:
        result = await uploads.write_part(upload_id, n, request.stream())
    except uploads.UploadError as exc:
        return _err(exc.code, exc.detail, exc.status)
    return JSONResponse(result)


async def upload_status(request: Request) -> Response:
    if (resp := _guard(request)) is not None:
        return resp
    try:
        return JSONResponse(uploads.status(request.path_params["upload_id"]))
    except uploads.UploadError as exc:
        return _err(exc.code, exc.detail, exc.status)


async def upload_complete(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    try:
        return JSONResponse(uploads.complete(request.path_params["upload_id"]))
    except uploads.UploadError as exc:
        return _err(exc.code, exc.detail, exc.status)


async def upload_delete(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    try:
        uploads.delete(request.path_params["upload_id"])
    except uploads.UploadError as exc:
        return _err(exc.code, exc.detail, exc.status)
    return Response(status_code=204)


# ── import: packs ────────────────────────────────────────────────────────────

def _safe_list_jobs() -> list[dict]:
    try:
        return import_service.list_jobs()
    except Exception:
        return []


async def packs_list(request: Request) -> Response:
    if (resp := _guard(request)) is not None:
        return resp
    packs = uploads.list_packs()
    by_pack: dict[str, list[str]] = {}
    for job in _safe_list_jobs():
        by_pack.setdefault(job.get("pack_id"), []).append(job.get("job_id"))
    for p in packs:
        p["imported_by"] = by_pack.get(p["pack_id"], [])
    return JSONResponse({"packs": packs})


async def packs_get(request: Request) -> Response:
    if (resp := _guard(request)) is not None:
        return resp
    try:
        return JSONResponse(uploads.get_pack(request.path_params["pack_id"]))
    except uploads.UploadError as exc:
        return _err(exc.code, exc.detail, exc.status)


async def packs_delete(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    pack_id = request.path_params["pack_id"]
    for job in _safe_list_jobs():
        if job.get("pack_id") == pack_id and job.get("status") == "running":
            return _err("busy", f"job {job.get('job_id')} is running against this pack", 409)
    try:
        uploads.delete_pack(pack_id)
    except uploads.UploadError as exc:
        return _err(exc.code, exc.detail, exc.status)
    return Response(status_code=204)


# ── import: jobs ─────────────────────────────────────────────────────────────
#
# import_service's job-lookup functions (get_job/cancel/resume/rollback/
# restore_snapshot) all bottom out in app.jobs.load(), which raises
# FileNotFoundError for an unknown job_id rather than returning None — and
# start_job/resume/rollback/restore_snapshot raise import_service.Busy when
# the one global import lock is held by another job. _map_exc turns those
# (plus a plain ValueError for a bad request the seam rejects, e.g. resuming
# a job that isn't 'interrupted') into the {"error","detail"} shape every
# other route in this file already uses.

def _map_exc(exc: Exception) -> tuple[str, str, int]:
    if isinstance(exc, import_service.Busy):
        return "busy", str(exc), 409
    if isinstance(exc, FileNotFoundError):
        return "not_found", str(exc), 404
    if isinstance(exc, ValueError):
        return "bad_request", str(exc), 400
    return "error", str(exc), 500


async def jobs_create(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    body = await _json_body(request)
    pack_id = body.get("pack_id")
    mode = body.get("mode")
    allow_overwrite = bool(body.get("allow_overwrite", False))
    allow_same_title = bool(body.get("allow_same_title", False))
    if not pack_id or mode not in ("dry-run", "apply"):
        return _err("bad_request", "pack_id and mode ('dry-run'|'apply') are required")
    try:
        job_id = import_service.start_job(pack_id, mode, allow_overwrite, _operator(request),
                                          allow_same_title=allow_same_title)
    except Exception as exc:
        code, detail, status = _map_exc(exc)
        return _err(code, detail, status)
    return JSONResponse({"job_id": job_id}, status_code=202)


async def jobs_list(request: Request) -> Response:
    if (resp := _guard(request)) is not None:
        return resp
    return JSONResponse({"jobs": _safe_list_jobs()})


async def jobs_get(request: Request) -> Response:
    if (resp := _guard(request)) is not None:
        return resp
    try:
        job = import_service.get_job(request.path_params["job_id"])
    except FileNotFoundError:
        return _err("not_found", "no such job", 404)
    return JSONResponse(job)


async def jobs_log(request: Request) -> Response:
    if (resp := _guard(request)) is not None:
        return resp
    job_id = request.path_params["job_id"]
    try:
        import_service.get_job(job_id)  # existence check
    except FileNotFoundError:
        return _err("not_found", "no such job", 404)
    try:
        after = int(request.query_params.get("after", 0) or 0)
    except ValueError:
        after = 0
    return JSONResponse(import_service.read_log(job_id, after))


async def jobs_report(request: Request) -> Response:
    if (resp := _guard(request)) is not None:
        return resp
    job_id = request.path_params["job_id"]
    path = Path(_JOBS_DIR) / job_id / "report.md"
    if not path.exists():
        return _err("not_found", "no report for this job", 404)
    return Response(path.read_text(encoding="utf-8"), media_type="text/markdown")


async def _job_action(request: Request, fn, ok_status: str) -> Response:
    job_id = request.path_params["job_id"]
    try:
        fn(job_id)
    except Exception as exc:
        code, detail, status = _map_exc(exc)
        return _err(code, detail, status)
    return JSONResponse({"status": ok_status})


async def jobs_cancel(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    return await _job_action(request, import_service.cancel, "cancelling")


async def jobs_resume(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    return await _job_action(request, import_service.resume, "running")


async def jobs_rollback(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    job_id = request.path_params["job_id"]
    try:
        job = import_service.get_job(job_id)
    except FileNotFoundError:
        return _err("not_found", "no such job", 404)
    body = await _json_body(request)
    if body.get("confirm") != job_id:
        return _err("confirm_mismatch", "confirm must equal the job id", 400)
    try:
        import_service.rollback(job_id)
    except Exception as exc:
        code, detail, status = _map_exc(exc)
        return _err(code, detail, status)
    return JSONResponse({"status": "rolling_back"})


async def jobs_restore_snapshot(request: Request) -> Response:
    if (resp := _guard(request, mutating=True)) is not None:
        return resp
    job_id = request.path_params["job_id"]
    try:
        job = import_service.get_job(job_id)
    except FileNotFoundError:
        return _err("not_found", "no such job", 404)
    snap = (job.get("snapshot") or {}).get("name")
    body = await _json_body(request)
    if not snap or body.get("confirm") != snap:
        return _err("confirm_mismatch", "confirm must equal the snapshot name", 400)
    try:
        import_service.restore_snapshot(job_id)
    except Exception as exc:
        code, detail, status = _map_exc(exc)
        return _err(code, detail, status)
    return JSONResponse({"status": "restoring"})


app = Starlette(routes=[
    Route("/", index),
    Route("/keys", create_key, methods=["POST"]),
    Route("/keys/{key_id:int}/revoke", revoke_key, methods=["POST"]),
    Route("/keys/{key_id:int}/delete", delete_key, methods=["POST"]),

    Route("/import", import_page),
    Route("/static/import.js", static_import_js),

    Route("/import/uploads", upload_create, methods=["POST"]),
    Route("/import/uploads/{upload_id}/parts/{n:int}", upload_put_part, methods=["PUT"]),
    Route("/import/uploads/{upload_id}", upload_status, methods=["GET"]),
    Route("/import/uploads/{upload_id}/complete", upload_complete, methods=["POST"]),
    Route("/import/uploads/{upload_id}", upload_delete, methods=["DELETE"]),

    Route("/import/packs", packs_list, methods=["GET"]),
    Route("/import/packs/{pack_id}", packs_get, methods=["GET"]),
    Route("/import/packs/{pack_id}", packs_delete, methods=["DELETE"]),

    Route("/import/jobs", jobs_create, methods=["POST"]),
    Route("/import/jobs", jobs_list, methods=["GET"]),
    Route("/import/jobs/{job_id}", jobs_get, methods=["GET"]),
    Route("/import/jobs/{job_id}/log", jobs_log, methods=["GET"]),
    Route("/import/jobs/{job_id}/report", jobs_report, methods=["GET"]),
    Route("/import/jobs/{job_id}/cancel", jobs_cancel, methods=["POST"]),
    Route("/import/jobs/{job_id}/resume", jobs_resume, methods=["POST"]),
    Route("/import/jobs/{job_id}/rollback", jobs_rollback, methods=["POST"]),
    Route("/import/jobs/{job_id}/restore-snapshot", jobs_restore_snapshot, methods=["POST"]),
])

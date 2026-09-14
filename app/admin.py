"""Key-management UI — create, revoke and delete API keys.

Served on a separate port (``ADMIN_PORT``, default 8081) that is meant to be
reachable only from the hosting machine / LAN, never via the public proxy.
Protected by HTTP Basic auth with ``ADMIN_PASSWORD`` (any username).
POSTs additionally require a same-origin ``Origin`` header against CSRF.
"""

from __future__ import annotations

import base64
import html
import os
import secrets
from datetime import datetime

from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import HTMLResponse, RedirectResponse, Response
from starlette.routing import Route

from . import keystore

_PASSWORD = os.environ.get("ADMIN_PASSWORD", "")
_PUBLIC_URL = os.environ.get("PUBLIC_URL", "https://qdrant.example.com").rstrip("/")


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


def _fmt(ts: float | None) -> str:
    return datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M") if ts else "—"


_CSS = """
body{font:15px system-ui,sans-serif;max-width:900px;margin:2rem auto;padding:0 1rem;color:#222}
table{border-collapse:collapse;width:100%}td,th{padding:.45rem;border-bottom:1px solid #ddd;text-align:left}
.revoked{color:#999;text-decoration:line-through}.new{background:#e8f6e8;border:1px solid #9c9;padding:1rem;margin:1rem 0}
code{background:#f3f3f3;padding:.15rem .3rem;word-break:break-all}button{cursor:pointer}form{display:inline}
"""


def _page(new_key: tuple[str, str] | None = None, error: str = "") -> HTMLResponse:
    rows = []
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
            f'<td>{_fmt(k["last_used_at"])}</td><td>{_fmt(k["revoked_at"])}</td><td>{action}</td></tr>'
        )
    banner = ""
    if new_key:
        name, key = (html.escape(x) for x in new_key)
        banner = (
            f'<div class="new"><b>Key “{name}” created — copy it now, it is not shown again.</b>'
            f'<p>Key: <code>{key}</code></p>'
            f'<p>Connector URL: <code>{_PUBLIC_URL}/mcp</code> (or <code>/sop/mcp</code>, <code>/bible/mcp</code>)</p>'
            f'<p>Header: <code>Authorization: Bearer {key}</code></p></div>'
        )
    err = f'<p style="color:#b00">{html.escape(error)}</p>' if error else ""
    body = f"""<!doctype html><meta charset="utf-8"><title>Qdrant MCP keys</title><style>{_CSS}</style>
<h1>Qdrant MCP — API keys</h1>{banner}{err}
<form method="post" action="/keys"><input name="name" placeholder="Key name" required maxlength="80">
<button>Create key</button></form>
<table><tr><th>Name</th><th>Prefix</th><th>Created</th><th>Last used</th><th>Revoked</th><th></th></tr>
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


app = Starlette(routes=[
    Route("/", index),
    Route("/keys", create_key, methods=["POST"]),
    Route("/keys/{key_id:int}/revoke", revoke_key, methods=["POST"]),
    Route("/keys/{key_id:int}/delete", delete_key, methods=["POST"]),
])

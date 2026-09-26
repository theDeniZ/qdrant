"""MCP authentication: API-key bearer tokens plus a minimal OAuth 2.1 server.

Two ways in, both ending in the same key store (``keystore.py``):

1. **Header** — ``Authorization: Bearer qd_…`` (Claude Code, other MCP clients,
   claude.ai organisations with request headers). Unchanged.
2. **OAuth** — for claude.ai custom connectors, which only speak OAuth
   (authorization code + PKCE; ``client_credentials`` is not supported by
   claude.ai). The API key is the only credential, presented in one of two
   ways:

   * **as the client secret** — the user enters any client ID (it is not
     checked) and the key as the client secret under "Advanced settings".
     ``/oauth/authorize`` redirects straight back without a page; the key is
     checked at ``/oauth/token``.
   * **on a paste page** — no client credentials entered, so Claude registers
     itself (DCR, ``/oauth/register``) or identifies with a Client ID Metadata
     Document (an ``https://`` client_id). ``/oauth/authorize`` then shows a
     one-field page where the user pastes the key.

   Tokens are opaque (``qda_`` access, ``qdr_`` refresh, rotated on use) and
   bound to the key's row, so revoking the key in the admin UI ends the OAuth
   session on the next request.

Redirect URIs are allow-listed: claude.ai / claude.com's callback, loopback on
any port (Claude Code), plus ``OAUTH_REDIRECT_URIS`` (comma-separated, exact).
"""

from __future__ import annotations

import base64
import hashlib
import html
import json
import os
import secrets
import threading
import time
from urllib.parse import unquote_plus, urlencode, urlsplit

import anyio
from starlette.requests import Request
from starlette.responses import HTMLResponse, JSONResponse, RedirectResponse, Response
from starlette.routing import Route

from . import keystore

ACCESS_TTL_S = int(os.environ.get("OAUTH_ACCESS_TTL", "3600"))
REFRESH_TTL_S = int(os.environ.get("OAUTH_REFRESH_TTL", str(180 * 86400)))
CODE_TTL_S = 300

_HOSTED_CALLBACKS = {"https://claude.ai/api/mcp/auth_callback",
                     "https://claude.com/api/mcp/auth_callback"}
_EXTRA_CALLBACKS = {u.strip() for u in os.environ.get("OAUTH_REDIRECT_URIS", "").split(",")
                    if u.strip()}
DCR_PREFIX = "dcr-"


# ── helpers ──────────────────────────────────────────────────────────────────

def _public_url() -> str:
    return os.environ.get("PUBLIC_URL", "").rstrip("/")


def base_url_from_scope(scope) -> str:
    """Public base URL: ``PUBLIC_URL`` when set, else the request's scheme + Host."""
    if _public_url():
        return _public_url()
    host = next((v.decode("latin-1") for k, v in scope.get("headers", []) if k == b"host"), "")
    return f"{scope.get('scheme', 'https')}://{host}"


def _base(request: Request) -> str:
    return base_url_from_scope(request.scope)


def redirect_allowed(uri: str) -> bool:
    if uri in _HOSTED_CALLBACKS or uri in _EXTRA_CALLBACKS:
        return True
    try:
        parts = urlsplit(uri)
    except ValueError:
        return False
    # RFC 8252 loopback redirect, any port (Claude Code uses an ephemeral one).
    return parts.scheme == "http" and parts.hostname in ("localhost", "127.0.0.1", "::1")


def _pkce_ok(verifier: str, challenge: str) -> bool:
    digest = hashlib.sha256(verifier.encode("ascii", "replace")).digest()
    expected = base64.urlsafe_b64encode(digest).rstrip(b"=").decode()
    return secrets.compare_digest(expected, challenge)


def _asks_for_paste(client_id: str) -> bool:
    """A client Claude registered or identified itself (DCR / CIMD) carries no
    secret, so the key must be pasted. Any other client_id is user-entered and
    is expected to bring the key as its client secret."""
    return client_id.startswith((DCR_PREFIX, "https://", "http://"))


# ── authorization codes (in memory; single process, 5-minute lifetime) ───────

_codes: dict[str, dict] = {}
_codes_lock = threading.Lock()


def _new_code(**grant) -> str:
    code = secrets.token_urlsafe(32)
    now = time.time()
    with _codes_lock:
        for k in [k for k, g in _codes.items() if g["expires"] <= now]:
            del _codes[k]
        _codes[code] = {**grant, "expires": now + CODE_TTL_S}
    return code


def _take_code(code: str) -> dict | None:
    with _codes_lock:
        grant = _codes.pop(code, None)
    return grant if grant and grant["expires"] > time.time() else None


# ── discovery ────────────────────────────────────────────────────────────────

def _protected_resource(path: str):
    async def endpoint(request: Request):
        base = _base(request)
        return JSONResponse({
            "resource": base + path,
            "authorization_servers": [base],
            "bearer_methods_supported": ["header"],
            "resource_name": "bible-sop",
        })
    return endpoint


async def _authorization_server(request: Request):
    base = _base(request)
    return JSONResponse({
        "issuer": base,
        "authorization_endpoint": f"{base}/oauth/authorize",
        "token_endpoint": f"{base}/oauth/token",
        "registration_endpoint": f"{base}/oauth/register",
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic", "none"],
        "client_id_metadata_document_supported": True,
    })


# ── dynamic client registration (stateless: ids are not checked later) ──────

async def _register(request: Request):
    try:
        meta = await request.json()
    except (ValueError, json.JSONDecodeError):
        meta = None
    if not isinstance(meta, dict):
        return _oauth_error("invalid_client_metadata", "body must be a JSON object")
    uris = meta.get("redirect_uris") or []
    if not isinstance(uris, list) or not uris or not all(isinstance(u, str) and redirect_allowed(u)
                                                         for u in uris):
        return _oauth_error("invalid_redirect_uri",
                            "redirect_uris must be the claude.ai callback or loopback URIs")
    return JSONResponse({
        "client_id": DCR_PREFIX + secrets.token_urlsafe(16),
        "client_id_issued_at": int(time.time()),
        "client_name": str(meta.get("client_name", ""))[:200],
        "redirect_uris": uris,
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    }, status_code=201)


# ── authorize ────────────────────────────────────────────────────────────────

_AUTH_PARAMS = ("response_type", "client_id", "redirect_uri", "state",
                "code_challenge", "code_challenge_method", "scope", "resource")

_PAGE_CSS = ("body{font:15px/1.5 system-ui,sans-serif;max-width:30rem;margin:3rem auto;padding:0 1rem;"
             "color:#222}input[type=password]{width:100%;padding:.5rem;font:inherit;box-sizing:border-box}"
             "button{margin-top:.8rem;padding:.5rem 1.2rem;font:inherit}.err{color:#b00}"
             ".host{font-weight:600}.warn{background:#fff4d6;padding:.5rem .7rem;border-radius:4px}")


def _error_page(msg: str, status: int = 400) -> HTMLResponse:
    return _html(f"<h1>Cannot connect</h1><p class=err>{html.escape(msg)}</p>", status)


def _html(body: str, status: int = 200) -> HTMLResponse:
    return HTMLResponse(
        f'<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width">'
        f"<title>Connect bible-sop</title><style>{_PAGE_CSS}</style>{body}",
        status_code=status,
        headers={"X-Frame-Options": "DENY", "Content-Security-Policy": "frame-ancestors 'none'",
                 "Cache-Control": "no-store", "Referrer-Policy": "no-referrer"})


def _redirect_with(redirect_uri: str, **params) -> RedirectResponse:
    sep = "&" if "?" in redirect_uri else "?"
    clean = {k: v for k, v in params.items() if v}
    return RedirectResponse(redirect_uri + sep + urlencode(clean), status_code=302)


def _paste_page(p: dict, error: str = "") -> HTMLResponse:
    host = urlsplit(p["redirect_uri"]).netloc
    loopback = urlsplit(p["redirect_uri"]).hostname in ("localhost", "127.0.0.1", "::1")
    hidden = "".join(f'<input type="hidden" name="{k}" value="{html.escape(p.get(k) or "")}">'
                     for k in _AUTH_PARAMS)
    warn = ('<p class=warn>This sign-in returns to a program on <b>this computer</b>. Continue '
            'only if you started it yourself (for example Claude Code).</p>' if loopback else "")
    err = f"<p class=err>{html.escape(error)}</p>" if error else ""
    return _html(f"""<h1>Connect bible-sop</h1>
<p>Paste your bible-sop API key (<code>qd_…</code>). It will be exchanged for a
sign-in token for <span class=host>{html.escape(host)}</span>; revoking the key
ends that sign-in.</p>{warn}{err}
<form method="post" action="/oauth/authorize">{hidden}
<input type="password" name="api_key" placeholder="qd_…" autocomplete="off" autofocus required>
<button>Connect</button></form>""")


def _check_authorize(p: dict) -> tuple[Response | None, bool]:
    """Validate an authorization request. Returns (error_response, redirect_ok)."""
    uri = p.get("redirect_uri") or ""
    if not p.get("client_id"):
        return _error_page("client_id is missing."), False
    if not redirect_allowed(uri):
        return _error_page(f"Redirect URI not allowed: {uri or '(none)'}"), False
    if p.get("response_type") != "code":
        return _redirect_with(uri, error="unsupported_response_type", state=p.get("state")), True
    if not p.get("code_challenge") or p.get("code_challenge_method") != "S256":
        return _redirect_with(uri, error="invalid_request",
                              error_description="PKCE with S256 is required",
                              state=p.get("state")), True
    return None, True


def _grant_redirect(p: dict, key_id: int | None) -> RedirectResponse:
    code = _new_code(client_id=p["client_id"], redirect_uri=p["redirect_uri"],
                     challenge=p["code_challenge"], key_id=key_id)
    return _redirect_with(p["redirect_uri"], code=code, state=p.get("state"))


async def _authorize(request: Request):
    if request.method == "GET":
        p = {k: request.query_params.get(k) for k in _AUTH_PARAMS}
        err, _ = _check_authorize(p)
        if err is not None:
            return err
        if not _asks_for_paste(p["client_id"]):
            # User-entered client: the key arrives as client_secret at /oauth/token.
            return _grant_redirect(p, key_id=None)
        return _paste_page(p)

    form = await request.form()
    p = {k: str(form.get(k) or "") or None for k in _AUTH_PARAMS}
    err, _ = _check_authorize(p)
    if err is not None:
        return err
    kid = await anyio.to_thread.run_sync(keystore.key_id, str(form.get("api_key") or "").strip())
    if kid is None:
        return _paste_page(p, "That key is not valid or has been revoked.")
    return _grant_redirect(p, key_id=kid)


# ── token ────────────────────────────────────────────────────────────────────

def _oauth_error(error: str, description: str, status: int = 400) -> JSONResponse:
    headers = {"Cache-Control": "no-store"}
    if error == "invalid_client":
        headers["WWW-Authenticate"] = 'Basic realm="bible-sop"'
    return JSONResponse({"error": error, "error_description": description},
                        status_code=status, headers=headers)


def _client_credentials(request: Request, form) -> tuple[str, str]:
    """(client_id, client_secret) from client_secret_basic or client_secret_post."""
    raw = request.headers.get("authorization", "")
    if raw[:6].lower() == "basic ":
        try:
            cid, _, secret = base64.b64decode(raw[6:].strip()).decode().partition(":")
            return unquote_plus(cid), unquote_plus(secret)
        except (ValueError, UnicodeDecodeError):
            return "", ""
    return str(form.get("client_id") or ""), str(form.get("client_secret") or "")


def _tokens(key_id: int, client_id: str) -> JSONResponse:
    access, refresh = keystore.issue_tokens(key_id, client_id, ACCESS_TTL_S, REFRESH_TTL_S)
    return JSONResponse({"access_token": access, "token_type": "Bearer",
                         "expires_in": ACCESS_TTL_S, "refresh_token": refresh},
                        headers={"Cache-Control": "no-store", "Pragma": "no-cache"})


async def _token(request: Request):
    form = await request.form()
    client_id, secret = _client_credentials(request, form)
    grant_type = form.get("grant_type")

    if grant_type == "authorization_code":
        grant = _take_code(str(form.get("code") or ""))
        if grant is None:
            return _oauth_error("invalid_grant", "authorization code is invalid or expired")
        if client_id and client_id != grant["client_id"]:
            return _oauth_error("invalid_grant", "code was issued to another client")
        if str(form.get("redirect_uri") or grant["redirect_uri"]) != grant["redirect_uri"]:
            return _oauth_error("invalid_grant", "redirect_uri does not match")
        if not _pkce_ok(str(form.get("code_verifier") or ""), grant["challenge"]):
            return _oauth_error("invalid_grant", "PKCE verification failed")
        kid = grant["key_id"]
        if kid is None:
            kid = await anyio.to_thread.run_sync(keystore.key_id, secret.strip())
            if kid is None:
                return _oauth_error("invalid_client",
                                    "client_secret must be a valid bible-sop API key (qd_…)", 401)
        return await anyio.to_thread.run_sync(_tokens, kid, grant["client_id"])

    if grant_type == "refresh_token":
        found = await anyio.to_thread.run_sync(keystore.consume_refresh,
                                               str(form.get("refresh_token") or ""))
        if found is None:
            return _oauth_error("invalid_grant", "refresh token is invalid, expired or revoked")
        kid, bound_client = found
        if client_id and client_id != bound_client:
            return _oauth_error("invalid_grant", "refresh token was issued to another client")
        return await anyio.to_thread.run_sync(_tokens, kid, bound_client)

    return _oauth_error("unsupported_grant_type", f"grant_type {grant_type!r} is not supported")


def routes(mcp_paths: list[str]) -> list[Route]:
    """OAuth discovery + endpoints. Protected-resource metadata is served per MCP
    path (RFC 9728 path suffix) and at the bare well-known path for the first."""
    out = [Route("/.well-known/oauth-protected-resource", _protected_resource(mcp_paths[0]))]
    out += [Route(f"/.well-known/oauth-protected-resource{p}", _protected_resource(p))
            for p in mcp_paths]
    out += [
        Route("/.well-known/oauth-authorization-server", _authorization_server),
        Route("/oauth/register", _register, methods=["POST"]),
        Route("/oauth/authorize", _authorize, methods=["GET", "POST"]),
        Route("/oauth/token", _token, methods=["POST"]),
    ]
    return out


# ── bearer middleware ────────────────────────────────────────────────────────

_OPEN_PREFIXES = ("/healthz", "/.well-known/", "/oauth/")


class ApiKeyMiddleware:
    """Pure-ASGI auth for the MCP endpoints: ``Authorization: Bearer <key>`` where
    the key is an API key or an OAuth access token. A 401 points at the
    protected-resource metadata so OAuth clients can discover the sign-in."""

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope["type"] != "http" or scope["path"].startswith(_OPEN_PREFIXES):
            return await self.app(scope, receive, send)

        key = ""
        for name, value in scope.get("headers", []):
            if name == b"authorization":
                raw = value.decode("latin-1")
                if raw[:7].lower() == "bearer ":
                    key = raw[7:].strip()
                break

        client = await anyio.to_thread.run_sync(keystore.verify, key)
        if client is None:
            path = scope["path"].rstrip("/") or "/"
            meta = f"{base_url_from_scope(scope)}/.well-known/oauth-protected-resource{path}"
            body = b'{"error":"unauthorized","detail":"Missing, invalid or revoked API key"}'
            challenge = f'Bearer realm="bible-sop", resource_metadata="{meta}"'
            await send({"type": "http.response.start", "status": 401, "headers": [
                (b"content-type", b"application/json"),
                (b"www-authenticate", challenge.encode("latin-1")),
                (b"content-length", str(len(body)).encode())]})
            await send({"type": "http.response.body", "body": body})
            return
        scope.setdefault("state", {})["api_key_name"] = client
        await self.app(scope, receive, send)

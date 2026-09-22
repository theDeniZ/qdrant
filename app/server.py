"""Networked, read-only Qdrant MCP server (SoP + Bible tools).

Endpoints (Streamable HTTP, stateless, JSON responses)::

    /mcp        all six tools  — for the claude.ai custom connector
    /sop/mcp    sop_lookup, sop_book_paragraphs, sop_list_books   (local "sop-tools")
    /bible/mcp  bible_search, bible_lookup, bible_list_translations (local "bible-tools")
    /healthz    unauthenticated liveness probe

Every MCP request needs a valid, unrevoked API key as
``Authorization: Bearer <key>``. Keys are managed in the admin UI
(``admin.py``) on a separate, non-public port.

Only the read tools of ``sop_tools`` / ``bible_tools`` are registered; the
server talks to Qdrant with search/scroll/GET calls only, and every tool is
annotated ``readOnlyHint`` so clients need not ask before calling it.
"""

from __future__ import annotations

import asyncio
import contextlib
import functools
import logging
import os
import threading

import anyio
import uvicorn
from mcp.server.fastmcp import FastMCP
from mcp.server.transport_security import TransportSecuritySettings
from mcp.types import ToolAnnotations
from starlette.applications import Starlette
from starlette.responses import PlainTextResponse
from starlette.routing import Route

from . import admin, bible_tools, keystore, seed, sop_tools

log = logging.getLogger("qdrant-mcp")

SOP_TOOLS = [sop_tools.sop_lookup, sop_tools.sop_book_paragraphs, sop_tools.sop_list_books]
BIBLE_TOOLS = [bible_tools.bible_search, bible_tools.bible_lookup,
               bible_tools.bible_list_translations]

# One shared embedder (the e5-large model is ~2 GB in RAM); lock the lazy load
# because tools now run in worker threads.
_embed_lock = threading.Lock()
_load_embedder = sop_tools._get_embedder


def _shared_embedder():
    with _embed_lock:
        return _load_embedder()


sop_tools._get_embedder = _shared_embedder
bible_tools._get_embedder = _shared_embedder


def _threaded(fn):
    """Async wrapper so blocking Qdrant/embedding calls don't stall the loop."""
    @functools.wraps(fn)
    async def wrapper(*args, **kwargs):
        return await anyio.to_thread.run_sync(functools.partial(fn, *args, **kwargs))
    return wrapper


def _transport_security() -> TransportSecuritySettings:
    hosts = [h.strip() for h in os.environ.get("MCP_ALLOWED_HOSTS", "").split(",") if h.strip()]
    if not hosts:
        return TransportSecuritySettings(enable_dns_rebinding_protection=False)
    return TransportSecuritySettings(
        enable_dns_rebinding_protection=True, allowed_hosts=hosts,
        allowed_origins=[f"https://{h}" for h in hosts] + [f"http://{h}" for h in hosts])


_READ_ONLY = ToolAnnotations(readOnlyHint=True, destructiveHint=False,
                             idempotentHint=True, openWorldHint=False)


def _build_mcp(name: str, path: str, tools, instructions: str) -> FastMCP:
    mcp = FastMCP(name, instructions=instructions, stateless_http=True, json_response=True,
                  streamable_http_path=path, transport_security=_transport_security())
    for fn in tools:
        mcp.add_tool(_threaded(fn), annotations=_READ_ONLY)
    return mcp


_INSTR_SOP = ("Read-only Spirit of Prophecy (Ellen G. White) paragraph lookup against a "
              "multilingual Qdrant index. Query in the language you want results in.")
_INSTR_BIBLE = ("Read-only Bible verse lookup and semantic search across the indexed "
                "translations. OSIS keys are KJV-numbered; mind versification differences.")

SERVERS = [
    _build_mcp("qdrant-tools", "/mcp", SOP_TOOLS + BIBLE_TOOLS, _INSTR_SOP + " " + _INSTR_BIBLE),
    _build_mcp("sop-tools", "/sop/mcp", SOP_TOOLS, _INSTR_SOP),
    _build_mcp("bible-tools", "/bible/mcp", BIBLE_TOOLS, _INSTR_BIBLE),
]


@contextlib.asynccontextmanager
async def _lifespan(app):
    keystore.init()
    # The persistent volume outlives the image, so the title table and the
    # import pipeline's working directories are established here rather than in
    # the Dockerfile — an existing named volume never receives a rebuilt image's
    # new directories. See app/seed.py.
    seed.ensure_data_dirs()
    seed.seed_book_titles()
    async with contextlib.AsyncExitStack() as stack:
        for s in SERVERS:
            await stack.enter_async_context(s.session_manager.run())
        if os.environ.get("WARM_EMBEDDER", "1") == "1":
            threading.Thread(target=_shared_embedder, daemon=True).start()
        yield


async def _healthz(request):
    return PlainTextResponse("ok")


def _build_mcp_app() -> Starlette:
    routes = [Route("/healthz", _healthz)]
    for s in SERVERS:
        routes.extend(s.streamable_http_app().routes)
    return Starlette(routes=routes, lifespan=_lifespan)


class ApiKeyMiddleware:
    """Pure-ASGI auth: ``Authorization: Bearer <key>`` checked against the key store."""

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope["type"] != "http" or scope["path"] == "/healthz":
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
            body = b'{"error":"unauthorized","detail":"Missing, invalid or revoked API key"}'
            await send({"type": "http.response.start", "status": 401, "headers": [
                (b"content-type", b"application/json"),
                (b"www-authenticate", b'Bearer realm="qdrant-mcp"'),
                (b"content-length", str(len(body)).encode())]})
            await send({"type": "http.response.body", "body": body})
            return
        scope.setdefault("state", {})["api_key_name"] = client
        await self.app(scope, receive, send)


app = ApiKeyMiddleware(_build_mcp_app())


async def _serve() -> None:
    keystore.init()
    mcp_port = int(os.environ.get("MCP_PORT", "8765"))
    admin_port = int(os.environ.get("ADMIN_PORT", "8081"))
    if not os.environ.get("ADMIN_PASSWORD"):
        log.warning("ADMIN_PASSWORD not set — admin UI will reject every request")
    servers = [
        uvicorn.Server(uvicorn.Config(app, host="0.0.0.0", port=mcp_port,
                                      proxy_headers=True, forwarded_allow_ips="*")),
        uvicorn.Server(uvicorn.Config(admin.app, host="0.0.0.0", port=admin_port)),
    ]
    await asyncio.gather(*(s.serve() for s in servers))


def main() -> None:
    logging.basicConfig(level=logging.INFO)
    asyncio.run(_serve())


if __name__ == "__main__":
    main()

"""Networked, read-only Qdrant MCP server (SoP + Bible tools).

Endpoints (Streamable HTTP, stateless, JSON responses)::

    /mcp        all nine tools — for the claude.ai custom connector
    /sop/mcp    the six sop_* tools   (drop-in for a local "sop-tools")
    /bible/mcp  the three bible_* tools (drop-in for a local "bible-tools")
    /healthz    unauthenticated liveness probe
    /.well-known/oauth-*, /oauth/*   OAuth sign-in (``auth.py``)

Every MCP request needs a valid, unrevoked API key as
``Authorization: Bearer <key>``, or an OAuth access token obtained with one
(``auth.py``: the key as client secret, or pasted on the sign-in page). Keys
are managed in the admin UI (``admin.py``) on a separate, non-public port.

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

from . import admin, auth, bible_tools, keystore, seed, sop_tools

log = logging.getLogger("qdrant-mcp")

SOP_TOOLS = [sop_tools.sop_lookup, sop_tools.sop_book_paragraphs, sop_tools.sop_list_books,
             sop_tools.sop_context, sop_tools.sop_parallel, sop_tools.sop_by_bible_ref]
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

MCP_PATHS = [
    ("/mcp", ("qdrant-tools", SOP_TOOLS + BIBLE_TOOLS, _INSTR_SOP + " " + _INSTR_BIBLE)),
    ("/sop/mcp", ("sop-tools", SOP_TOOLS, _INSTR_SOP)),
    ("/bible/mcp", ("bible-tools", BIBLE_TOOLS, _INSTR_BIBLE)),
]
SERVERS = [_build_mcp(name, path, tools, instr) for path, (name, tools, instr) in MCP_PATHS]


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
    routes = [Route("/healthz", _healthz)] + auth.routes([p for p, _ in MCP_PATHS])
    for s in SERVERS:
        routes.extend(s.streamable_http_app().routes)
    return Starlette(routes=routes, lifespan=_lifespan)


app = auth.ApiKeyMiddleware(_build_mcp_app())


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

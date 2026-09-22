# bible-sop

A read-only corpus of **Bible verses** (11 translations) and **Ellen G. White**
writings (12 languages), offered in three parts:

1. **MCP server** (`app/`, Docker): six lookup tools over Streamable HTTP, backed
   by an existing Qdrant instance, with per-client API keys you create and revoke
   in a small admin UI.
2. **Plugin** (`plugin/`): the connector plus three skills: `corpus-lookup`,
   `quote-verify`, `quote-translate`.
3. **Project setup** (`docs/`): instructions for new projects and for adding this
   to existing ones (claude.ai and Claude Code).

```
.
├── app/                    MCP server + admin UI (Python, Starlette, FastMCP)
│   └── data/sop_books.json SoP book code → title tables (scripts/export_book_titles.py)
├── sopack/                 CLI for extracting and packing corpus data (Python 3.11)
├── Dockerfile, docker-compose.yml, requirements.txt
├── homebrew/               sopack.rb formula + tap setup guide
├── .github/workflows/      release-sopack.yml (GitHub Actions)
├── .claude-plugin/         marketplace.json → ./plugin
├── plugin/                 the bible-sop plugin (.mcp.json + skills/)
├── references/             CANONICAL skill references; copied into each skill by the build
├── scripts/                build_plugin.py, export_book_titles.py, merge_corpus_titles.py
└── docs/                   INTEGRATION.md, PROJECT-INSTRUCTIONS.md, IMPORT-PIPELINE.md, IMPORT-API.md
```

## Tools

| Tool | Does |
|---|---|
| `bible_lookup(ref, bible?, numbering?)` | Verses by OSIS ref: verse, range, chapter or list; KJV→edition remapping |
| `bible_search(query, bible?, limit?)` | Semantic verse search |
| `bible_list_translations()` | Indexed translations + verse counts |
| `sop_lookup(query \| queries, codes?, lang?)` | Semantic paragraph search, single or batched |
| `sop_book_paragraphs(book_code, page_from, page_to?, lang?)` | Exact page ranges (paged) |
| `sop_list_books(lang?, search?)` | Languages, or books with codes and titles |

| Endpoint | Tools |
|---|---|
| `/mcp` | all six |
| `/sop/mcp` | the three `sop_*` tools (drop-in for a `sop-tools` server) |
| `/bible/mcp` | the three `bible_*` tools (drop-in for a `bible-tools` server) |
| `/healthz` | liveness, no auth |

Only read calls reach Qdrant (`points/search[/batch]`, `points/scroll`, `facet`),
and every tool carries MCP's `readOnlyHint`. The server expects these payload
indexes: `bible`, `osis` on `bibles`; `lang`, `book_code`, `page` on `sop`. The
sdarm build scripts create them (`--indexes-only` for an existing collection).

## Deploy

1. Edit `docker-compose.yml`: `QDRANT_URL`, `ADMIN_PASSWORD`, `PUBLIC_URL`, and the
   Traefik `Host(...)` rule, entrypoint and certresolver.
2. `docker compose up -d --build`. On first start the e5-large model (~2 GB)
   downloads into the `qdrant-mcp-data` volume, and the container needs about
   2.5 GB of RAM.
3. Open the admin UI on the host at `http://127.0.0.1:8081` (any username +
   `ADMIN_PASSWORD`), or tunnel it with `ssh -L 8081:127.0.0.1:8081 <host>`.

Update: `git pull && docker compose up -d --build`. Keys and the model cache
live in the volume and survive rebuilds.

## Keys

In the admin UI you can create a named key (shown **once**), revoke it (takes
effect on the next request), or delete a revoked key. Keys are stored as SHA-256
hashes in `/data/keys.db`, and "Last used" shows which consumers are active.
Clients send `Authorization: Bearer qd_…`.

## Plugin and project setup

```bash
python3 scripts/build_plugin.py          # sync references → skills, validate, zip to dist/
python3 scripts/build_plugin.py --check  # CI: fail if skills are stale or invalid
```

Edit `references/*.md` and `plugin/skills/*/SKILL.md`, never the copied
`plugin/skills/*/references/`. Then run the build and commit `plugin/`
(`dist/` is ignored).

- Connecting claude.ai, Claude Code or another client, and adding the skills to
  a new or existing project (e.g. sbl): **[docs/INTEGRATION.md](docs/INTEGRATION.md)**
- Instruction blocks to paste: **[docs/PROJECT-INSTRUCTIONS.md](docs/PROJECT-INSTRUCTIONS.md)**

## Corpus import

Add books to the SoP or Bible collections using the **sopack** CLI, which runs on
the Mac (where fastembed is available). The workflow is:

1. Extract a book (EPUB, Markdown, JSON) into a reviewable intermediate format
2. Pack it with embeddings into a `.sopack` file
3. Upload and import it via the admin UI with a dry-run proof and rollback safety

**References:**
- **[docs/IMPORT-PIPELINE-PLAN.md](docs/IMPORT-PIPELINE-PLAN.md)** — full design, stages,
  canary probes, and rollback / restore
- **[docs/IMPORT-API.md](docs/IMPORT-API.md)** — the server's import HTTP routes
- **sopack CLI** — install via `brew install theDeniZ/tap/sopack` or from a GitHub Release
  (see **[homebrew/README.md](homebrew/README.md)** for details)

## Development

```bash
pip install -r requirements.txt
QDRANT_URL=http://10.10.10.10:6333 ADMIN_PASSWORD=dev KEYS_DB=./keys.db \
  MCP_PORT=8765 ADMIN_PORT=8081 python -m app.server
```

`app/sop_tools.py` and `app/bible_tools.py` also run as stdio MCP servers
(`python app/sop_tools.py`). The sdarm workspace keeps identical copies in
`translator/*_tools_mcp.py`; change both. `app/versification.py` is a copy of
sdarm's `generator/src/sdarm/core/bible/versification.py`.

When the SoP corpus gains books, regenerate the title table:
`python scripts/export_book_titles.py <sdarm>/generator/data/sop`.

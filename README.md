# bible-sop

A read-only corpus of **Bible verses** (11 translations) and **Ellen G. White**
writings (12 languages), offered in three parts:

1. **MCP server** (`app/`, Docker): nine lookup tools over Streamable HTTP, backed
   by an existing Qdrant instance, with per-client API keys you create and revoke
   in a small admin UI. The keys work as a bearer header or through OAuth
   (claude.ai custom connectors).
2. **Plugin** (`plugin/`): the connector plus four skills for users of the
   corpus: `corpus-lookup`, `corpus-research`, `quote-verify`, `quote-translate`.
3. **Project setup** (`docs/`): instructions for new projects and for adding this
   to existing ones (claude.ai and Claude Code).

```
.
├── app/                    MCP server + admin UI (Python, Starlette, FastMCP)
│   └── data/sop_books.json SoP book code → title tables (scripts/export_book_titles.py)
├── sopack/                 CLI for extracting and packing corpus data (Python 3.14)
├── Dockerfile, docker-compose.yml, requirements.txt
├── Formula/                sopack.rb — this repo is the Homebrew tap (Formula/README.md)
├── .github/workflows/      release-sopack.yml (GitHub Actions)
├── .claude-plugin/         marketplace.json → ./plugin
├── plugin/                 the bible-sop plugin (.mcp.json + skills/)
├── .claude/skills/         corpus-prep — maintainer-only sopack skill (not in the plugin)
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
| `sop_context(book_code, para_key, lang?, before?, after?)` | Paragraphs around a known paragraph |
| `sop_parallel(book_code, para_key, lang?, target_lang?)` | Published de↔en counterpart of a paragraph |
| `sop_by_bible_ref(osis, lang?, limit?)` | Paragraphs citing a Bible verse |

| Endpoint | Tools |
|---|---|
| `/mcp` | all nine |
| `/sop/mcp` | the six `sop_*` tools (drop-in for a `sop-tools` server) |
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

**OAuth** (`app/auth.py`), for claude.ai, whose custom connectors only speak
OAuth: the server is its own authorization server (`/.well-known/oauth-*`,
`/oauth/{authorize,token,register}`, authorization code + PKCE). The key is the
only credential. Either enter it as the connector's **OAuth Client Secret** (any
Client ID), or leave the credentials empty and paste the key on the sign-in page.
Issued tokens (`qda_…`, 1 h; refresh `qdr_…`, 180 days, rotated) stay bound to
the key, so revoking the key ends them. The admin UI counts live OAuth sessions
per key. `PUBLIC_URL` must be the exact public origin, because discovery
metadata is built from it. Optional: `OAUTH_REDIRECT_URIS` (extra allowed
callbacks), `OAUTH_ACCESS_TTL`, `OAUTH_REFRESH_TTL` (seconds).

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

Add books to the SoP or Bible collections using the **sopack** CLI
(`sopack-rs/`, a single Rust binary — macOS or Linux, no Python needed to
build or verify a pack). The workflow is:

1. Extract a book (EPUB, Markdown, text, sop_json) into a reviewable
   intermediate `book.json`, with the metadata you supply (flags or a
   `<source>.meta.toml`)
2. Pack it with embeddings into a `.sopack` file, then verify it offline
3. Upload and import it via the admin UI with a dry-run proof and rollback
   safety. The importer — not the CLI — decides book identity from the
   store: a taken code, or a title already live under another code, is
   refused at `preflight` (see [docs/IMPORT-API.md](docs/IMPORT-API.md) §3)

Steps 1–2 (metadata research through a verified pack) are the
**`corpus-prep`** project skill (`.claude/skills/corpus-prep/`, maintainers only,
not part of the public plugin) — it drives `sopack extract/inspect/pack/
verify` end to end and hands the finished pack to you; it never uploads
or imports (step 3 is always a manual, confirmed action).

**References:**
- **[docs/IMPORT-PIPELINE-PLAN.md](docs/IMPORT-PIPELINE-PLAN.md)** — full design, stages,
  canary probes, and rollback / restore
- **[docs/IMPORT-API.md](docs/IMPORT-API.md)** — the server's import HTTP routes
- **sopack CLI** — `brew tap theDeniZ/qdrant https://github.com/theDeniZ/qdrant && brew install
  theDeniZ/qdrant/sopack`; releasing and tap details in **[Formula/README.md](Formula/README.md)**

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

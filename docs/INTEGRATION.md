# Integrating bible-sop

Three pieces, each optional on its own:

| Piece | What it gives | Where |
|---|---|---|
| **Connector** (MCP server) | The nine read-only tools | `https://<host>/mcp` + API key |
| **Skills** (plugin) | How to use the tools correctly | `plugin/` (or `dist/*.zip`) |
| **Instructions** | When to use which skill; the rules | [PROJECT-INSTRUCTIONS.md](PROJECT-INSTRUCTIONS.md) |

The tools work without the skills, but the skills carry the knowledge that
prevents wrong quotations: numbering quirks, score calibration, per-language
pagination, and compilations. Install both.

## 0. Get an API key

Open the admin UI on the host (`http://127.0.0.1:8081`, see
[../README.md](../README.md)), create a key named after its consumer
(`claude-ai-denis`, `sdarm-workspace`, `sbl-project`), and copy it; it is shown
once. Give each consumer its own key so you can revoke one without touching the
others.

Endpoints on the same host and key:

| Path | Tools | Use for |
|---|---|---|
| `/mcp` | all nine | new setups, claude.ai |
| `/sop/mcp` | the six `sop_*` tools | drop-in for an existing `sop-tools` server |
| `/bible/mcp` | `bible_search`, `bible_lookup`, `bible_list_translations` | drop-in for an existing `bible-tools` server |

## 1. Claude.ai

### Connector

claude.ai custom connectors sign in with OAuth. Request headers are a beta that
only some organisations have. The server runs a small OAuth endpoint whose only
credential is the API key, so any plan can connect.

Settings → **Connectors** → **Add custom connector**:

- Name: `bible-sop`
- URL: `https://qdrant.thedeniz.dev/mcp`
- **Advanced settings** (optional):
  - OAuth Client ID: anything, e.g. `bible-sop` (it is not checked)
  - OAuth Client Secret: your key `qd_…`

  Then **Connect**. The sign-in completes without a page, because the key is
  checked when Claude exchanges the code.
- **Or** leave Advanced settings empty and click **Connect**. A small bible-sop
  page opens where you paste the key once.

Either way, Claude stores a token tied to that key, and revoking the key in the
admin UI disconnects it. Organisations that do have **Request headers** can
still use `Authorization` = `Bearer qd_…` instead.

Enable it in the chat or project where you want the tools. Every tool is marked
read-only, so claude.ai does not ask for approval per call.

### Skills

Build first: `python3 scripts/build_plugin.py`. Then either:

- **Plugin:** add this repository as a plugin marketplace (it has
  `.claude-plugin/marketplace.json`) or upload `dist/bible-sop.zip`, depending on
  what your plan offers; or
- **Individual skills:** Settings → Capabilities → Skills → upload
  `dist/skills/corpus-lookup.zip`, `corpus-research.zip`, `quote-verify.zip`,
  `quote-translate.zip`.

The plugin's `.mcp.json` is for Claude Code. On claude.ai the connector above is
what supplies the tools.

### New project

Create a Project, paste **Block A** of
[PROJECT-INSTRUCTIONS.md](PROJECT-INSTRUCTIONS.md) into its instructions, and
enable the `bible-sop` connector and skills.

### Existing project (e.g. `sbl`)

1. Enable the `bible-sop` connector next to the project's own connector.
2. Install the `bible-sop` skills next to the project's skills.
3. Append **Block B** of [PROJECT-INSTRUCTIONS.md](PROJECT-INSTRUCTIONS.md) to the
   project instructions. (The sbl project instructions in
   `plugins/PROJECT-INSTRUCTIONS.md` already contain it; skip this step there.)

For **sbl** specifically: its `lesson-translate`, `plan-*` and `sermon-prep`
skills call "the connected Bible and SoP lookup tools"
(`mcp__sop-tools__*` / `mcp__bible-tools__*` in the manuals). Block B maps those
names to this connector. sbl's own skill stays in charge of each workflow (the
Qdrant-before-DeepL gate, notes format, style sheets), and bible-sop supplies the
lookup method underneath. Add one line to the sbl project's "Choose the persona"
table if you want the verification skill reachable there:

```
| check the quotations in a plan, lesson or sermon draft | `quote-verify` (bible-sop) | — |
```

## 2. Claude Code

### Connector only

Project `.mcp.json`. The key comes from the environment, so do not commit it:

```json
{
  "mcpServers": {
    "bible-sop": {
      "type": "http",
      "url": "https://qdrant.thedeniz.dev/mcp",
      "headers": { "Authorization": "Bearer ${BIBLE_SOP_MCP_KEY}" }
    }
  }
}
```

Tool names become `mcp__bible-sop__bible_lookup`, and so on. Without the
`headers` block, Claude Code signs in through the OAuth page instead (`/mcp` →
Authenticate), where you paste the key once.

**Keeping existing tool names** (the sdarm workspace, whose manuals and
`allowed-tools` say `mcp__sop-tools__*` / `mcp__bible-tools__*`): register the
two drop-in endpoints under the old server names instead:

```json
"sop-tools":   { "type": "http", "url": "https://qdrant.thedeniz.dev/sop/mcp",
                 "headers": { "Authorization": "Bearer ${BIBLE_SOP_MCP_KEY}" } },
"bible-tools": { "type": "http", "url": "https://qdrant.thedeniz.dev/bible/mcp",
                 "headers": { "Authorization": "Bearer ${BIBLE_SOP_MCP_KEY}" } }
```

### Plugin (skills + connector)

```
/plugin marketplace add <git URL of this repo>
/plugin install bible-sop@bible-sop
```

The plugin brings its own `bible-sop` server
(`${BIBLE_SOP_MCP_URL:-https://qdrant.thedeniz.dev/mcp}` with
`${BIBLE_SOP_MCP_KEY}`). Export `BIBLE_SOP_MCP_KEY` before starting Claude Code.
If the project already registers the drop-in endpoints under `sop-tools` /
`bible-tools`, you get both sets of names. That is harmless, but you can disable
the plugin's server in `/mcp`.

### CLAUDE.md

Append **Block B** of [PROJECT-INSTRUCTIONS.md](PROJECT-INSTRUCTIONS.md) (or use
Block A for a project that is only about quotations).

## 3. Other MCP clients

Any client that speaks Streamable HTTP and can send a header works:
URL `https://<host>/mcp`, header `Authorization: Bearer <key>`. Without the
skills, give the model at least [references/TOOLS.md](../references/TOOLS.md)
and the rules from Block A.

## Troubleshooting

| Symptom | Cause |
|---|---|
| No response / Traefik 404 or 502, and the container log shows **no** requests except `/healthz` | Container is not on Traefik's Docker network. Set `TRAEFIK_NETWORK` (see `docker-compose.yml`) and `docker compose up -d` |
| `401 unauthorized` | Key missing, mistyped, or revoked. Check "Last used" in the admin UI |
| claude.ai: "Authorization with the MCP server failed" | The client secret is not a valid, unrevoked key. Or `PUBLIC_URL` is not the exact public origin, so discovery points elsewhere |
| Sign-in page says "Redirect URI not allowed" | A client other than claude.ai / Claude Code. Add its callback to `OAUTH_REDIRECT_URIS` |
| `421 Invalid Host header` | `MCP_ALLOWED_HOSTS` is set but does not include the public host |
| First `sop_lookup` / `bible_search` slow | Embedding model still loading after a restart (`WARM_EMBEDDER=1` preloads it) |
| Tools listed but lookups time out | Container cannot reach `QDRANT_URL` |

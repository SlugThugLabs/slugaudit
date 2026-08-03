# Connecting SlugAudit to Codex

## One-line setup

```bash
slugaudit-mcp-rust connect codex
```

That registers the `slugaudit` stdio MCP server globally in
`~/.codex/config.toml`. Verify:

```bash
codex mcp list
# slugaudit  /path/to/slugaudit-mcp-rust  enabled
```

Codex has no user/project scope distinction — it always writes to the
global config, which is what you want for a per-project server like
SlugAudit.

## Enable a project

Connecting the MCP server makes the tools available; enabling a project
indexes it:

```bash
slugaudit-mcp-rust enable /path/to/your-project
```

This creates `.slugaudit/` inside the project and runs the first import.
After that, Codex can query the project's evidence through the four
SlugAudit tools (`query`, `report`, `structure`, `finding`).

## Re-running `connect`

Safe to re-run — it removes any existing `slugaudit` entry and re-adds
it, so upgrading the binary and re-running `connect codex` always points
at the current executable.

## Manual alternative

```bash
codex mcp add slugaudit -- $(which slugaudit-mcp-rust)
```

## PostgreSQL / shared index

To serve one shared PostgreSQL index across multiple developers or
machines instead of the default per-project SQLite:

```bash
codex mcp add slugaudit \
  --env SLUGAUDIT_CONFIG=/path/to/config.toml \
  -- $(which slugaudit-mcp-rust)
```

See `config.toml.example` in the repo for the format. Most users don't
need this.

## Troubleshooting

- **`codex` not found** — install the Codex CLI first.
- **Tools don't appear after `connect`** — restart Codex. Already-running
  sessions won't pick up a newly registered MCP server.
- **"project not enabled"** — run `slugaudit-mcp-rust enable <path>`.
- **Codex shows the server as "Unsupported"** — this is a Codex display
  quirk for stdio servers that don't declare OAuth metadata. The server
  is still functional; verify with `codex mcp list` and try a `query`
  tool call.

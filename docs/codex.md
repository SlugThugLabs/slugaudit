# Connecting SlugAudit to Codex

## One-line setup

```bash
slugaudit-mcp connect codex
```

That registers the `slugaudit` stdio MCP server globally in
`~/.codex/config.toml`. Verify:

```bash
codex mcp list
# slugaudit  /path/to/slugaudit-mcp  enabled
```

Codex has no user/project scope distinction — it always writes to the
global config, which is what you want for a per-project server like
SlugAudit.

## Enable a project

Connecting the MCP server makes the tools available; enabling a project
indexes it. Have the agent call the `project_control` tool with
`action = "on"` (optionally with a project path). This creates the
activation marker and SQLite database under `.planning/slugaudit/` inside
the project and runs the first import. After that, Codex can query the
project's evidence through all six SlugAudit tools (`query`, `report`,
`structure`, `finding`, `project_control`, `health`).

## Re-running `connect`

Safe to re-run — it removes any existing `slugaudit` entry and re-adds
it, so upgrading the binary and re-running `connect codex` always points
at the current executable.

## Manual alternative

```bash
codex mcp add slugaudit -- $(which slugaudit-mcp)
```

## Troubleshooting

- **`codex` not found** — install the Codex CLI first.
- **Tools don't appear after `connect`** — restart Codex. Already-running
  sessions won't pick up a newly registered MCP server.
- **"project not enabled"** — have the agent call `project_control` with
  `action = "on"` for the project.
- **Codex shows the server as "Unsupported"** — this is a Codex display
  quirk for stdio servers that don't declare OAuth metadata. The server
  is still functional; verify with `codex mcp list` and try a `query`
  tool call.

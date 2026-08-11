# Connecting SlugAudit to Grok

## One-line setup

```bash
slugaudit-mcp connect grok
```

That registers the `slugaudit` stdio MCP server at user scope
(`~/.grok/config.toml`), available in every project. Verify:

```bash
grok mcp list --scope user
# slugaudit: /path/to/slugaudit-mcp - connected
```

In an active Grok session, run `/mcps` (press `r` to refresh if it's
already running) to confirm SlugAudit's tools are loaded.

## Scope options

`connect grok` defaults to `--scope user` (global). If you want a
project-scoped registration instead — only available when working in that
directory — use the manual form:

```bash
grok mcp add slugaudit --scope project -- $(which slugaudit-mcp)
```

For nearly all users, the user-scope default from `connect grok` is what
you want.

## Enable a project

Connecting the MCP server makes the tools available; enabling a project
indexes it. Have the agent call the `project_control` tool with
`action = "on"` (optionally with a project path). This creates the
activation marker and SQLite database under `.planning/slugaudit/` inside
the project and runs the first import. After that, Grok can query the
project's evidence through all six SlugAudit tools (`query`, `report`,
`structure`, `finding`, `project_control`, `health`).

## Re-running `connect`

Safe to re-run — it removes any existing `slugaudit` entry and re-adds
it, so upgrading the binary and re-running `connect grok` always points
at the current executable.

## Manual alternative

```bash
grok mcp add slugaudit --scope user -- $(which slugaudit-mcp)
```

## Troubleshooting

- **`grok` not found** — install the Grok CLI first.
- **Tools don't appear after `connect`** — restart Grok, or run `/mcps`
  and press `r` to refresh the MCP server list.
- **"project not enabled"** — have the agent call `project_control` with
  `action = "on"` for the project.
- **Diagnose connection issues:** `grok mcp doctor slugaudit`

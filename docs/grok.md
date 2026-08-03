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
indexes it:

```bash
slugaudit-mcp enable /path/to/your-project
```

This creates `.slugaudit/` inside the project and runs the first import.
After that, Grok can query the project's evidence through the four
SlugAudit tools (`query`, `report`, `structure`, `finding`).

## Re-running `connect`

Safe to re-run — it removes any existing `slugaudit` entry and re-adds
it, so upgrading the binary and re-running `connect grok` always points
at the current executable.

## Manual alternative

```bash
grok mcp add slugaudit --scope user -- $(which slugaudit-mcp)
```

## PostgreSQL / shared index

To serve one shared PostgreSQL index across multiple developers or
machines instead of the default per-project SQLite:

```bash
grok mcp add slugaudit --scope user \
  -e "SLUGAUDIT_CONFIG=/path/to/config.toml" \
  -- $(which slugaudit-mcp)
```

See `config.toml.example` in the repo for the format. Most users don't
need this.

## Troubleshooting

- **`grok` not found** — install the Grok CLI first.
- **Tools don't appear after `connect`** — restart Grok, or run `/mcps`
  and press `r` to refresh the MCP server list.
- **"project not enabled"** — run `slugaudit-mcp enable <path>`.
- **Diagnose connection issues:** `grok mcp doctor slugaudit`

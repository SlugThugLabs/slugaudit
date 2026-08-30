# Connecting SlugAudit to Grok

SlugAudit is an MCP server for AI coding agents. It indexes your codebase
once and exposes the facts to Grok as queryable tools — Grok reads only the
files and lines that matter, instead of re-reading the repository to find
where everything is. SlugAudit gathers facts; **Grok does the analysis**.

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

## Normal use

Start a fresh Grok session after connecting and work normally. Grok should
call `project_control` internally when it first needs the index, then use
`report`, `query`, and `structure` to narrow the source it reads. SlugAudit
handles the repetitive repository fact-gathering so Grok can spend more
context on actual analysis.

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
- **"project not enabled"** — Grok needs to call `project_control` once for
  the project. This is an integration issue, not normal user setup.
- **Diagnose connection issues:** `grok mcp doctor slugaudit`

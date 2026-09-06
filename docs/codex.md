# Connecting SlugAudit to Codex

SlugAudit is an MCP server for AI coding agents. It indexes your codebase
once and exposes the facts to Codex as queryable tools — Codex reads only
the files and lines that matter, instead of re-reading the repository to
find where everything is. SlugAudit gathers facts; **Codex does the
analysis**.

## One-line setup

```bash
slugaudit connect codex
```

That registers the `slugaudit` stdio MCP server globally in
`~/.codex/config.toml`. Verify:

```bash
codex mcp list
# slugaudit  /path/to/slugaudit  enabled
```

Codex has no user/project scope distinction — it always writes to the
global config, which is what you want for a per-project server like
SlugAudit.

## Normal use

Start a fresh Codex session after connecting and work normally. Codex should
call `project_control` internally when it first needs the index, then use
`report`, `query`, and `structure` to narrow the source it reads. SlugAudit
handles repetitive repository fact-gathering so Codex can spend more context
on actual analysis rather than rereading files.

## Re-running `connect`

Safe to re-run — it removes any existing `slugaudit` entry and re-adds
it, so upgrading the binary and re-running `connect codex` always points
at the current executable.

## Manual alternative

```bash
codex mcp add slugaudit -- $(which slugaudit)
```

## Troubleshooting

- **`codex` not found** — install the Codex CLI first.
- **Tools don't appear after `connect`** — restart Codex. Already-running
  sessions won't pick up a newly registered MCP server.
- **"project not enabled"** — Codex needs to call `project_control` once for
  the project. This is an integration issue, not normal user setup.
- **Codex shows the server as "Unsupported"** — this is a Codex display
  quirk for stdio servers that don't declare OAuth metadata. The server
  is still functional; verify with `codex mcp list` and try a `query`
  tool call.

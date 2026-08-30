# Connecting SlugAudit to your AI agent

SlugAudit is an MCP server built to save an AI's context and time. It gathers
repository facts once—files, symbols, calls, imports, structure, diagnostics,
and changes—so the agent does not have to read and reread file after file.

The division of labor is deliberate: SlugAudit gathers facts; the AI does the
actual analysis. It decides what is important, what is a real issue, and what
to do about it.

After the one-time connection, SlugAudit should be invisible during normal
work. The agent starts it and uses it when useful; the user does not manage
the index or database.


## Quick start

```bash
# 1. Build and install the binary (or use a released artifact)
cargo install --path .

# 2. Connect your agent — run this once, from any directory
slugaudit-mcp connect

# Or connect a specific agent directly:
slugaudit-mcp connect bob
slugaudit-mcp connect claude
slugaudit-mcp connect grok
slugaudit-mcp connect codex
```

`connect` with no argument shows an interactive menu of the supported
agents. With an agent name it registers this binary as the `slugaudit`
MCP server in that agent's config immediately.

## What `connect` does

`connect` writes a single entry into your agent's MCP configuration:

- **Server name:** `slugaudit`
- **Transport:** stdio (the agent launches the binary on demand)
- **Command:** the path to the `slugaudit-mcp` binary itself
  (resolved via `current_exe()`, so a `cargo install`-ed binary keeps
  working across upgrades automatically)

It registers at **user/global scope** so SlugAudit is available in every
project. The server itself is per-project — each project you enable gets
its own `.planning/slugaudit/project.db` SQLite index — so one global
registration covers everything.

If a `slugaudit` entry already exists, it is removed and re-added (so
re-running `connect` after upgrading the binary always points at the
current binary).

## After connecting

Start a fresh AI session and work normally. The agent should use
`project_control` internally when it first needs to index the current project;
that is not a step the user should have to perform. The first indexing pass
creates the disposable `.planning/slugaudit/` data, and later calls keep it
fresh as files change.

The intended workflow is simple: SlugAudit narrows the search and returns
compact facts; the AI reads only the relevant source and spends its context
on reasoning instead of repository housekeeping.

See the agent-specific guides for what to do next.

## Agent-specific guides

- [Bob](bob.md)
- [Claude Code](claude-code.md)
- [Grok](grok.md)
- [Codex](codex.md)

## Manual connection (no `connect` command)

If you prefer to wire it up by hand, or your agent isn't one of the four
above, register the binary as a stdio MCP server named `slugaudit`:

```
slugaudit-mcp
```

No arguments, no environment variables, no config file. The server uses
zero-config SQLite by default — one `.planning/slugaudit/project.db` per
enabled project.

## Troubleshooting

> **Licensing:** SlugAudit is free to use—including for your own commercial
> software. See the [license summary](../README.md#license) and complete
> [LICENSE](../LICENSE).

**`unknown agent "..."`** — `connect` accepts `bob`, `claude`, `grok`,
or `codex` (case-insensitive; `claude-code` and `claude_code` also map
to Claude Code).

**`<agent> CLI not found on PATH`** — the agent's CLI must be installed
and on `PATH` before `connect` can register with it. Install Claude Code
(`npm install -g @anthropic-ai/claude-code`), Grok, or Codex first.

**Agent doesn't see the `query`/`report`/`structure`/`finding` tools** —
restart the AI session after connecting the server.

**"project not enabled" / empty results** — this is an agent-integration
issue: the agent needs to call `project_control` internally once for the
current project. Users should not need to manage the SQLite data manually.

**I moved or copied the project and it seems stuck on the old path** -
SlugAudit's index is disposable derived data tied to a project root. When a
repo is moved, copied, or re-extracted from an archive (e.g. unzipping a
GitHub zip to a new directory), the stored index is recognized as stale and
rebuilt from the current source automatically. You should not need to delete
the `.planning/slugaudit/` directory or re-enable the project by hand.

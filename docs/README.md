# Connecting SlugAudit to your AI agent

**What this is:** an MCP server for AI coding agents. It indexes a codebase
once and exposes the facts to your agent as queryable tools.

**Who it's for:** your AI agent — that's who uses it. You (the human) install
and connect it once; after that it's invisible.

**What it's for:** saving your agent's context and time. SlugAudit gathers
repository facts — files, symbols, calls, imports, structure, diagnostics,
and changes — so the agent does not have to read and reread file after file
while auditing.

**What it is not:** a standalone audit tool, and not an auditor. There is no
command that prints an audit report, and SlugAudit doesn't judge code or
find bugs. It supplies evidence; **your AI does the analysis** — that
division of labor is deliberate. Ask your agent to audit the repo and it
will use SlugAudit's tools to do it.

After the one-time connection, SlugAudit should be invisible during normal
work. The agent starts it and uses it when useful; the user does not manage
the index or database.


## Quick start

```bash
# 1. Build and install the binary (or use a released artifact)
cargo install --path .

# 2. Connect your agent — run this once, from any directory
slugaudit connect

# Or connect a specific agent directly:
slugaudit connect bob
slugaudit connect claude
slugaudit connect grok
slugaudit connect codex
```

`connect` with no argument shows an interactive menu of the supported
agents. With an agent name it registers this binary as the `slugaudit`
MCP server in that agent's config immediately.

## Updating an installed copy

SlugAudit is a single binary, so upgrading is just replacing it at the same
path. For a binary that's already installed, run:

```bash
slugaudit update
```

`update` fetches the latest GitHub release (via `curl`), verifies its
SHA-256 checksum, and atomically replaces the installed binary in place
(no `curl`? download the release from GitHub and re-run `install`).

## What `connect` does

`connect` writes a single entry into your agent's MCP configuration:

- **Server name:** `slugaudit`
- **Transport:** stdio (the agent launches the binary on demand)
- **Command:** the path to the `slugaudit` binary itself
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
slugaudit
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

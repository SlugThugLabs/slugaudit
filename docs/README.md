# Connecting SlugAudit to your AI agent

SlugAudit is an MCP server — it exposes its tools (`query`, `report`,
`structure`, `finding`, `project_control`, `health`) to any AI agent that
speaks the Model Context Protocol. Once connected, the agent can query
codebase evidence directly instead of reading hundreds of files one at a
time.

SlugAudit does not audit. It performs no risk detection, assigns no
severity, and draws no conclusions — it supplies evidence, and the calling
AI performs all judgment.

## Quick start

```bash
# 1. Build and install the binary (or use a released artifact)
cargo install --path .

# 2. Connect your agent — run this once, from any directory
slugaudit-mcp connect

# Or connect a specific agent directly:
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

Connecting the MCP server only makes SlugAudit's tools *available* to the
agent. To actually index a project, enable it from inside the AI session
by calling the `project_control` tool with `action = "on"` (optionally
with a project path). That creates the activation marker under
`.planning/slugaudit/` and runs the first import immediately. From then
on, every AI tool call independently verifies freshness before executing
— if the project has a pending import, the call waits for it rather than
answering from partial state.

See the agent-specific guides for what to do next.

## Agent-specific guides

- [Claude Code](claude-code.md)
- [Grok](grok.md)
- [Codex](codex.md)

## Manual connection (no `connect` command)

If you prefer to wire it up by hand, or your agent isn't one of the three
above, register the binary as a stdio MCP server named `slugaudit`:

```
slugaudit-mcp
```

No arguments, no environment variables, no config file. The server uses
zero-config SQLite by default — one `.planning/slugaudit/project.db` per
enabled project.

## Troubleshooting

**`unknown agent "..."`** — `connect` accepts `claude`, `grok`, or
`codex` (case-insensitive; `claude-code` and `claude_code` also map to
Claude Code).

**`<agent> CLI not found on PATH`** — the agent's CLI must be installed
and on `PATH` before `connect` can register with it. Install Claude Code
(`npm install -g @anthropic-ai/claude-code`), Grok, or Codex first.

**Agent doesn't see the `query`/`report`/`structure`/`finding` tools** —
the project probably isn't enabled yet. Have the agent call
`project_control` with `action = "on"` and a project path, and wait for
the import to finish before asking it to use SlugAudit on that project.

**"project not enabled" / empty results** — same cause. Enable the
project first via `project_control`; the agent can't query evidence that
doesn't exist yet.

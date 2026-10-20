# Connecting SlugAudit to MCP clients

`slugaudit connect` registers the SlugAudit binary as a stdio MCP server named
`slugaudit` in an AI agent or editor. `slugaudit disconnect` (alias
`slugaudit remove`) removes that registration.

This document lists every target in the registry
(`src/connect_agents.rs`, 23 entries in this revision), the route `connect`
takes for it, the config path involved, the scope, and how to verify the
registration. The registry is the source of truth; if this page and the
registry disagree, the registry wins and this page is the bug.

## The connect/disconnect flow

```bash
slugaudit connect            # list detected agents and prompts for one
slugaudit connect <id>       # configure one target directly
slugaudit disconnect         # list connected agents and prompts for one
slugaudit disconnect <id>    # remove one target directly
```

Accepted aliases: `claude-code` → `claude`, `antigravity` → `agy`,
`github-copilot` → `copilot`, `oh-my-pi` → `omp`.

For a named target, `connect` decides in this order:

1. **Refuse, with instructions.** An entry with no established mechanism
   (`goose`, `crush`) is never attempted. The refusal names why, prints a
   paste-ready remedy with the binary path filled in, and ends with the
   `Reference:` (the probed version or documentation date) and the vendor
   `Docs:` link.
2. **Run the client's own CLI.** A `CliDirect` entry whose executable is on
   `PATH` runs that client's `mcp add` grammar (the exact argv is in the table
   below). Some entries allow a config-file fallback when the CLI is absent or
   rejects the add (`opencode`, `bob`, `pi`).
3. **Write the config file (config-file entries).** These merge exactly one
   `slugaudit` key into a file they can parse, using an atomic temp-file +
   rename. A config that is invalid JSON, whose top level is not an object, or
   whose target section is not an object is refused rather than rewritten.
   OpenCode's `.jsonc` file is parsed as strict JSON, so a file containing
   comments is refused; use OpenCode's own CLI in that case.
4. **Fall back or instruct (CLI entries).** If the client CLI is not on `PATH`
   and the entry has no file fallback, the message names the missing
   executable, the exact command to run, and the docs link.

`connect` never runs the client's `mcp remove` before `add`: a failed remove
could deregister a working server. Re-adding is idempotent. `disconnect` runs
`mcp remove` only when the client CLI is present, removes the file entry only
for configs SlugAudit writes, and never touches detection-only files (for
example `~/.claude.json`, `~/.codex/config.toml`, `~/.hermes/config.yaml`). If
no route was touched it prints `Nothing to disconnect.`

The registered binary is the stable install at
`~/.slugthug/slugaudit/slugaudit` when it exists, otherwise the running
binary. Client config paths are resolved against `$SLUGTHUG_CONFIG_HOME` when
set, otherwise `$HOME`. Every config-file write is **user scope**; SlugAudit
does not write project-scoped MCP configuration. For a CLI-driven entry with
no verified scope flag, the table says "client default"; where a client's CLI
defaults to project scope, `connect` passes the explicit user/global flag
shown below.

## Targets

`<binary>` is the absolute path described above. "Client default" means the
registry passes no scope flag because that client has no verified one; the
config path shown is the user-level file the client reads.

| `connect` id | Display name | Route | Config path (user home) | Scope | Command or written entry | Verify |
|---|---|---|---|---|---|---|
| `agy` | Antigravity | CLI | — (no verified path) | client default | `agy mcp add slugaudit <binary>` | `agy mcp list` |
| `amp` | Amp Agent | CLI | — (no verified path) | client default | `amp mcp add slugaudit -- <binary>` | `amp mcp list` |
| `bob` | Bob Agent | CLI, file fallback | `~/.bob/mcp_settings.json` | user (`--scope global`) | `bob mcp add --transport stdio --scope global slugaudit <binary>` | `bob mcp list` |
| `claude` | Claude Code | CLI | `~/.claude.json` (detection only) | user (`-s user`) | `claude mcp add -s user slugaudit -- <binary>` | `claude mcp list` |
| `claude-desktop` | Claude Desktop | config file | macOS `~/Library/Application Support/Claude/claude_desktop_config.json`; Windows `%APPDATA%/Claude/claude_desktop_config.json`; Linux `~/.config/Claude/claude_desktop_config.json` | user | writes `mcpServers.slugaudit` | restart Claude Desktop and check the server list |
| `codex` | Codex CLI | CLI | `~/.codex/config.toml` (detection only) | client default | `codex mcp add slugaudit -- <binary>` | `codex mcp list` |
| `copilot` | GitHub Copilot | CLI | `~/.copilot/mcp-config.json` (detection only) | client default | `copilot mcp add slugaudit -- <binary>` | `copilot mcp list` |
| `crush` | Crush | manual | `~/.config/crush/crushrc` | user | paste the block below | start `crush` and check its MCP servers |
| `cursor` | Cursor IDE | config file | `~/.cursor/mcp.json` | user | writes `mcpServers.slugaudit` | `agent mcp list` (Cursor's `agent` verifies but cannot add) |
| `droid` | Droid (Factory) | CLI | `~/.factory/mcp.json` (detection only) | client default | `droid mcp add --type stdio slugaudit <binary>` | `droid mcp list` |
| `gemini` | Gemini CLI | CLI | `~/.gemini/settings.json` (detection only) | user (`-s user`) | `gemini mcp add -s user slugaudit <binary>` | `gemini mcp list` |
| `goose` | Goose CLI | manual | `~/.config/goose/config.yaml` | user | paste the block below | start `goose` and check its extensions |
| `grok` | Grok CLI | CLI | `~/.grok/config.toml` (detection only) | user (`--scope user`) | `grok mcp add slugaudit --scope user -- <binary>` | `grok mcp list` |
| `hermes` | Hermes Agent | CLI | `~/.hermes/config.yaml` (detection only) | client default | `hermes mcp add slugaudit --command <binary>` | `hermes mcp list` |
| `kimi` | Kimi Code | config file | `~/.kimi-code/mcp.json` | user | writes `mcpServers.slugaudit` | restart Kimi Code and check its MCP settings |
| `omp` | Oh My Pi | config file | `~/.omp/agent/mcp.json` | user | writes `mcpServers.slugaudit` | load `omp` with a configured model provider and check MCP |
| `opencode` | OpenCode | CLI, file fallback | `~/.config/opencode/opencode.jsonc` | client default; user fallback | `opencode mcp add slugaudit -- <binary>`; fallback writes `mcp.slugaudit` | `opencode mcp list` |
| `openhands` | OpenHands | CLI | `~/.openhands/mcp.json` (detection only) | client default | `openhands mcp add slugaudit --transport stdio <binary>` | `openhands mcp list` |
| `pi` | Pi Agent | CLI, file fallback | `~/.pi/agent/mcp.json` | client default; user fallback | `pi mcp add slugaudit -- <binary>` | `pi mcp list` |
| `q` | Amazon Q Developer | CLI | — (no verified path) | client default | `q mcp add --name slugaudit --command <binary>` | `q mcp list` |
| `warp` | Warp Terminal | config file | `~/.warp/.mcp.json` (leading dot) | user | writes `mcpServers.slugaudit` | check Warp's MCP settings; `oz` is detection only |
| `windsurf` | Windsurf | config file | `~/.codeium/windsurf/mcp_config.json` | user | writes `mcpServers.slugaudit` | restart Windsurf and check Cascade's MCP servers |
| `zed` | Zed Editor | config file | `~/.config/zed/settings.json` | user | writes `context_servers.slugaudit` | open Zed's agent panel and check its context servers |

### Written JSON shapes

- **`mcpServers.slugaudit`** (`bob`, `claude-desktop`, `cursor`, `kimi`, `omp`,
  `pi`, `warp`, `windsurf`):

  ```json
  { "mcpServers": { "slugaudit": { "command": "<binary>", "args": [] } } }
  ```

  Detection also accepts the legacy `mcp` alias for this shape.
- **`mcp.slugaudit`** (`opencode`):

  ```json
  { "mcp": { "slugaudit": { "type": "local", "command": ["<binary>"], "enabled": true } } }
  ```

- **`context_servers.slugaudit`** (`zed`): `command` is a **string** with a
  sibling `args` array:

  ```json
  { "context_servers": { "slugaudit": { "command": "<binary>", "args": [] } } }
  ```

### Manual setups (`goose`, `crush`)

`goose` has no `mcp add` subcommand. Add this block to
`~/.config/goose/config.yaml` yourself; `connect` refuses because merging YAML
without a parser would clobber the existing file:

```yaml
extensions:
  slugaudit:
    type: stdio
    cmd: <binary>
    args: []
```

`crush` has no `mcp` shell subcommand; `mcp add` is only a `crushrc` bash
builtin. Paste this line into `~/.config/crush/crushrc`, or use the legacy
`crush.json` with key `mcp`:

```text
mcp add slugaudit --command <binary>
```

## Verifying a registration

For CLI-driven targets, `connect` prints `Verify with: <cli> mcp list`, and
that list should contain `slugaudit`. `is_connected` uses the same route when
deciding whether to print `[Connected]` in the interactive list.

For config-file targets, restart the client and inspect its MCP/extension
settings; there is no universal list command. `cursor` is the exception with a
verify-only CLI: `agent mcp list` reports the registration but `agent` cannot
create it.

`disconnect <id>` prints `Done.` only when a route was actually touched, and
`Nothing to disconnect.` otherwise.

## Maintainer note

The client registry records, per entry, how its facts were established:
`VerifiedByProbe` (an installed binary was run), `VerifiedFromSource` (the
client's source or schema was read), or `DocVerified` (vendor documentation).
A failed attempt prints the entry's `Reference:` and, for
documentation-derived entries, its `Docs:` link; a refusal always prints both,
because it is sending the user to the vendor page.

The client-facing test suites are hermetic: `tests/connect_tests.rs` runs the
real binary with a temporary `HOME`/`SLUGTHUG_HOME`/`SLUGTHUG_CONFIG_HOME` and
a **replaced** `PATH` containing fake CLIs that record their argv, so no test
can resolve or execute an installed client or read a real client config.
`tests/menu_tests.rs` is sandboxed the same way because its connect and
disconnect options run this path.

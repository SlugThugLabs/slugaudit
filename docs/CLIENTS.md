# Client integrations

SlugAudit is a stdio MCP server named `slugaudit`. `slugaudit connect [TARGET]`
registers it with an AI agent or editor; `slugaudit disconnect [TARGET]`
(alias `slugaudit remove`) removes it. With no target, both commands are
interactive, as is `slugaudit menu` (option 2 connects, option 3 disconnects)
(`src/menu.rs`).

The command a client launches is the binary itself, with no arguments —
`serve` is the default subcommand. The path written into a client config is
the installed stable path `$SLUGTHUG_HOME/slugaudit/slugaudit` when it exists,
otherwise the running executable (`prefer_slugthug_binary`,
`src/connect.rs`; resolved by `running_binary()`,
`src/install.rs`).

This page is an integration reference: which targets exist and how registration
actually happens. It is not a per-client guide to using SlugAudit inside that
client.

## Source of truth and evidence

The source of truth is the code, not this page. `src/connect_agents.rs` holds
the `AGENTS` registry (`src/connect_agents.rs`) — one `AgentDef` row per
supported target, each recording its strategy, client executable, config path,
config shape, argument dialect, evidence tier, fallback flag, manual-steps flag
and a reconciliation note. The argv that actually runs is in
`run_add_cmd`/`run_remove_cmd` (`src/connect_exec.rs`), and
the probe-or-fallback decision is `connect_agent` (`src/connect_exec.rs`).

Re-derive the table instead of trusting it:

```bash
sed -n '/pub const AGENTS/,/^];/p' src/connect_agents.rs
sed -n '/fn run_add_cmd/,/^}/p' src/connect_exec.rs
sed -n '/fn run_remove_cmd/,/^}/p' src/connect_exec.rs
```

Every row carries an `Evidence` tier (`src/connect_agents.rs`). Tiers,
with the number of rows at each:

| Tier | Meaning | Rows |
|---|---|---|
| `VerifiedByProbe` | the real CLI was installed and its `mcp` subcommands were probed | 18 |
| `DocVerified` | established from vendor/user documentation | 3 |
| `VerifiedFromSource` | established from the client's own source; stronger than docs, but no runtime probe | 2 |

How those facts were obtained: the real CLIs were installed and their `mcp`
subcommands probed by hand — that is the 18 `VerifiedByProbe` rows. Before the
probes, two independent documentation-based passes had disagreed with each
other on seven of the original 26 targets, and each pass was wrong on roughly half of
those (`src/connect_agents.rs`). Documented commands that turned out not
to exist include:

- `cursor` — there is no `agent mcp add`. The `agent` CLI exposes
  login/list/list-tools/enable/disable, so `agent mcp list` can verify a
  registration but cannot create one (`src/connect_agents.rs`).
- `kimi` — the current Kimi Code CLI has no `mcp` subcommand at all; `kimi mcp
  add` belonged to the archived legacy kimi-cli
  (`src/connect_agents.rs`).

`bob` and `droid` were missed by both documentation passes and were resolved
only by probing: bob's `-t/--transport` and `-s/--scope` flags and droid's
`--type` flag (there is no `--transport` alias) are in the registry solely
because the installed binaries were run (`src/connect_agents.rs`).

A client whose mechanism could not be established is not in the registry.
`pearai` (no mechanism either research pass could agree on), `trae` (its
installer is geo-blocked and its CLI is enterprise-gated, so it could not be
probed) and `1mcp` (a gateway, so its `mcp add` registers a backend rather than
a client) were removed on 2026-10-17. `gemini` was probed against the real CLI
that day (0.64.0) and `omp` was checked against its own source and schema
(18.12.0). The three remaining `DocVerified` rows (`windsurf`, `warp`,
`claude-desktop`) are GUI applications with no CLI to run. No row currently uses
`Strategy::Unknown`; the variant exists so an unestablished mechanism refuses
rather than running an invented command (`src/connect_exec.rs`).

## The three registration states

`Strategy` (`src/connect_agents.rs`):

- **`CliDirect` (14 rows)** — the client documents a scriptable `mcp add`, so
  slugaudit runs `<cli> mcp add ...`. `connect` first runs the client's remove
  command so a re-connect replaces the entry instead of duplicating it
  (`src/connect_exec.rs`).
- **`CliInteractiveOnly` (1 row: `omp`)** — the client's only add path is
  in-session (`/mcp add`), which slugaudit cannot drive programmatically, so
  the installer writes the client's config file instead.
- **`ConfigFile` (8 rows)** — there is no usable CLI, so slugaudit merges the
  client's config file. Two of these, `goose` and `crush`, are refused with
  printed instructions because no safe machine-writable shape exists.
- **`Unknown` (0 rows)** — reserved for "the mechanism was never established";
  it refuses.

A `CliDirect` row may additionally set `file_fallback`. When the CLI is not on
`PATH`, or when its `mcp add` exits non-zero, `connect` then writes the config
file instead (`src/connect_exec.rs`). Three rows do this: `opencode`,
`bob`, `pi`. Without the flag, an absent CLI is a hard error (`<agent> CLI
(<cli>) not found on PATH — install it first`, `src/cli.rs`,
`src/connect_exec.rs`). Falling back
is deliberate: the client ecosystem moves, so the installer probes rather than
assuming a fixed command forever.

## Two different home directories

`slugthug_home()` and the client config base are different directories, and
conflating them was a bug:

- `slugthug_home()` — SlugAudit's own base: `$SLUGTHUG_HOME` if set, otherwise
  `$HOME/.slugthug` (default `~/.slugthug`). It holds the binary install
  (`~/.slugthug/slugaudit/slugaudit`) and runtime state
  (`src/install.rs`).
- `config_home()` — the **user's** home: `$SLUGTHUG_CONFIG_HOME` if set,
  otherwise `$HOME` (`src/install.rs`). Client config paths resolve
  against this (`src/connect.rs`).

So `.cursor/mcp.json` resolves to `~/.cursor/mcp.json`. A client config written
under `~/.slugthug/` is a file no client ever reads — the write succeeds and the
client silently lists nothing. Set `SLUGTHUG_CONFIG_HOME` when `connect` runs
under a different home than the agent it registers (for example, root running
`connect` for a desktop user).

Path kinds (`ConfigPath`, `src/connect_agents.rs`; resolution in `ConfigPath::resolve`):

- `Home(rel)` → `<user home>/<rel>`.
- `PerOs { macos, windows, linux }` → the string for the running OS; on
  Windows a `%APPDATA%/` prefix expands to `$APPDATA`, or
  `<user home>/AppData/Roaming` when unset (`expand_os_tokens`,
  `src/connect_agents.rs`).
- `Unverified` → no verified path; the writer refuses.

## Targets

23 rows — exactly the 23 `AgentDef` rows in `AGENTS`. "Config file" is the path
when slugaudit writes one; where registration goes through the CLI, the config
path is read for detection only and slugaudit does not write it. Paths are
relative to the user's home unless marked otherwise.

| Target id | Client | Registration today | Config file | Manual steps | Evidence | Reference | Docs |
|---|---|---|---|---|---|---|---|
| `agy` | Antigravity (agy) | `agy mcp add slugaudit <binary>` | — | No | `VerifiedByProbe` | agy 1.2.14 (probed 2026-09-30) | <https://antigravity.google/docs/mcp/> |
| `gemini` | Gemini CLI | `gemini mcp add -s user slugaudit <binary>` | `~/.gemini/settings.json` (detection only) | No | `VerifiedByProbe` | gemini 0.64.0 (probed 2026-10-17) | <https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md> |
| `claude` | Claude Code | `claude mcp add -s user slugaudit -- <binary>` | `~/.claude.json` (detection only) | No | `VerifiedByProbe` | claude 2.1.233 (probed 2026-09-30) | <https://code.claude.com/docs/en/mcp> |
| `hermes` | Hermes Agent | `hermes mcp add slugaudit --command <binary>` | `~/.hermes/config.yaml` (detection only) | No | `VerifiedByProbe` | hermes 0.18.2 (probed 2026-09-30) | <https://hermes-agent.nousresearch.com/docs/reference/mcp-config-reference> |
| `copilot` | GitHub Copilot | `copilot mcp add slugaudit -- <binary>` | `~/.copilot/mcp-config.json` (detection only) | No | `VerifiedByProbe` | copilot 1.0.89 (probed) | <https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers> |
| `codex` | Codex CLI | `codex mcp add slugaudit -- <binary>` | `~/.codex/config.toml` (detection only) | No | `VerifiedByProbe` | codex 0.149.1 (probed 2026-09-30) | <https://developers.openai.com/codex/mcp/> |
| `opencode` | OpenCode | `opencode mcp add slugaudit -- <binary>`; config-file fallback | `~/.config/opencode/opencode.jsonc` (written on fallback) | No | `VerifiedByProbe` | opencode 1.18.33 (probed) | <https://opencode.ai/docs/mcp-servers/> |
| `amp` | Amp Agent | `amp mcp add slugaudit -- <binary>` | — | No | `VerifiedByProbe` | amp 0.0.1790778300-g865493 (probed) | <https://ampcode.com/docs/customize/mcp> |
| `bob` | Bob Agent | `bob mcp add --transport stdio --scope global slugaudit <binary>`; config-file fallback | `~/.bob/mcp_settings.json` (written on fallback) | No | `VerifiedByProbe` | bob 2.0.5 (probed 2026-09-30) | <https://bob.ibm.com/docs/shell/configuration/mcp/mcp-bobshell> |
| `grok` | Grok CLI | `grok mcp add slugaudit --scope user -- <binary>` | `~/.grok/config.toml` (detection only) | No | `VerifiedByProbe` | grok 1.0.44 (probed) | <https://docs.x.ai/build/features/mcp-servers> |
| `goose` | Goose CLI | refuses: YAML config cannot be merged safely; instructions printed | `~/.config/goose/config.yaml` (not written) | **Yes** | `VerifiedByProbe` | goose 1.52.0 (probed 2026-09-30) | <https://goose-docs.ai/docs/getting-started/using-extensions/> |
| `openhands` | OpenHands | `openhands mcp add slugaudit --transport stdio <binary>` | `~/.openhands/mcp.json` (detection only) | No | `VerifiedByProbe` | openhands 1.16.0 (probed 2026-09-30) | <https://docs.openhands.dev/openhands/usage/cli/mcp-servers> |
| `kimi` | Kimi Code | config write (no `mcp` subcommand exists) | `~/.kimi-code/mcp.json` (written) | No | `VerifiedByProbe` | kimi 2.1.1 (probed) | <https://moonshotai.github.io/kimi-code/en/customization/mcp> |
| `droid` | Droid (Factory) | `droid mcp add --type stdio slugaudit <binary>` | `~/.factory/mcp.json` (detection only) | No | `VerifiedByProbe` | droid 0.230.0 (probed) | <https://docs.factory.ai/cli/configuration/mcp> |
| `crush` | Crush | refuses: `mcp add` is only a crushrc bash builtin, no shape; instructions printed | `~/.config/crush/crushrc` (not written) | **Yes** | `VerifiedByProbe` | crush 0.97.1 (probed) | <https://github.com/charmbracelet/crush> |
| `q` | Amazon Q Developer | `q mcp add --name slugaudit --command <binary>` | — | No | `VerifiedByProbe` | q 1.19.7 (probed 2026-09-30) | <https://docs.aws.amazon.com/amazonq/latest/qdeveloper-ug/command-line-mcp.html> |
| `cursor` | Cursor IDE | config write (the `agent` CLI has no `mcp add`) | `~/.cursor/mcp.json` (written) | No | `VerifiedByProbe` | cursor 2026.09.28-64d2043 (probed) | <https://cursor.com/docs/cli/mcp> |
| `windsurf` | Windsurf | config write | `~/.codeium/windsurf/mcp_config.json` (written) | No | `DocVerified` | vendor docs, as of 2026-09-30 | <https://docs.devin.ai/windsurf/plugins/cascade/mcp> |
| `pi` | Pi Agent | `pi mcp add slugaudit -- <binary>`; config-file fallback | `~/.pi/agent/mcp.json` (written on fallback) | No | `VerifiedByProbe` | pi 0.99.1 (probed) | <https://pi.dev/docs/latest/mcp> |
| `omp` | Oh My Pi (omp) | config write (client's own add is in-session `/mcp add`) | `~/.omp/agent/mcp.json` (written) | No | `VerifiedFromSource` | omp 18.12.0 source and schema (checked 2026-10-17) | <https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md> |
| `warp` | Warp Terminal | config write | `~/.warp/.mcp.json` (written; filename has a leading dot) | No | `DocVerified` | vendor docs, as of 2026-09-30 | <https://docs.warp.dev/agents/capabilities/mcp/> |
| `claude-desktop` | Claude Desktop | config write | per-OS, see below | No | `DocVerified` | vendor docs, as of 2026-09-30 | <https://modelcontextprotocol.io/docs/develop/connect-local-servers> |
| `zed` | Zed Editor | config write (`context_servers`) | `~/.config/zed/settings.json` (written) | No | `VerifiedFromSource` | Zed source, as of 2026-09-30 | <https://zed.dev/docs/ai/mcp> |

Notes on individual rows:

- `—` in the config column means the row has no config path at all (`agy`,
  `amp`, `q`): detection is CLI-only, via `<cli> mcp list`.
- Rows that register through their CLI are detected by reading the config file
  for a `slugaudit` entry, and fall back to running `<cli> mcp list`
  (`AgentDef::is_connected`, `src/connect_agents.rs`;
  `cli_check_connected`, `src/connect_exec.rs`).
- Two rows set `manual_steps` — `goose` and `crush` — and `connect` refuses
  them with a paste-ready remedy instead of attempting a write. `goose`'s config
  is YAML and `crush`'s `mcp add` exists only as a crushrc bash builtin. `omp`
  needs no manual step for slugaudit's write even though the client's own
  `/mcp add` is in-session-only.
- Three CLI rows carry the config-file fallback (`opencode`, `bob`, `pi`).
  `opencode` additionally has no `--type`/`--transport` flag — it infers the
  transport — and its own writer omits `enabled`, while slugaudit's fallback
  emits it (`src/connect_agents.rs`).
- `goose` is never written: `ConfigShape::GooseExtensions::write_section()`
  returns `None`, so the generic JSON merge cannot be reached for it. That is
  deliberate — goose's `config.yaml` is YAML, and rewriting a hand-authored
  config as JSON would clobber it. `connect goose` prints the exact
  `extensions:` block to paste instead.
- `claude-desktop` uses a different path per OS (`ConfigPath::PerOs`):
  macOS `~/Library/Application Support/Claude/claude_desktop_config.json`,
  Windows `%APPDATA%/Claude/claude_desktop_config.json`, Linux
  `~/.config/Claude/claude_desktop_config.json`. The Linux path is convention,
  not vendor-documented (`src/connect_agents.rs`).

## What a config write produces

The written `slugaudit` value depends on the row's `ConfigShape`
(`ConfigShape::entry`, `src/connect_agents.rs`). The section it is
inserted into is the first of `ConfigShape::sections`
(`src/connect_agents.rs`):

- `McpServersCommandArgs` (most config-file rows) → top-level `mcpServers`:
  `"slugaudit": { "command": "<binary>", "args": [] }`. The legacy `mcp` key is
  also accepted when reading.
- `OpenCodeLocal` (`opencode`) → top-level `mcp`:
  `{ "type": "local", "command": ["<binary>"], "enabled": true }`.
- `ZedContextServer` (`zed`) → top-level `context_servers`:
  `{ "command": "<binary>", "args": [] }`. `command` is a string, with a
  sibling `args` array; the nested `{path,args}` form is Zed's legacy shape
  (`src/connect_agents.rs`).
- `GooseExtensions` (`goose`) → top-level `extensions`:
  `{ "type": "stdio", "cmd": "<binary>", "args": [] }` — detection only;
  `write_section()` returns `None` for this shape, so the user pastes the YAML
  block themselves (see the goose row above).
- `Unknown` (`crush`) → no section; the writer refuses rather
  than guessing.

The rest of the file is preserved; a file that exists but is not valid JSON is
an error and is left untouched (`load_json_or_empty`,
`src/connect_exec.rs`). Writes are atomic (temp file + rename,
`write_atomic`, `src/connect_exec.rs`).

## Disconnect

`slugaudit disconnect [TARGET]` (alias `slugaudit remove`) reverses the
registration (`disconnect_agent`, `src/connect_exec.rs`):

- Config-file rows: removes the `slugaudit` key from the section(s) the row
  reads — `mcpServers`/`mcp` for `McpServersCommandArgs`, `mcp` for
  `OpenCodeLocal`, `context_servers` for `zed`, `extensions` for `goose`
  (`remove_json_slugaudit`, `src/connect_exec.rs`). The same removal
  applies to the fallback config file of `opencode`, `bob` and `pi`.
  Detection-only paths (`.claude.json`, `config.yaml`, `config.toml`, …) are
  left untouched, since slugaudit never wrote them.
- CLI rows: runs the client's own remove command (`run_remove_cmd`,
  `src/connect_exec.rs`):
  - `claude` and `gemini` → `<cli> mcp remove -s user slugaudit`
  - `grok` → `grok mcp remove slugaudit --scope user`
  - `bob` → `bob mcp remove --scope global slugaudit`
  - `q` → `q mcp remove --name slugaudit`
  - every other CLI row → `<cli> mcp remove slugaudit`

With no target, `disconnect` lists the targets currently connected and prompts
for one (`src/connect.rs`). A target counts as connected when its
config file contains a `slugaudit` entry, or when `<cli> mcp list` output
contains `slugaudit` (`src/connect_agents.rs`,
`src/connect_exec.rs`).

## After connecting

Registering is only a config change — `connect` writes the client's config file
or runs the client's own `mcp add` (`connect_agent`,
`src/connect_exec.rs`) — and a client that is already running keeps the
server list it read at startup. Start a fresh client session after connecting;
an already-running one will not pick up the newly registered server. (The same
restart applies after `slugaudit update`, which replaces the binary and says so
at `src/update.rs`.) In the new session the client lists `slugaudit` as an
MCP server and exposes its tools: `report`, `leads`, `query`, `structure`,
`finding`, `project_control` (`src/server.rs`). For CLI rows, `connect` prints the check to run:
`Verify with: <cli> mcp list` (`src/connect.rs`).

## Re-running `connect`

For config-file rows the `slugaudit` key is merged in place, so re-running is
safe. For CLI rows `connect` is **add-only**: it never runs the client's
`remove` or `disable`, because a failed remove can deregister a working server.
If the client already holds a `slugaudit` entry, its own `mcp add` may refuse;
slugaudit then prints what it tried, why it failed, and the exact command to
finish by hand — or merges the config file where that row has a fallback.

## What `connect` prints

`connect` says what it did, not just that it worked, and no path ends in a bare
failure. The policy is *tell the user how, rather than leaving them not knowing
or breaking it trying*.

- **Probed success** (CLI route): `Ran: claude mcp add -s user slugaudit --
  <binary>`, then `Reference: claude 2.1.233 (probed 2026-09-30)`. No `Docs:`
  line — the grammar came from running the real binary.
- **Documented success** (any non-probed evidence): `Wrote
  mcpServers.slugaudit to <path>`, then `Reference: vendor docs, as of
  2026-09-30` and `Docs: <vendor page>`.
- **Fallback**: the CLI route was tried and could not be used, so the message
  states which route was taken and why before the write report.
- **Failure**: the attempted action, the failure, the exact command to run by
  hand, plus `Reference:` and `Docs:`.
- **Refusal**: what cannot be automated, the paste-ready remedy, `Reference:`
  and `Docs:` — the two `manual_steps` rows.

Automation is attempted only when slugaudit can be confident at the moment of
the call: probed rows as before; documented rows only for a single-key merge
into a JSON file that parses. Non-JSON configs, a platform outside a row's
`PerOs` paths and `manual_steps` rows are instructed instead of attempted, and
nothing is ever overwritten that slugaudit could not parse.

## Errors

- Unknown target name → `unknown agent "<name>"; run `slugaudit connect` to
  see detected agents` (`src/cli.rs`). A target is matched
  case-insensitively by id or display name, with these aliases
  (`src/connect_agents.rs`): `claude-code` / `claude_code` → `claude`;
  `antigravity` → `agy`; `github-copilot` / `github_copilot` → `copilot`;
  `oh-my-pi` / `oh_my_pi` → `omp`.
- A CLI row whose executable is not on `PATH` and has no config fallback →
  `cannot be connected automatically`, naming the missing binary, the exact
  command slugaudit would have run, the `Reference:` the row's facts come from,
  and the vendor `Docs:` link.
- A row that cannot be automated (`goose`, `crush`) → the same message shape,
  carrying the paste-ready remedy the registry holds for that row.
- A config file that exists but is not valid JSON → `cannot modify <path>:
  configuration file exists but contains invalid JSON: … (fix
  syntax/comments or remove the file to reset, then retry)`
  (`src/cli.rs`).
- The interactive list is built from targets detected on this machine; a
  target is detected when its CLI is on `PATH`, or its config file (or the
  file's parent directory) exists under the user home
  (`AgentDef::is_installed`, `src/connect_agents.rs`).

## Troubleshooting

- **The client's server list does not show `slugaudit`** (or lists it but
  fails to launch it) — the path recorded at registration may no longer be
  where the binary lives. Re-run `slugaudit connect [TARGET]`: every connect
  re-resolves the path and rewrites the entry, so an entry left over from
  before `slugaudit install` (which puts the binary at the preferred stable
  path, `src/install.rs`) or from before the binary was moved is
  repaired to the current one (`src/connect.rs`,
  `src/install.rs`). Re-running replaces the existing entry rather than
  duplicating it (see Re-running `connect` above), and the repaired
  registration is live in a fresh session.
- **A config-file row appears connected but the client ignores it** — check
  the write landed under the user's home (`$HOME`, or
  `$SLUGTHUG_CONFIG_HOME`), not under `~/.slugthug` (see Two different home
  directories above).

## `install.sh`

`install.sh` installs the binary to `${HOME}/.slugthug/slugaudit/slugaudit` and
offers to add that directory to `PATH`. It registers SlugAudit with no client;
it prints a `slugaudit connect` hint instead.

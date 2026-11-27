# Privacy and security

## SlugAudit is not a security boundary

SlugAudit is a local codebase index that saves your AI coding agent context and tokens on repetitive file reads. It is **not** a security scanner, sandbox, or permission boundary:

- **Host coding harness owns permissions.** Your OS and host coding harness (Claude Code, Cursor, Gemini CLI, Codex, Antigravity, etc.) own workspace access control and already grant the model direct read access to the repository's files.
- **`.planning/slugaudit/` is disposable local scratch.** `project.db`, `findings-<id>.db`, and `results/` hold only derived data from that repository and session notes written by your agent. They are created with your normal default user permissions — SlugAudit does not chmod `0600`, enforce symlink/path jails, or run SQL authorizers to defend its own scratch files against you or the model.
- **Bounded against accidental runaway work.** SlugAudit bounds resource use so an accidental broad query or pathological file does not hang your session: `query` opens SQLite read-only, accepts a single `SELECT` or `WITH` statement, and enforces SQLite VM-step, wall-clock, and byte ceilings; Tree-sitter parses and structure/mechanic scans run under wall-clock deadlines, depth bounds, and match caps.
- **Counts are mechanical facts, not safety verdicts.** `leads` counts syntactic patterns, and `credential_files` only matches path names. A zero from any tool is never a statement that code is safe.

## How it runs

- Local process started by your coding client over standard input and output (`stdio`). It opens no listening port and accepts no inbound network connections.
- Runs as your local user account.
- Zero telemetry: SlugAudit never reports usage, errors, paths, queries, or code anywhere.

## What it reads

| What | When |
|---|---|
| Regular files in an enabled project, except excluded/ignored paths (see [how it works](how-it-works.md)) | On every tool call except `health` (and cursor continuations). Full UTF-8 text of indexed files is stored in `project.db`. |
| `.gitignore`, `.ignore`, `.git/info/exclude`, and your global gitignore | On every sync walk, to skip ignored files. |
| `~/.slugthug/slugaudit/config.json` (optional, read-only) | At startup and when resolving project limits. Tools never write this file. |
| `<project>/.gitignore` | When enabling a project (`project_control` `"action": "on"`). |
| Client MCP configuration files | Only when you run `slugaudit connect`, `slugaudit disconnect`, or `slugaudit menu`. |
| Your shell rc file (`~/.bashrc`, `~/.zshrc`, `~/.config/fish/config.fish`) | Only if you confirm the interactive `PATH` prompt during `slugaudit install`. |

Files matching credential-like path patterns (`.env`, `*.pem`, `*.key`, `id_rsa*`, ...) are indexed like any other non-ignored text file. Add them to `.gitignore` or `.ignore` to exclude them; `report` and `leads` count matching paths so the agent can see when such files are present.

## What it writes

| What | Where |
|---|---|
| Derived index (`project.db`), per-session findings (`findings-<id>.db`), lock files, WAL sidecars, and paged result spill files (`results/`) | `<project>/.planning/slugaudit/` |
| `.planning/README.md` (if absent) and `.planning/slugaudit/` ignore rule | `<project>/.planning/README.md` and `<project>/.gitignore` on `project_control` `on` |
| `slugaudit` and previous binary `slugaudit.prev` | `~/.slugthug/slugaudit/` (on `install.sh`, `slugaudit install`, or `slugaudit update`) |
| `"slugaudit"` MCP server entry | Your chosen client's CLI or config file (on `connect` / `disconnect`; `.jsonc` files containing comments are left untouched with manual instructions) |
| Downloaded non-Rust Tree-sitter grammars | `tree-sitter-language-pack`'s user cache directory (see [languages](languages.md)) |

SlugAudit never modifies your source code. `project_control` with `"action": "off"` deletes the entire `<project>/.planning/slugaudit/` directory.

## Outbound network requests

SlugAudit makes **no** network requests during normal Rust MCP tool calls (Rust's Tree-sitter grammar is compiled statically into the binary). Outbound HTTPS requests happen only in three explicit cases:

| Trigger | Destination | What is sent |
|---|---|---|
| First-use non-Rust grammar download | `github.com/xberg-io/tree-sitter-language-pack` releases (or `TREE_SITTER_LANGUAGE_PACK_MANIFEST_URL`) | Standard HTTPS GET for the grammar manifest/shared library when an enabled project contains a non-Rust language not yet cached locally. No repository data is sent. |
| `slugaudit update` | `api.github.com` and `github.com/SlugThugLabs/slugaudit/releases` | HTTPS-only `curl -q` GET for the latest release tag, binary, and `SHA256SUMS` (verified before replacing the binary). |
| `install.sh` | `github.com/SlugThugLabs/slugaudit` releases (or `cargo install --git` fallback) | HTTPS download of the release binary and `SHA256SUMS` (or source crate dependencies when building from source). |

## Your coding agent and model provider

SlugAudit returns file text, AST evidence, and session findings to the MCP client that called it. Where those tool responses go next depends on your coding harness and model provider. Ensure `.planning/slugaudit/` stays gitignored (done automatically on `project_control` `on`) so local index files are not committed to version control.

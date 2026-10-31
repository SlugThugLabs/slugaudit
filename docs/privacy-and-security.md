# Privacy and security

## SlugAudit is not a security product

It indexes your code so an AI agent can query it. It does not scan for vulnerabilities, sandbox anything, or defend the index against a hostile user on your machine. The index is your own local scratch file. `leads` reports mechanical facts, and `credential_files` only matches path names. A zero from any tool is not a statement that the code is safe.

## How it runs

- It is a local program. Your agent starts it and talks to it over standard input and output (stdio). It opens no network port and accepts no network connections.
- It runs with your user's permissions.
- It has no telemetry. A search of the source finds no code that reports usage, errors or any other data to anyone. The only network code is listed below.

## What it reads

| What | When |
|---|---|
| Every file in an enabled project, except those excluded (see [how it works](how-it-works.md)) | On every tool call. The full text of text files is copied into the index. |
| `.gitignore`, `.ignore`, `.git/info/exclude`, and your global gitignore | On every tool call, to decide what to skip. |
| `.planning/slugaudit/config.json` | On every call. |
| The project's `.gitignore` | When you enable a project. |
| Each agent's config file | When you run `connect`, `disconnect` or `menu`. |
| Your shell config (`~/.bashrc` or similar) | Only if you accept the PATH prompt in `slugaudit install`. |

Files that look like credentials (`.env`, `*.pem`, `*.key`, `id_rsa`, ...) are indexed like any other text file if they are not ignored. Their text is stored in the index and an agent can read it with `query`. Add them to `.gitignore` or `.ignore` to keep them out. `report` and `leads` count credential-shaped paths so the agent knows they are there.

## What it writes

| What | Where |
|---|---|
| The index (a copy of your source text and extracted facts), its WAL files, stored paged results, and optional `config.json` | `<project>/.planning/slugaudit/` |
| One line, `.planning/slugaudit/`, appended | `<project>/.gitignore` (when you enable a project) |
| The `slugaudit` binary and `slugaudit.prev` | `~/.slugthug/slugaudit/` |
| An MCP server entry named `slugaudit` | Your agent's config, via its CLI or by editing its config file |
| A PATH line | Your shell config, only if you say yes |
| Downloaded grammars | The grammar library's cache directory (see [languages](languages.md)) |

The runtime directory is created owner-only (mode 0700 on Unix) and new database files are created owner-only (mode 0600), because they contain a copy of your source text. Existing files are used as they are.

SlugAudit does not modify your source files. The one edit to a file of yours is the `.gitignore` line above.

Findings you record are stored in the index database. They are plain text that your agent wrote.

## What it sends over the network

The server itself makes no network calls while answering tools. These are the only places anything touches the network:

| What | Trigger | Where | What is sent |
|---|---|---|---|
| Grammar download | The first time a project contains a language whose grammar is not cached. Done by the `tree-sitter-language-pack` library. | `github.com/xberg-io/tree-sitter-language-pack` release downloads (a manifest, then a grammar bundle) | An ordinary HTTPS download request. No project data is sent. The library uses your `HTTPS_PROXY`-style environment settings and can be pointed at a mirror with `TREE_SITTER_LANGUAGE_PACK_MANIFEST_URL`. |
| Update | You run `slugaudit update`. Uses `curl`. | `api.github.com` (latest release), then `github.com/SlugThugLabs/slugaudit/releases` (binary and `SHA256SUMS`) | HTTPS GET requests only. HTTP and downgrades are refused. The checksum is verified before the installed binary is touched. |
| Install | You run `install.sh`. | `github.com/SlugThugLabs/slugaudit/releases/latest/download` (binary and `SHA256SUMS`). With a Rust toolchain and no prebuilt binary, `cargo install --git` fetches the repository and crates. | HTTPS downloads. The script checks the SHA-256 checksum before installing. |

Your source code, queries, findings and paths are never sent anywhere by SlugAudit.

A GitHub request exposes your IP address and request metadata to GitHub, as any download does.

## Your agent is a separate party

SlugAudit answers your agent with your code: file text, symbols, comments and findings. What the agent does with those answers is up to the agent and its model provider. If your agent sends tool results to a hosted model, then the parts of your code that SlugAudit returns go there too. SlugAudit does not control that. The `credential_files` and `credential_pattern_file_count` figures exist so an agent can know such files are in the index.

## Index file safety

- The index is a local file in your project. Add `.planning/slugaudit/` to `.gitignore` (SlugAudit does this for you) so it is not committed.
- Anyone who can read `.planning/slugaudit/project.db` can read the text of every indexed file. Do not share or upload that folder.
- `project_control` with `off` removes the database and its sidecars.
- SlugAudit refuses to open a `project.db` that is a symlink, so it never writes through a link to another file.
- The SQL tool is read-only by construction: it accepts only `SELECT` or `WITH`. There is no SQL permission layer beyond that, because the database is your own scratch file.

# SlugAudit

SlugAudit is a local stdio MCP server that indexes an opted-in project into a
disposable SQLite database (`<root>/.planning/slugaudit/project.db`), so an AI
coding agent queries symbols, call sites, imports, comments, diagnostics,
dependency edges, and file text instead of reading files one by one.

## Language support

**Rust is the only tested language.**

Rust's Tree-sitter grammar is compiled statically into the `slugaudit` binary,
so Rust works offline with zero runtime grammar downloads. Other languages
download Tree-sitter grammars on first use, remain available, and emit a line
in `notices` on tool responses (`report`, `leads`, `structure`) so the agent
knows their results are untested and best-effort.

If you hit a problem with a language other than Rust, please open an issue or
contribute a tested adapter for it.

## Always matches your code

On every tool call except `health` (and cursor continuations reading a stored
page), SlugAudit walks the project, BLAKE3-hashes every regular file on disk,
compares against `files.content_hash` in SQLite, updates changed rows, adds new
files, and removes deleted ones. Stored `files.content` is never re-read or
re-hashed out of SQLite. There is no file watcher.

- **Opt-in per repo:** a project is enabled iff `<root>/.planning/slugaudit/`
  exists (`project_control` `"action": "on"` creates it; `"action": "off"`
  deletes it). Everything inside `.planning/slugaudit/` is disposable local
  scratch; a missing or damaged database is deleted and rebuilt from disk.
- **Session-only model state:** findings (`finding`, `finding_read`,
  `finding_clear`), custom mechanics (`mechanic`), and the session `profile`
  last one server process and are wiped when SlugAudit restarts.

## Install & CLI

```sh
curl -fsSL https://raw.githubusercontent.com/SlugThugLabs/slugaudit/main/install.sh | bash
```

Connect to your coding client or manage the installation:

```sh
slugaudit connect claude            # or: cursor, gemini, agy, codex, opencode, ...
slugaudit disconnect claude         # alias: slugaudit remove claude
slugaudit install                   # install this binary to ~/.slugthug/slugaudit/
slugaudit grammar <list|install>    # list or install optional non-Rust grammars
slugaudit menu                      # interactive setup menu
slugaudit update                    # update to the latest Linux x86_64 release
```

## Tools

| Tool | What it does |
|---|---|
| `report` | Project snapshot: file and language counts, skipped files, import resolution, notices |
| `query` | Read-only SQL (`SELECT` / `WITH`) over `files`, `evidence`, `dependency_edges`, and `revisions` |
| `structure` | Tree-sitter pattern search over one file or all files of a language |
| `leads` | Unranked counts and file/line rows for mechanical facts (`unwrap`/`expect`/`panic`/`unreachable`/`todo`/`unimplemented`/`assert*` calls, comment markers, diagnostics, credential-shaped paths) |
| `mechanic` | Register, list, or remove session Tree-sitter counters for `leads` |
| `finding`, `finding_read`, `finding_clear` | Record, read, and clear this session's review findings, bound to each file's content hash |
| `project_control` | Turn indexing `on` or `off` for a project |
| `health` | Read-only server, database, and result-store state (never syncs) |

## Documentation

Start at [docs/README.md](docs/README.md):

- [Getting started](docs/getting-started.md)
- [Tools and resources](docs/tools.md)
- [How it works](docs/how-it-works.md)
- [Languages](docs/languages.md)
- [Performance](docs/performance.md)
- [Configuration](docs/configuration.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Privacy and security](docs/privacy-and-security.md)
- [Releasing](docs/RELEASING.md)

## License

See [LICENSE](LICENSE).

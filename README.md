# SlugAudit

An MCP server that indexes your codebase into a local SQLite database, so an
AI coding agent can query symbols, call sites, imports, comments, diagnostics
and file text instead of reading files one by one.

## Language support

**Rust is the only tested language.**

Other languages are parsed with tree-sitter grammars that are downloaded the
first time a repo contains them. They usually work, but they are **untested**:
treat their results as best-effort. When a repo contains an untested language,
SlugAudit says so in its tool responses (the `notices` field), so the agent
knows which answers to double-check.

If you hit a problem with a language other than Rust, please open an issue, or
contribute a tested adapter for it.

## Always matches your code

Before every tool call, SlugAudit re-hashes every file in the project and
updates anything that changed, so every answer reflects the code on disk at
the moment of the call. There is no file watcher and no cached answer that can
go stale. The database is disposable: if it is damaged or out of date,
SlugAudit deletes it and rebuilds it.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/SlugThugLabs/slugaudit/main/install.sh | bash
```

Then connect it to your agent:

```sh
slugaudit connect claude      # or: cursor, gemini, agy
slugaudit menu                # interactive setup
slugaudit update              # update to the latest release
```

## Tools

| Tool | What it does |
|---|---|
| `report` | Project snapshot: file and language counts, skipped files, import resolution, notices |
| `query` | Read-only SQL over the index (files, evidence, dependency edges) |
| `structure` | Tree-sitter pattern search over one file or the whole project |
| `leads` | Counts and file/line rows for mechanical facts (unwrap/expect/panic calls, TODO markers, diagnostics) |
| `finding`, `finding_read`, `finding_clear` | Record, read and clear this session's review findings, tied to a file's current content |
| `mechanic` | Register your own tree-sitter counters |
| `project_control` | Turn indexing on or off for a project |
| `health` | Server and database state |

## Documentation

Start at [docs/README.md](docs/README.md). Main pages:

- [Getting started](docs/getting-started.md)
- [Tools and resources](docs/tools.md)
- [How it works](docs/how-it-works.md)
- [Languages](docs/languages.md)
- [Configuration](docs/configuration.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Privacy and security](docs/privacy-and-security.md)

## License

See [LICENSE](LICENSE).

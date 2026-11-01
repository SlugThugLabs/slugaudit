# Getting started

SlugAudit is a local MCP server. Your AI agent starts it over stdio and calls its tools. This page takes you from install to a first answer.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/SlugThugLabs/slugaudit/main/install.sh | bash
```

What the script does:

- Puts the binary at `~/.slugthug/slugaudit/slugaudit`. Set `SLUGTHUG_HOME` to use another base directory for `slugaudit install` and `connect` (the script itself always uses `~/.slugthug/slugaudit`).
- On Linux x86_64 it downloads the release binary and checks it against the release `SHA256SUMS`. It fails if the check fails.
- On other systems it needs `cargo`. It builds from the repository (if you run it inside a checkout) or runs `cargo install --git`.
- If a binary was already there, the old one is kept as `slugaudit.prev` in the same directory. To revert, copy it back.

The script does not edit your `PATH`. Add the directory yourself:

```sh
export PATH="$HOME/.slugthug/slugaudit:$PATH"
```

If you already have a `slugaudit` binary from somewhere else, `slugaudit install` copies it to the same place. When run in a terminal, it offers to add the directory to your PATH. It never edits a shell config without asking.

## Connect an agent

```sh
slugaudit connect claude      # Claude Code
slugaudit connect cursor      # Cursor
slugaudit connect gemini      # Gemini CLI
slugaudit connect agy         # Antigravity
```

`connect` registers the installed binary (`~/.slugthug/slugaudit/slugaudit`) as an MCP server named `slugaudit`. If no installed copy exists, it registers the binary you ran. Run `connect` again to update an existing registration.

With no agent name, `slugaudit connect` lists the agents it detects on your machine and asks you to choose.

Per agent:

| Agent | Command | How it registers |
|---|---|---|
| Claude Code | `claude` | runs `claude mcp add` |
| Cursor | `cursor` | writes `~/.cursor/mcp.json` |
| Gemini CLI | `gemini` | runs `gemini mcp add` with user scope. Gemini shows the server as Disabled until you trust the folder. |
| Antigravity | `agy` | runs `agy mcp add` |

All supported agent ids:

`agy`, `gemini`, `claude`, `hermes`, `copilot`, `codex`, `opencode`, `amp`, `bob`, `grok`, `goose`, `openhands`, `kimi`, `droid`, `crush`, `q`, `cursor`, `windsurf`, `pi`, `omp`, `warp`, `claude-desktop`, `zed`.

Aliases: `claude-code`, `antigravity`, `github-copilot`, `oh-my-pi`.

`goose` and `crush` cannot be configured automatically. `connect goose` and `connect crush` refuse and print the lines to paste by hand.

Each client command that `connect` runs is stopped after 15 seconds.

To remove the registration:

```sh
slugaudit disconnect claude    # also: slugaudit remove claude
```

For any other MCP client, use `slugaudit menu`, choice 4 ("other client"), to get the settings to paste. The server command is the installed `slugaudit` binary with no arguments.

## The menu

```sh
slugaudit menu
```

An interactive menu with six choices: install the binary, connect an agent, disconnect an agent, show settings for another client, run the server (for testing), exit.

## Update

```sh
slugaudit update
```

Checks the latest GitHub release. If it is newer, downloads it, verifies the checksum, and replaces the installed binary. The old binary is kept as `slugaudit.prev`. It uses `curl` and fetches only the Linux x86_64 release. On other platforms it stops with an error. Your project indexes are not touched. They rebuild themselves when needed.

## Other commands

```sh
slugaudit version     # also --version, -V, -v
slugaudit help        # also --help, -h
slugaudit             # run the MCP server on stdio (your agent does this)
```

## Turn a project on

SlugAudit does nothing in a project until you enable it. Ask your agent to do it, or have it call the `project_control` tool:

```json
{ "action": "on", "path": "/home/you/code/myproject" }
```

`path` is the project root. If you leave it out, the server's current directory is used. The directory you give is the root. SlugAudit does not search upward for a git root when enabling.

What `on` does:

- Creates the directory `<project>/.planning/slugaudit/`. This directory is the on switch.
- Creates the database `<project>/.planning/slugaudit/project.db` and indexes every file.
- Adds `.planning/slugaudit/` to the project's `.gitignore` (see below).
- Returns an `import` summary: `added`, `modified`, `deleted`, `unchanged`, `skipped`, `revision_id`.

A project counts as enabled while the directory `.planning/slugaudit/` exists. A symlink at that path does not count. Every other tool finds the project by looking for that directory in the given path and its parent directories. So you can pass any file or folder inside the project.

An optional `profile` (`adaptive`, `audit`, `lean`) applies to this session only. It is held in memory until the server exits and is not saved. See [configuration](configuration.md).

To turn a project off, call `project_control` with `"action": "off"`. This deletes the whole `.planning/slugaudit/` directory and then `.planning` if that leaves it empty. See [tools](tools.md).

## `.gitignore`

On `on`, SlugAudit appends `.planning/slugaudit/` to the `.gitignore` in the project root, unless a rule that already ignores it is present. The response has a `gitignore` field:

| `status` | Meaning |
|---|---|
| `added` | The rule was appended. The file is created if missing. |
| `already_ignored` | An equivalent rule was there. The file is untouched. |
| `not_written` | The file was left alone and `error` says why. The project is still enabled. |

Existing content is kept byte for byte. The line ending style of the file is respected. `.gitignore` is not written if it is a symlink, is not a regular file, or is larger than 1 MiB. `off` never edits `.gitignore`.

## First call

Start with `report`:

```json
{ "path": "/home/you/code/myproject" }
```

It returns a snapshot: file counts, languages, evidence counts, parser failures, open findings, dependency-edge counts, and `notices`. The first `report` of a server session also carries a short note about mechanics. Then call `leads` with only `path` to get counts of mechanical facts. These two calls need no source reads.

If the project is not enabled you get the error code `project_not_enabled` with the exact `project_control` call to make. See [troubleshooting](troubleshooting.md).

## Where the database lives

```
<project>/.planning/slugaudit/project.db        the index (SQLite, WAL mode)
<project>/.planning/slugaudit/project.db-wal    SQLite sidecar files
<project>/.planning/slugaudit/project.db-shm
<project>/.planning/slugaudit/findings-<id>.db       this session's findings (SQLite, WAL mode)
<project>/.planning/slugaudit/findings-<id>.db-wal   SQLite sidecar files
<project>/.planning/slugaudit/findings-<id>.db-shm
<project>/.planning/slugaudit/findings-<id>.lock     held while the session is alive
<project>/.planning/slugaudit/results/          stored pages of paged answers
```

The database is disposable scratch. If it is damaged or out of date, SlugAudit deletes it and rebuilds it on the next call. If you delete `project.db` yourself, the next call rebuilds it. Everything in the directory is scratch. A session's findings file is deleted by the next server process that touches the project once the session has ended, so findings never outlive the session that wrote them. The project is turned off only by deleting the directory itself (`project_control` with `off`), after which the next call fails with `project_not_enabled`. See [how it works](how-it-works.md).

Next: [tools](tools.md), [how it works](how-it-works.md).

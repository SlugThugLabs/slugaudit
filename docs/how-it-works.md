# How it works

## The one rule

Every tool answer matches the code on disk at the moment of the call.

To keep that rule, SlugAudit does this before it answers a call:

1. Walks the project and lists the files it indexes.
2. Reads and hashes every one of them (BLAKE3).
3. Compares each hash with the stored one. Files that changed are re-parsed and rewritten. New files are added. Missing files are removed.
4. Re-checks that the text stored in the database still hashes to its stored hash. A row edited behind SlugAudit's back is repaired from the file.
5. Answers from the updated database.

There is no file watcher and no timestamp shortcut. There is no cached answer that can go stale. If nothing changed, nothing is rewritten and no new revision is made.

Two exceptions to know:

- A call that continues a paged answer (`cursor`) reads from the stored copy of the first call. It does not sync. Its `revision_id` is the first call's.
- `health` never syncs.

Files that are skipped (see below) are identified by size and modification time, not by content. If such a file changes without changing its size or modification time, the change is not noticed.

## The database

- One SQLite file per project: `<project>/.planning/slugaudit/project.db`, in WAL mode.
- It is scratch. Anything derived from your source can be rebuilt from your source.
- SlugAudit deletes and rebuilds it when it is corrupt, has an incompatible schema or contract version, or was built for a different project root (for example, the project folder was copied or unpacked elsewhere). The call that did this carries `database_discard` with `findings_discarded` and `cause`.
- The one thing in the database that cannot be rebuilt from source is findings. A rebuild loses them. Hence the `findings_discarded` count.
- If `project.db` is a symlink, SlugAudit refuses to open it.
- A new database file is created owner-only (mode 0600 on Unix).
- If you delete `project.db` yourself, the project is turned off. Call `project_control` with `on` again.

Only the revision "now" exists. `files`, `evidence` and `dependency_edges` hold the current state. A changed file is replaced in place. A deleted file is removed.

## What gets indexed

SlugAudit walks the project root and indexes every regular file it finds, with these exceptions.

Never indexed:

| What | Detail |
|---|---|
| `.git` | Any path with a `.git` component. |
| `.planning/slugaudit/` | SlugAudit's own directory. A sibling like `.planning/slugaudit-notes/` is indexed. |
| Scratch and editor files | Names ending in `.claude_output.txt`, `.tmp`, `.bak`, `.swp` or `~`, and names starting with `scratch.`. |
| Ignored files | Files matched by `.gitignore`, `.ignore`, `.git/info/exclude` and your global gitignore. Nested ignore files apply in their folder. Ignore files above the project root are not applied. The ignore rules apply even when the project is not a git repository. |
| Symlinks | Symlinks are not followed and not indexed. A symlink to a file or to a folder does not appear in the index at all, not even as a skipped file. |
| Files with a non-UTF-8 name | They cannot be stored. They are only counted: `report.completeness.unrepresentable_path_count`. |

Hidden files and folders (`.github/`, `.env.example`) are indexed.

If the walk cannot list a folder, or cannot apply an ignore file, the whole call fails. SlugAudit does not index a partial set, because a missing file would look like a deleted one.

Indexed with a row, but treated specially:

| Case | What is stored | How it shows up |
|---|---|---|
| Binary file (a NUL byte in the first 8,000 bytes) | A row with `file_kind = 'binary'`. Hash, size. No content. | Counted in `report`. No evidence. |
| Larger than `max_file_bytes` | A row with `file_kind = 'skipped'`, `skip_reason = 'oversize'`. No content. | See below. |
| Unreadable (open or read failed) | `skipped`, `skip_reason = 'unreadable'`. | See below. |
| Size unknown (stat failed) | `skipped`, `skip_reason = 'stat_failed'`, `byte_len` NULL. | See below. |
| Content is not valid UTF-8 | `skipped`, `skip_reason = 'not_utf8'`. | See below. |
| Larger than `max_parse_bytes` (2 MiB by default) but within `max_file_bytes` | Stored with its text. Not parsed. `parse_error_reason = 'oversize_for_parse'`. | Searchable by `query` on `content`. No evidence. Counted in `files_not_parsed`. |
| Parse took longer than the per-file allowance (5 s by default) | Stored with its text. `parse_error_reason = 'parse_timeout'`. | Same as above, possibly with partial evidence. |
| Prose: `.md`, `.markdown`, `.mdx`, `.rst`, `.adoc`, `.asciidoc`, `.org`, `.txt`, `.text`, and ignore lists such as `.gitignore`, `.dockerignore` | Stored with its text as `language = 'prose'`. Never parsed. | Searchable. No evidence. Not a parser failure. |
| Language the grammar pack does not know | Stored with its text. | `ContentOnly`. No evidence. |

### The size cap

`max_file_bytes` depends on the profile:

| Profile | Default `max_file_bytes` |
|---|---|
| `lean` | 32 MiB |
| `audit` | 256 MiB |
| `adaptive` (the default) | one tenth of the machine's RAM, at least 256 MiB and at most 1,000,000,000 bytes |

So on a machine with 16 GiB of RAM or more, the default cap is 1,000,000,000 bytes. Nothing larger than 1,000,000,000 bytes can be stored, whatever you configure. Change the cap in [configuration](configuration.md).

There is also a cap on the total size of all files read in one call, `max_total_import_bytes` (2 GiB on `lean`, 32 GiB on `audit`, on `adaptive` half of the available RAM but at least 4 GiB). If a project is bigger than that, the call fails with a message that says how many bytes it would load and what the limit is.

### What a skipped file looks like in answers

A skipped file is not invisible. It has a row in `files` and counts everywhere files are counted:

- `files.file_kind = 'skipped'`, with `skip_reason` set, `content` NULL, and `byte_len` set (NULL when the stat failed).
- `report.completeness.skipped_file_count` and `skipped_by_reason` count it. `index_partial` is `true`.
- `SELECT content FROM files WHERE path = '...'` returns NULL for it.
- It has no evidence, so `leads` and `structure` cannot see it.
- `finding` on it fails with `file_not_indexed`.
- `project_control` `on` counts it in `import.skipped`.

To list them:

```sql
SELECT path, skip_reason, byte_len FROM files WHERE file_kind = 'skipped'
```

Check `index_partial` before you treat an absence as proof.

## What the index holds

For each indexed text file:

- `files`: the path, language, size, full text, hash, and how parsing went.
- `evidence`: Structure, Import, Export, Comment, Docstring, Symbol, Diagnostic and Call rows with line spans.
- `dependency_edges`: import edges between files, marked `Resolved`, `Unresolved` or `External`.

Rust has its own import resolver. Other languages go through a generic one, and `syntax_unmodeled = 1` on an edge marks import syntax that was not parsed. `report.import_resolution` shows the Resolved / Unresolved / External split, so check it before you trust dependency traversal.

## Limits and failures

Every limit is bounded and every failure is reported, not hidden:

- A sync that runs too long fails with `sync_budget_exhausted` (120 s on `lean`, 1200 s otherwise).
- A query that runs too long fails and returns nothing.
- Pages say when an answer is incomplete (`page.complete`, `page.completeness`, `limited_by`).
- Counts say when they are a floor (`files_not_parsed`, `index_partial`, `evidence_capped_file_count`).

See [performance](performance.md) for what the freshness check costs.

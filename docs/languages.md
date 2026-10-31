# Languages

## Rust is the only tested language

SlugAudit's tests cover Rust. Nothing else is tested.

Other languages are parsed with Tree-sitter grammars from the `tree-sitter-language-pack` library (version 1.18.0). The library downloads a grammar the first time a project contains that language. These languages usually work, but they are untested. Treat their results as best-effort and double-check the ones that matter.

This is not a promise that Rust is perfect either. It means Rust is the language the test suite exercises.

## How SlugAudit tells you

`report`, `leads` and `structure` return a `notices` array. It holds one line for each untested code language the answer covers. It is empty when there is nothing to say.

The exact text:

```
python: 12 files parsed with an untested grammar (downloaded on first use). Only Rust is tested; treat python results as best-effort.
```

- In `report`, the count is the language's file count, written as `1 file parsed` or `N files parsed`.
- In `leads` and `structure`, there is no count: `python: parsed with an untested grammar (downloaded on first use). Only Rust is tested; treat python results as best-effort.`
- Prose files (Markdown, text) are not a code language and get no notice.
- Rust gets no notice.

`report.languages[].tested` says which languages are tested (`true` only for Rust). `leads` `mechanic_counts[].covers` says which languages each count read.

## What is parsed

| Language class | What happens |
|---|---|
| Rust | Parsed. Tested. |
| Other languages the pack knows | Parsed with a downloaded grammar. Untested. Notice in the response. |
| Prose (`.md`, `.rst`, `.adoc`, `.org`, `.txt`, and ignore-list files like `.gitignore`) | Stored and searchable by `content`. Never parsed. |
| A language the pack does not know, or a file with no detected language | Stored and searchable by `content`. No evidence. |
| A file over `max_parse_bytes` (2 MiB by default) | Stored and searchable. Not parsed. See [how it works](how-it-works.md). |

The built-in call-site mechanics of `leads` (`unwrap_calls`, `expect_calls`, `panic_calls`, `todo_calls`, `assert_calls`) read calls in rust, python, javascript, typescript, tsx, go, java and c. They match by Rust callee names. A matching call in another language is counted too. `covers` in `mechanic_counts` names the languages each count read. A language missing from `covers` was not checked, so a zero there means nothing.

For `structure` and `mechanic`, give the grammar name exactly as the index stores it (`python`, `rust`, `javascript`, `tsx`). `report.languages` lists the names in your project.

## Offline use

The first time a project contains a language whose grammar is not on your machine, the library downloads it. The downloads come from GitHub (see [privacy and security](privacy-and-security.md)).

If the download fails (for example, you are offline):

1. The failed fetch can block the call for about 60 seconds. That is the library's own HTTP timeout, which SlugAudit cannot change.
2. The affected files are stored with their full text but no evidence. Their `files.parser_availability` is `LoadFailed`. `report.parser_failure_files` lists them with the reason.
3. SlugAudit stops trying to download for 10 minutes (the cool-down in `src/parse/fetch_gate.rs`). During the cool-down, files whose grammar is not already on your machine are recorded as `LoadFailed` with the reason `grammar fetch skipped: an earlier fetch failed and is in cool-down; it is retried after the cool-down or on the next server start`. They do not wait for another timeout.
4. A grammar that is already on your machine still loads normally during the cool-down.
5. After 10 minutes, or after a server restart, the next call retries the files marked `LoadFailed`. Once the network is back, they are parsed and the evidence appears.

The cool-down is per server process.

The text of those files stays searchable the whole time:

```sql
SELECT path FROM files WHERE language = 'python' AND content LIKE '%eval(%'
```

To see what is affected:

```sql
SELECT path, parser_availability, parse_error_reason FROM files
WHERE parser_availability = 'LoadFailed'
```

While `leads` shows `files_not_parsed` above 0, every count is a floor.

To use a language with no network at all, get the grammar onto the machine while you have a connection (open any project that contains that language once). Grammars are cached on disk by the library, under your platform cache directory in `tree-sitter-language-pack/v1.18.0/libs`. The library reads `TREE_SITTER_LANGUAGE_PACK_CACHE_DIR` to change the cache base directory. This variable belongs to the library, not to SlugAudit.

## Report a problem

If a language other than Rust gives a wrong or missing result, open an issue on the project's GitHub repository (`SlugThugLabs/slugaudit`). Include:

- the language and the file extension;
- the tool and the request you made;
- the response, including `notices` and `files_not_parsed`;
- the smallest source file that shows the problem.

## Contribute an adapter

A tested adapter for a language other than Rust is welcome (see the [README](../README.md)). Until a language has tests, it stays in the untested group and its responses keep the `notices` line.

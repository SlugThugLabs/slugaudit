# `evidence.payload` keys

`evidence.payload` is a JSON object whose shape depends on `evidence.kind`
(`src/model/evidence.rs::EvidenceKind`). This is the key/type reference for
every kind, so the shape does not have to be rediscovered by trial.

Three rules:

- **Line numbers are 1-based, inclusive.** `evidence.start_line` and
  `evidence.end_line` match what `cat -n` shows; they are `NULL` when the
  evidence has no span.
- **Read keys with `json_extract`** (`json_extract(e.payload, '$.name')`),
  never with `LIKE` on the payload text.
- **Every key listed for a kind is always present.** A value the extractor
  did not have is `null` (or an empty array), never a missing key. `NULL`
  from `json_extract` therefore means "present and null" for these keys.

## Structure

Pack structural items (`src/evidence/normalize_builders.rs::flatten_structure`).

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `kind` | string | pack structure kind as a Rust `Debug` name, e.g. `Function`, `Method`, `Class`, `Struct`, `Interface`, `Enum`, `Module`, `Trait`, `Impl` |
| `name` | string \| null | declared name, absent for anonymous items |
| `visibility` | string \| null | e.g. `pub`, `public`, `private` |
| `decorators` | array of string | `[]` when none |
| `signature` | string \| null | full signature text when the pack provides it |

## Import

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `source` | string | module or path text being imported |
| `items` | array of string | named imports; `[]` for a bare import |
| `alias` | string \| null | e.g. the `np` in `import numpy as np` |
| `is_wildcard` | boolean | `true` for `import *` / `use foo::*` |

## Export

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `name` | string | exported name |
| `kind` | string | pack export kind as a Rust `Debug` name: `Named`, `Default`, `ReExport` |

## Comment

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `text` | string | raw comment text |
| `kind` | string | pack comment kind as a Rust `Debug` name: `Line`, `Block`, `Doc` |
| `associated_node` | string \| null | syntax node the comment is attached to |

## Docstring

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `text` | string | raw docstring text |
| `format` | string | pack format as a Rust `Debug` name: `PythonTripleQuote`, `JSDoc`, `Rustdoc`, `GoDoc`, `JavaDoc` |
| `associated_item` | string \| null | item the docstring documents |

## Symbol

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `name` | string | symbol name |
| `kind` | string | pack symbol kind as a Rust `Debug` name: `Variable`, `Constant`, `Function`, `Class`, `Type`, `Interface`, `Enum`, `Module` |
| `type_annotation` | string \| null | explicit annotation from the source |
| `doc` | string \| null | documentation comment attached to the symbol |

## Diagnostic

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `message` | string | diagnostic text |
| `severity` | string | one of `error`, `warning`, `info` (lowercase; mapped in `src/evidence/normalize.rs::severity_text`) |

## Call

Rust files only — `Call` evidence has exactly one producer, the Rust walker
in `src/evidence/calls.rs`. On a project with no Rust files the `Call` count
is zero because nothing was checked, not because nothing is there.

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `language` | string | always `"rust"` in this revision |
| `callee_text` | string | callee as written, e.g. `self.conn.query_row` |
| `callee_name` | string | last simple name of the callee, e.g. `query_row` |
| `caller` | string \| null | enclosing function/method name; `null` at file scope |
| `resolution` | string | `macro`, `qualified-syntactic`, or `unresolved-syntactic` — a syntax classification, not a compiler-grade call graph |
| `confidence` | string | `high` for the first two, `medium` for `unresolved-syntactic` |

## Example

```sql
SELECT f.path, e.start_line, e.payload
FROM evidence e
JOIN files f ON e.file_id = f.id
WHERE e.kind = 'Call'
  AND json_extract(e.payload, '$.callee_name') = 'unwrap'
ORDER BY f.path, e.start_line;
```

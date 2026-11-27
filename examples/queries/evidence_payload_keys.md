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
- **Every baseline key listed for a kind is always present.** A value the
  extractor did not have is `null` (or an empty array), never a missing key,
  except for optional language-specific enrichments noted below
  (`Structure`'s `rust_mod_path`, attached only on external Rust `mod foo;`
  declarations, and `Diagnostic`'s `original_item_count` / `kept_item_count`
  when per-file evidence truncation fires).

## Structure

Structural declarations extracted directly from the `tree-sitter-rust` CST for
Rust (`src/evidence/rust_items.rs` and `src/evidence/rust_mod_paths.rs`) and
from the language pack (`src/evidence/normalize.rs::flatten_structure`) for
other languages.

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `kind` | string | structure kind name: `Function`, `Method` (including Rust `impl`/`trait` methods and trait `function_signature_item` declarations), `Class`, `Struct`, `Union`, `Interface`, `Enum`, `Module`, `Trait`, `Impl`, `Macro` (`macro_rules!`), `Namespace` |
| `name` | string \| null | declared name (`"Trait for Type"` or `"Type"` on Rust `impl` blocks), `null` for anonymous items |
| `visibility` | string \| null | visibility modifier slice on Rust items (`pub`, `pub(crate)`, `pub(super)`, `pub(in ...)`), or Elixir (`public`, `private`); `null` when private in Rust or in languages where visibility is not extracted |
| `decorators` | array of string | outer `#[...]` attribute slices in source order on Rust items (`[]` when absent, and inner `#![...]` attributes are never included); `[]` for other languages |
| `signature` | string \| null | declaration header up to the body start when available (including body-less Rust trait method signatures and `macro_rules!` headers) |
| `doc_comment` | string \| null | preceding outer doc comment (`///` and `/** */` in Rust, including across outer `#[...]` attributes — inner `//!` / `/*!` module docs never attach to items; `/** */` in Java/JS/TS; `null` otherwise) |
| `rust_mod_path` | object \| null | present only on external (body-less `mod foo;`) Rust `Module` items (`NULL` via `json_extract` on inline `mod foo { ... }` and all non-`Module` items): `{"path_attr": string \| null, "inline_dir": string, "inline_depth": number, "inline_segments": string[]}`. `path_attr` carries any explicit `#[path = "..."]` / `#[cfg_attr(..., path = "...")]` value (or implicit segment under an inline `#[path]` ancestor) and is `null` on plain `mod foo;` |

## Import

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `source` | string | module or path text being imported (`use ...;` and `extern crate ...;` in Rust) |
| `items` | array of string | named imports (Python and JS/TS); `[]` for Rust, Go, Java, Kotlin, Elixir, bare imports, and `GenericWalker` imports |
| `alias` | string \| null | e.g. the `np` in `import numpy as np` (Python and single-specifier/namespace JS/TS imports; `null` otherwise) |
| `is_wildcard` | boolean | `true` for `import *` / `use foo::*` |

## Export

JavaScript, TypeScript, and TSX (`export_statement`) only. In Rust and every
other language `Export` has zero rows — query Rust's public API via
`Structure` / `Symbol` rows with `json_extract(e.payload, '$.visibility') = 'pub'`
and `Import` rows whose `source` starts with `pub use`.

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `name` | string | exported name |
| `kind` | string | pack export kind as a Rust `Debug` name: `Named`, `Default`, `ReExport` |

## Comment

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `text` | string | raw comment text |
| `kind` | string | comment kind: `Line`, `Block`, `Doc` (`///`, `//!`, `/**`, `/*!` in Rust) |
| `associated_node` | string \| null | syntax node kind immediately following the comment on an adjacent line |

## Docstring

Python only (module, class, and function string-literal docstrings). In Rust,
Java, JavaScript, and TypeScript, doc comments (`///`, `//!`, `/** ... */`) are
indexed as `Comment` rows with `kind = 'Doc'` and outer doc comments are
attached to `Structure` (`$.doc_comment`) and `Symbol` (`$.doc`), never as
`Docstring` rows.

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `text` | string | raw docstring text |
| `format` | string | pack format as a Rust `Debug` name: `PythonTripleQuote` or `Other("python-single-quote")` |
| `associated_item` | string \| null | item the docstring documents |

## Symbol

Named symbol declarations (`origin = 'PackSymbol'`, including Rust `fn` / trait
method signatures, `struct`, `union`, `enum`, `trait`, `mod`, `type`,
`associated_type`, `const`, and `macro_rules!`) plus variable, parameter,
closure-parameter, `if let` / `for` pattern, `static`, enum-variant, and
struct/union/class field declarations (`origin = 'GenericWalker'`). Rust
destructuring bindings (`let (a, b) = ...;`, `let Foo { x, y: z } = ...;`) emit
one `Symbol` row per bound identifier.

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `name` | string | symbol name |
| `kind` | string | symbol kind: `Variable`, `Constant`, `Function`, `Class`, `Struct`, `Trait`, `Type` (including Rust `type`, `associated_type`, and `union`), `Interface`, `Enum`, `Module`, `Macro` (`macro_rules!`), or `Other("Field")` (struct/union/class fields; Rust `enum_variant` and immutable `static` items use `Constant`, and `static mut` items use `Variable`) |
| `visibility` | string \| null | Rust declaration/field visibility modifier (`pub`, `pub(crate)`, `pub(super)`, ...); `null` when private, on local bindings/parameters, or in other languages |
| `type_annotation` | string \| null | explicit `: <type>` (or `type` alias RHS / `associated_type` bounds) slice when written in source (populated on Rust `const`, `static`, `type`, `associated_type`, `field_declaration`, `parameter`, typed closure parameters, and typed `let` bindings, plus pack-supported `PackSymbol` declarations); `null` when omitted/inferred or in non-Rust `GenericWalker` bindings |
| `doc` | string \| null | preceding outer doc comment attached to the symbol (`///` and `/** */` in Rust on `PackSymbol` declarations, `static` items, `enum_variant`s, and struct/union `field_declaration`s, including across outer `#[...]` attributes — inner `//!` / `/*!` module docs never attach; `/** */` in Java/JS/TS on `PackSymbol` declarations; `null` on local `let`/parameter bindings and other languages) |

## Diagnostic

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `message` | string | diagnostic text |
| `severity` | string | one of `error`, `warning`, `info` (lowercase) |

## Call

Rust, Python, JavaScript, TypeScript, TSX, Go, Java and C files only — the
Rust walker in `src/evidence/calls.rs` (origin `RustCallWalker`, macro
arguments, statement blocks, and item macros included) and one embedded call
query per other language (`examples/calls/<language>.scm`, origin `RawTree`).
In any other language the `Call` count is zero because nothing was checked, not
because nothing is there; `leads` lists the languages each count read in
`covers`.

| Key | JSON type | Value |
| :-- | :-- | :-- |
| `language` | string | the file's language: `rust`, `python`, `javascript`, `typescript`, `tsx`, `go`, `java`, or `c` |
| `callee_text` | string | callee as written, e.g. `self.conn.query_row` |
| `callee_name` | string | last simple name of the callee, e.g. `query_row` |
| `caller` | string \| null | enclosing function/method name; `null` at file scope or inside an anonymous function (closure, lambda, arrow function) |
| `resolution` | string | `macro`, `qualified-syntactic` (a `::` path), or `unresolved-syntactic` — a syntax classification, not a compiler-grade call graph |
| `confidence` | string | `high` for the first two, `medium` for `unresolved-syntactic` |

## Example

```sql
SELECT f.path, e.start_line, e.payload
FROM evidence e
JOIN files f ON e.file_id = f.id
WHERE e.kind = 'Call'
  AND json_extract(e.payload, '$.callee_name') = 'expect'
ORDER BY f.path, e.start_line;
```

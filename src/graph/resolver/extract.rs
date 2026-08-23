//! Raw import-statement → reference extraction for the generic resolver.
//!
//! `extract_reference` is the one hand-rolled parser that recognizes the
//! ~20 import syntaxes the generic resolver models (`import`/`from`/
//! `using`/`use`/`open`/`#include`/quoted forms across languages). It is
//! order-sensitive and every branch is guarded by grammar-matrix tests, so
//! it lives alone where it is individually testable instead of buried
//! inside the resolver struct. The `LanguageResolver` impl in `generic.rs`
//! delegates here.

use super::generic::GenericResolverConfig;
use super::js::extract_js_reference;
use super::path_helpers::{extract_quoted_string, starts_with_python_dot_prefix};
use crate::graph::reference::ImportReference;

/// Strips a leading `keyword` when it is a whole word — i.e. immediately
/// followed by whitespace (Python's grammar requires a separator between
/// `import`/`from` and the module, and `fromfoo` must never match `from`).
/// Returns the remainder after the keyword, or `None`.
fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let after = text.strip_prefix(keyword)?;
    after.chars().next().is_some_and(char::is_whitespace).then_some(after)
}

/// Extracts the module/reference text from one raw import statement, or
/// `None` when the text matches no modeled import syntax — callers treat
/// that the same as an unparseable import, not an error.
pub(super) fn extract_reference(config: &GenericResolverConfig, raw: &str) -> Option<ImportReference> {
    let trimmed = raw.trim();

    // Bare `.` is a Python-style relative import (current package).
    if trimmed == "." {
        return Some(ImportReference {
            text: ".".to_owned(),
        });
    }

    // JS/TS relative: `./utils`, `../lib/helper`.
    if trimmed.starts_with("./") || trimmed.starts_with("../") {
        return Some(ImportReference {
            text: trimmed.to_owned(),
        });
    }

    // Python relative: `.bar`, `..pkg.mod` — starts with `.` but not
    // `./` or `../` (which are JS/TS-style filesystem paths).
    if starts_with_python_dot_prefix(trimmed) {
        return Some(ImportReference {
            text: trimmed.to_owned(),
        });
    }

    // Python `from X import ...` → `X`. Accepts any single whitespace
    // token (space or tab) between `from` and the module, matching
    // Python's grammar. Caught initially by the proptest in
    // `super::proptest`, which generated `from\tmod import\tbar` and
    // surfaced that the prior `"from "` literal-space strip was too
    // strict; `"fromfoo"` (no separator) must still be rejected, which
    // `strip_keyword` enforces.
    if let Some(after_from) = strip_keyword(trimmed, "from") {
        let module = after_from.split_whitespace().next()?;
        // Quoted module after `from` isn't valid Python — skip it so
        // we don't match Go's `import "fmt"` or similar.
        if !module.starts_with('"') && !module.starts_with('\'') {
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }
        return None;
    }

    // JS/TS `import ... from 'path'` — extracted via the dedicated
    // language helper so the quote-gating lives next to its matchers.
    // Token-gate on the literal `from` keyword, not a substring match:
    // a Python (and several other languages') bare-module import whose
    // name happens to contain `from` — `import from_util`,
    // `import foo_bar.from_baz`, etc. — is legal, must reach the
    // generic `import X` branch below, and the prior substring gate
    // was shadowing it (silently returning `None` instead of the
    // module path). The token check uses whitespace boundaries, which
    // is the same `split_whitespace().any(…)` shape the bare-name
    // candidate check below uses — consistent with how we treat
    // "does this look like a keyword or a token?" elsewhere.
    //
    // When `from` IS a real keyword but the statement has no quoted
    // path (`import x from broken`), extraction returns `None` and
    // we deliberately drop here; falling through would let the
    // generic `import X` branch lift the impossible `x` out as a
    // bare module name and call it External, which is worse than
    // honest `None` (it would mislabel unresolved syntax as a
    // third-party import).
    if trimmed.split_whitespace().any(|token| token == "from")
        && let Some(reference) = extract_js_reference(trimmed)
    {
        return Some(reference);
    }
    // If `from` was a real keyword but nothing was quoted, drop —
    // continuing would silently mislabel the next token as External.
    if trimmed.split_whitespace().any(|token| token == "from") {
        return None;
    }

    // Dart/Solidity-style quoted imports: `import '../util.dart';` or
    // `import "./Ownable.sol";`. Only explicit relative paths are
    // extracted — so Go's `import "fmt"` and bare-package imports
    // stay `Unresolved` exactly as before. Also lifts JS/TS side-effect
    // imports (`import './a.js';`) out of `Unresolved` into real
    // relative resolution, which the Python branch below never reached.
    if trimmed.starts_with("import")
        && let Some(quoted) = extract_quoted_string(trimmed)
    {
        // Relative paths resolve against the importing file.
        if quoted.starts_with("./") || quoted.starts_with("../") {
            return Some(ImportReference { text: quoted });
        }
        // Dart `package:`/`dart:` URIs are external by definition.
        if quoted.starts_with("package:") || quoted.starts_with("dart:") {
            return Some(ImportReference { text: quoted });
        }
        // Quoted non-relative imports (Go's `import "fmt"`) stay
        // unparsed so they remain Unresolved, not misresolved.
        return None;
    }

    // C# `using System.IO;` / julia `using LinearAlgebra` — strip the
    // keyword and trailing semicolon, skipping a `using static` marker.
    if let Some(after_using) = strip_keyword(trimmed, "using") {
        let mut tokens = after_using.split_whitespace();
        let mut module = tokens.next()?.trim_end_matches(';');
        if module == "static" {
            module = tokens.next()?.trim_end_matches(';');
        }
        return Some(ImportReference {
            text: module.to_owned(),
        });
    }

    // Perl/PHP `use strict;` / `use Foo\Bar;` — must come after the
    // `using` branch since "using…" starts with "use".
    if let Some(after_use) = strip_keyword(trimmed, "use") {
        let module = after_use.split_whitespace().next()?.trim_end_matches(';');
        // Perl `use My::Module;` and PHP `use Foo\Bar\Baz;` namespace
        // separators become module dots so the module-path resolver
        // can find `My/Module.pm` / `Foo/Bar/Baz.php` when the file
        // exists; bare names (`use strict;`) are untouched and stay
        // External.
        let module = module.replace("::", ".").replace('\\', ".");
        return Some(ImportReference {
            text: module.to_owned(),
        });
    }

    // OCaml `open Printf` / `open Util`.
    if let Some(after_open) = strip_keyword(trimmed, "open") {
        let module = after_open.split_whitespace().next()?;
        return Some(ImportReference {
            text: module.to_owned(),
        });
    }

    // C/C++ `#include "local.h"` / `#include <stdio.h>` — the path is
    // a filesystem path relative to the including file, so it is
    // prefixed with `./` to reach the relative-resolution branch.
    if let Some(rest) = trimmed.strip_prefix("#include") {
        let inner = rest.trim();
        let path = inner
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .or_else(|| inner.strip_prefix('<').and_then(|r| r.strip_suffix('>')));
        if let Some(path) = path {
            return Some(ImportReference {
                text: format!("./{}", path.trim()),
            });
        }
        return None;
    }

    // Python `import X` / `import X as Y` → `X`. Accepts any single
    // whitespace token (space or tab) between `import` and the module,
    // matching the same fix applied to the `from` branch above. Without
    // this, `import\tos` returns None even though it's valid Python.
    if let Some(after_import) = strip_keyword(trimmed, "import") {
        let module = after_import.split_whitespace().next()?;
        // Skip quoted imports (Go, etc.).
        if !module.starts_with('"') && !module.starts_with('\'') {
            return Some(ImportReference {
                text: module.to_owned(),
            });
        }
        return None;
    }

    // Bare package name (e.g. `react`, `os`, `numpy`). Only extract
    // if this resolver treats bare names as external (Python, JS,
    // etc.) — the resolution step will mark them as `External`.
    // Reject anything with spaces, quotes, or other statement
    // syntax to avoid matching full import statements like Go's
    // `import "fmt"`.
    if config.bare_names_are_external
        && !trimmed.is_empty()
        && !trimmed.starts_with('.')
        && !trimmed.contains(config.module_separator)
        && !trimmed.contains(' ')
        && !trimmed.contains('"')
        && !trimmed.contains('\'')
    {
        return Some(ImportReference {
            text: trimmed.to_owned(),
        });
    }

    None
}

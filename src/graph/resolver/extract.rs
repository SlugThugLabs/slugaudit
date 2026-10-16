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
use super::path_helpers::extract_quoted_string;
use super::python::extract_python_reference;
use crate::graph::reference::ImportReference;

/// Extracts the module/reference text from one raw import statement, or
/// `None` when the text matches no modeled import syntax — callers treat
/// that the same as an unparseable import, not an error.
pub(super) fn extract_reference(
    config: &GenericResolverConfig,
    raw: &str,
) -> Option<ImportReference> {
    let trimmed = raw.trim();

    // JS/TS relative: `./utils`, `../lib/helper`.
    if trimmed.starts_with("./") || trimmed.starts_with("../") {
        return Some(ImportReference {
            text: trimmed.to_owned(),
        });
    }

    // JS/TS `import ... from 'path'`. This must be dispatched before the
    // Python branch below, which claims every statement beginning with
    // `import`: JavaScript's `import x from 'p'` and Python's `import x`
    // share that prefix, and only the quoted path tells them apart. Ordering
    // it after the Python branch let `extract_python_reference` return the
    // local binding name — `import { b } from './b.js'` extracted `{`.
    //
    // Gating on the `import` prefix as well as the `from` token is what keeps
    // Python's `from X import Y`, which contains a bare `from` token, out of
    // this branch so it still reaches the Python handling that owns relative
    // imports.
    //
    // A `from` token with no quoted path returns `None` deliberately: falling
    // through would hand `import x from broken` to the bare-package branch,
    // which would lift the impossible binding `x` out as a module name and
    // label it External — worse than an honest `None`.
    if trimmed.starts_with("import") && trimmed.split_whitespace().any(|token| token == "from") {
        return extract_js_reference(trimmed);
    }

    if let Some(reference) = extract_python_reference(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_quoted_import(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_csharp_or_julia(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_perl_or_php(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_ocaml(trimmed) {
        return Some(reference);
    }

    if let Some(reference) = extract_include(trimmed) {
        return Some(reference);
    }

    extract_bare_package(config, trimmed)
}

fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let after = text.strip_prefix(keyword)?;
    after
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then_some(after)
}

fn extract_quoted_import(trimmed: &str) -> Option<ImportReference> {
    if !trimmed.starts_with("import") {
        return None;
    }
    let quoted = extract_quoted_string(trimmed)?;
    if quoted.starts_with("./")
        || quoted.starts_with("../")
        || quoted.starts_with("package:")
        || quoted.starts_with("dart:")
    {
        return Some(ImportReference { text: quoted });
    }
    None
}

fn extract_csharp_or_julia(trimmed: &str) -> Option<ImportReference> {
    let after_using = strip_keyword(trimmed, "using")?;
    let mut tokens = after_using.split_whitespace();
    let mut module = tokens.next()?.trim_end_matches(';');
    if module == "static" {
        module = tokens.next()?.trim_end_matches(';');
    }
    Some(ImportReference {
        text: module.to_owned(),
    })
}

fn extract_perl_or_php(trimmed: &str) -> Option<ImportReference> {
    let after_use = strip_keyword(trimmed, "use")?;
    let module = after_use.split_whitespace().next()?.trim_end_matches(';');
    let module = module.replace("::", ".").replace('\\', ".");
    Some(ImportReference { text: module })
}

fn extract_ocaml(trimmed: &str) -> Option<ImportReference> {
    let after_open = strip_keyword(trimmed, "open")?;
    let module = after_open.split_whitespace().next()?;
    Some(ImportReference {
        text: module.to_owned(),
    })
}

fn extract_include(trimmed: &str) -> Option<ImportReference> {
    let rest = trimmed.strip_prefix("#include")?;
    let inner = rest.trim();
    let path = inner
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| inner.strip_prefix('<').and_then(|r| r.strip_suffix('>')))?;
    Some(ImportReference {
        text: format!("./{}", path.trim()),
    })
}

fn extract_bare_package(config: &GenericResolverConfig, trimmed: &str) -> Option<ImportReference> {
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

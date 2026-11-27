//! Unit tests for [`super::catalog`].

use super::catalog::{compile, is_lower_snake, load};
use crate::model::ResourceLimits;
use crate::project::{CompiledMechanic, MechanicEntry, SessionMechanics};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};
use std::path::Path;
use std::sync::Arc;

#[test]
fn lowercase_digits_and_underscore_are_allowed() -> TestResult {
    ensure(is_lower_snake("abc_123"), "is_lower_snake(\"abc_123\")")?;
    ensure(is_lower_snake("_"), "is_lower_snake(\"_\")")?;
    Ok(())
}

#[test]
fn uppercase_punctuation_space_and_non_ascii_are_refused() -> TestResult {
    for text in ["Abc", "a-b", "a b", "a.b", "caf\u{e9}"] {
        ensure(!is_lower_snake(text), format!("{text:?} must be refused"))?;
    }
    Ok(())
}

#[test]
fn the_empty_string_has_no_offending_byte() -> TestResult {
    ensure(is_lower_snake(""), "is_lower_snake(\"\")")?;
    Ok(())
}

#[test]
fn load_reuses_precompiled_query_arc_and_scopes_by_project_root() -> TestResult {
    let session = SessionMechanics::default();
    let limits = ResourceLimits::default();
    let entry = MechanicEntry {
        id: "rust_fn".to_owned(),
        language: "rust".to_owned(),
        query: "(function_item name: (identifier) @name)".to_owned(),
        note: "functions".to_owned(),
    };
    let compiled = compile(&entry, &limits).check("compile entry")?;
    let root_a = Path::new("/tmp/project-a");
    let root_b = Path::new("/tmp/project-b");
    session
        .with_entries(root_a, |entries| {
            entries.push(CompiledMechanic {
                entry,
                compiled: Arc::clone(&compiled),
            });
        })
        .check("register entry")?;

    let first = load(&session, root_a).check("first load")?;
    let second = load(&session, root_a).check("second load")?;
    let other = load(&session, root_b).check("other project load")?;

    let q1 = &first.custom.first().check("first custom")?.query;
    let q2 = &second.custom.first().check("second custom")?.query;
    ensure(
        Arc::ptr_eq(q1, q2) && Arc::ptr_eq(q1, &compiled),
        "mechanic_catalog::load must reuse the pre-compiled Arc<Query> directly",
    )?;
    ensure_eq(&other.custom.len(), &0, "other project has no mechanics")?;
    Ok(())
}

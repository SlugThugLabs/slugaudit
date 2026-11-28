//! Unit tests for [`super`] (`load_grammar`, alias/prose checks, `run_budgeted_captures`).

use super::*;
use crate::parse::fetch_gate::test_controls::{GateResetGuard, set_grammar_cached};
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};
use tree_sitter::Parser;

#[test]
fn load_grammar_blocks_uncached_fetch_during_cooldown_and_allows_cached_grammar() -> TestResult {
    let _guard = GateResetGuard::new();
    set_grammar_cached(false);
    record_fetch_failure().check("record fetch failure")?;

    let blocked = load_grammar("rust").check_err("uncached grammar blocked in cooldown")?;
    ensure(
        matches!(blocked, GrammarLoadError::CooldownBlocked),
        format!("expected CooldownBlocked, got {blocked:?}"),
    )?;

    set_grammar_cached(true);
    load_grammar("rust").check("cached grammar still loads during cooldown")?;
    Ok(())
}

#[test]
fn alias_and_prose_languages_are_refused_with_actionable_reasons() -> TestResult {
    let alias_err =
        check_non_alias_non_prose_language("shell").check_err("alias shell must fail")?;
    ensure(
        alias_err.contains("use `bash`"),
        format!("expected canonical hint, got: {alias_err}"),
    )?;

    let prose_err =
        check_non_alias_non_prose_language("markdown").check_err("prose markdown must fail")?;
    ensure(
        prose_err.contains("is prose") && prose_err.contains("not parsed"),
        format!("expected prose explanation, got: {prose_err}"),
    )?;

    check_non_alias_non_prose_language("rust").check("rust is accepted")?;
    Ok(())
}

#[test]
fn run_budgeted_captures_detects_query_cursor_match_limit_overflow() -> TestResult {
    let grammar = load_grammar("rust").check("rust grammar")?;
    let mut parser = Parser::new();
    parser.set_language(&grammar).check("set rust grammar")?;

    let mut body = String::from("pub fn burst() {\n");
    for _ in 0..370 {
        body.push_str("    let _ = 0;\n");
    }
    body.push_str("}\n");
    let tree = parser.parse(&body, None).check("parse rust snippet")?;
    let query = Query::new(
        &grammar,
        "(block (let_declaration) @hit (let_declaration) @hit)",
    )
    .check("compile combinatorial pattern")?;

    let mut cursor = QueryCursor::new();
    let mut seen = 0usize;
    let outcome = run_budgeted_captures::<String>(
        &mut cursor,
        &query,
        tree.root_node(),
        body.as_bytes(),
        None,
        |_, _| {
            seen = seen
                .checked_add(1)
                .ok_or_else(|| "seen count overflowed".to_owned())?;
            Ok(())
        },
    )
    .check("scan finishes")?;

    ensure(!outcome.timed_out, "no deadline was set")?;
    ensure(
        outcome.match_limit_exceeded,
        format!(
            "370 sibling let declarations with 2-capture pattern must exceed {QUERY_CURSOR_MATCH_LIMIT} match states (saw {seen})"
        ),
    )?;
    ensure_eq(
        &outcome.timed_out,
        &false,
        "timed_out stays false on match-limit overflow",
    )?;
    Ok(())
}

#[test]
fn run_budgeted_captures_stops_immediately_when_deadline_expires_during_drain() -> TestResult {
    let grammar = load_grammar("rust").check("rust grammar")?;
    let mut parser = Parser::new();
    parser.set_language(&grammar).check("set rust grammar")?;

    let body = "pub fn f() { let a = 1; let b = 2; }\n";
    let tree = parser.parse(body, None).check("parse rust snippet")?;
    let query = Query::new(&grammar, "(let_declaration) @hit").check("compile query")?;

    let mut cursor = QueryCursor::new();
    let mut seen = 0usize;
    let outcome = run_budgeted_captures::<String>(
        &mut cursor,
        &query,
        tree.root_node(),
        body.as_bytes(),
        Some(std::time::Instant::now()),
        |_, _| {
            seen = seen
                .checked_add(1)
                .ok_or_else(|| "seen count overflowed".to_owned())?;
            Ok(())
        },
    )
    .check("scan finishes")?;

    ensure(
        outcome.timed_out,
        "expired deadline stops capture drain immediately",
    )?;
    ensure_eq(&seen, &0, "zero captures delivered after expired deadline")?;
    Ok(())
}

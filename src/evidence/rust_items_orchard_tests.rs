//! Regression tests for Rust Orchard (`tree-sitter-language-pack` 1.28.0) CST extraction.

use super::tests::{extract_rust_evidence, find_structure};
use crate::model::{EvidenceKind, EvidencePayload};
use slugaudit_test_check::{Check, TestResult, ensure_eq};

#[test]
fn rust_orchard_spans_imports_comments_macros_v2_and_blockers() -> TestResult {
    let source = concat!(
        "/// Doc on use.\n",
        "#[cfg(unix)]\n",
        "pub use crate::foo;\n",
        "\n",
        "/// Doc on fn.\n",
        "#[inline]\n",
        "pub fn f() {}\n",
        "\n",
        "/** Doc comment. */\n",
        "/* non-doc blocker */ pub fn blocked() {}\n",
        "\n",
        "pub macro m2($x:expr) { $x }\n",
    );
    let items = extract_rust_evidence(source)?;

    let import = items
        .iter()
        .find(|item| item.kind == EvidenceKind::Import)
        .check("Import item")?;
    let imp = match &import.payload {
        EvidencePayload::Import(imp) => Some(imp),
        _ => None,
    }
    .check("expected ImportPayload")?;
    ensure_eq(
        &imp.source.as_str(),
        &"pub use crate::foo;",
        "Import.source excludes attrs",
    )?;
    let imp_span = match &import.span {
        crate::model::SpanAvailability::Present(span) => Some(span),
        _ => None,
    }
    .check("expected Present import span")?;
    ensure_eq(
        &imp_span.start.line,
        &2_u32,
        "Import start_line on pub use (0-based)",
    )?;

    let fn_item = items
        .iter()
        .find(|item| {
            item.kind == EvidenceKind::Structure
                && matches!(&item.payload, EvidencePayload::Structure(s) if s.name.as_deref() == Some("f"))
        })
        .check("Structure f")?;
    let fn_span = match &fn_item.span {
        crate::model::SpanAvailability::Present(span) => Some(span),
        _ => None,
    }
    .check("expected Present fn span")?;
    ensure_eq(
        &fn_span.start.line,
        &6_u32,
        "fn start_line on pub fn (0-based)",
    )?;

    let doc_comment = items
        .iter()
        .find_map(|item| match &item.payload {
            EvidencePayload::Comment(c) if c.text.contains("Doc on fn.") => Some(c),
            _ => None,
        })
        .check("Doc on fn comment")?;
    ensure_eq(
        &doc_comment.associated_node.as_deref(),
        &Some("attribute_item"),
        "doc before #[inline] associates with attribute_item",
    )?;

    let source_no_attr = "/// Direct doc.\npub fn direct() {}\n";
    let direct_items = extract_rust_evidence(source_no_attr)?;
    let direct_doc = direct_items
        .iter()
        .find_map(|item| match &item.payload {
            EvidencePayload::Comment(c) if c.text.contains("Direct doc.") => Some(c),
            _ => None,
        })
        .check("Direct doc comment")?;
    ensure_eq(
        &direct_doc.associated_node.as_deref(),
        &Some("function_item"),
        "doc directly before pub fn associates with function_item",
    )?;

    let blocked = find_structure(&items, "blocked")?;
    ensure_eq(
        &blocked.doc_comment,
        &None,
        "non-doc blocker prevents doc attachment",
    )?;

    let m2 = find_structure(&items, "m2")?;
    ensure_eq(
        &m2.kind.as_str(),
        &"Macro",
        "macro_definition_v2 emits Macro Structure",
    )?;
    Ok(())
}

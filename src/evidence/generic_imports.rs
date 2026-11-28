//! Generic import extraction for non-Rust languages the pack's own import pass
//! doesn't cover.

use super::{Extraction, WalkInput};
use crate::model::{EvidenceItem, EvidenceKind, EvidenceOrigin, EvidencePayload, ImportPayload};
use tree_sitter::Node;

fn is_import_statement_kind(kind: &str) -> bool {
    matches!(
        kind,
        "import_statement"
            | "import_from_statement"
            | "import_declaration"
            | "import_header"
            | "using_directive"
            | "using_statement"
            | "import_directive"
            | "import_specification"
            | "library_import"
            | "use_statement"
            | "namespace_use_declaration"
            | "preproc_include"
            | "open_module"
            | "pp_include_lib"
            | "import"
    )
}

/// Iterative pre-order walk emitting one `Import` evidence item per
/// import-shaped node when the language pack produced zero imports.
pub(super) fn walk_imports(
    root: Node<'_>,
    input: &WalkInput<'_>,
    out: &mut Extraction,
    counter: &mut usize,
) -> Result<(), crate::model::PositionOverflow> {
    let mut pending = vec![(root, 0u32, false)];
    while let Some((node, depth, suppressed)) = pending.pop() {
        if super::deadline_spent(input.deadline) {
            out.timed_out = true;
            break;
        }
        if depth >= crate::evidence::MAX_WALK_DEPTH {
            out.truncated = true;
            continue;
        }
        let is_import = is_import_statement_kind(node.kind());
        if is_import && !suppressed {
            match super::node_text(input.source, node.start_byte(), node.end_byte()) {
                Ok(text) => {
                    let Some(next) = counter.checked_add(1) else {
                        tracing::warn!("import key counter exhausted; extraction is partial");
                        out.truncated = true;
                        break;
                    };
                    push_import(text.trim(), node, input.lines, &mut out.items, *counter)?;
                    *counter = next;
                }
                Err(error) => {
                    tracing::warn!(%error, "import statement skipped; extraction is partial");
                    out.truncated = true;
                }
            }
        }
        let Some(child_depth) = depth.checked_add(1) else {
            out.truncated = true;
            continue;
        };
        super::push_children(&mut pending, node, |child| {
            let nested = is_import && is_import_statement_kind(child.kind());
            (child, child_depth, nested)
        });
    }
    Ok(())
}

fn push_import(
    text: &str,
    node: Node<'_>,
    lines: &crate::model::LineIndex<'_>,
    items: &mut Vec<EvidenceItem>,
    key_number: usize,
) -> Result<(), crate::model::PositionOverflow> {
    let key = format!("generic_import:{key_number}");
    items.push(EvidenceItem {
        key,
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::GenericWalker,
        span: super::node_span(node, lines, "generic import")?,
        payload: EvidencePayload::Import(ImportPayload {
            source: text.to_owned(),
            items: Vec::new(),
            alias: None,
            is_wildcard: text.contains('*'),
            inline_segments: Vec::new(),
        }),
    });
    Ok(())
}

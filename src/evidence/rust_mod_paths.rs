//! Tracks enclosing inline `mod` ancestors and builds [`RustModPathPayload`]
//! metadata for external (body-less `mod foo;`) Rust module declarations during
//! the single-pass Rust CST walk.

use super::named_children;
use super::rust_path_attr::parse_attribute_node;
use crate::model::RustModPathPayload;
use tree_sitter::Node;

/// Stack of enclosing inline `mod` segments and whether each carried an
/// explicit `#[path = "..."]` / `#[cfg_attr(..., path = "...")]` attribute.
#[derive(Default)]
pub(super) struct InlineModStack {
    segments: Vec<String>,
    has_path_flags: Vec<bool>,
}

/// Outcome of inspecting a `mod_item` CST node against [`InlineModStack`].
pub(super) enum ModItemDisposition {
    /// Inline `mod foo { ... }`: `rust_mod_path` is `None`; `pushed` indicates
    /// whether an ancestor segment was pushed onto [`InlineModStack`] and must
    /// be popped when leaving the module's subtree.
    Inline { pushed: bool },
    /// External `mod foo;`: attach `payload` to `StructurePayload::rust_mod_path`.
    External(Option<RustModPathPayload>),
}

impl InlineModStack {
    pub(super) fn segments(&self) -> &[String] {
        &self.segments
    }

    pub(super) fn pop(&mut self) {
        self.segments.pop();
        self.has_path_flags.pop();
    }

    pub(super) fn enter_mod_item(
        &mut self,
        node: Node<'_>,
        mod_name: Option<&str>,
        outer_attrs: &[Node<'_>],
        source: &str,
    ) -> ModItemDisposition {
        let mut collected_attrs: Vec<String> = Vec::new();
        for attr in outer_attrs
            .iter()
            .copied()
            .chain(named_children(node).filter(|child| child.kind() == "attribute_item"))
        {
            if let Some(path) = parse_attribute_node(attr, source)
                && !collected_attrs.contains(&path)
            {
                collected_attrs.push(path);
            }
        }
        let mut attrs_iter = collected_attrs.into_iter();
        let path_attr = attrs_iter.next();

        if named_children(node).any(|child| child.kind() == "declaration_list") {
            let has_path = path_attr.is_some();
            let segment = path_attr.or_else(|| mod_name.map(strip_raw_ident));
            if let Some(segment) = segment {
                self.segments.push(segment);
                self.has_path_flags.push(has_path);
                return ModItemDisposition::Inline { pushed: true };
            }
            return ModItemDisposition::Inline { pushed: false };
        }

        let Some(name) = mod_name else {
            return ModItemDisposition::External(None);
        };
        let extra_path_attrs: Vec<String> = attrs_iter.collect();
        let implicit_path = if path_attr.is_none() && self.has_path_flags.iter().any(|&flag| flag) {
            Some(strip_raw_ident(name))
        } else {
            None
        };
        ModItemDisposition::External(Some(RustModPathPayload {
            path_attr: path_attr.or(implicit_path),
            extra_path_attrs,
            inline_dir: self.segments.join("/"),
            inline_depth: self.segments.len(),
            inline_segments: self.segments.clone(),
        }))
    }
}

fn strip_raw_ident(name: &str) -> String {
    match name.strip_prefix("r#") {
        Some(stripped) => stripped.to_owned(),
        None => name.to_owned(),
    }
}

#[cfg(test)]
#[path = "rust_mod_paths_tests.rs"]
mod tests;

//! Pure Rust edge and import change-impact matching for incremental
//! invalidation (`Low`-confidence edges and `super::` imports).

use super::rust_super_paths::{
    ChangedRustPaths, RustScopeView, super_spec_targets_any_changed_path,
};
use crate::graph::resolver::parent_dir;
use crate::graph::{rust_module_dir, split_leading_supers, strip_trailing_glob};

fn parse_super_aware_import(raw: &str) -> Option<Vec<String>> {
    if raw.trim() == "use super::*;" {
        Some(vec!["super::*".to_owned()])
    } else {
        crate::graph::rust_use_tree::parse_rust_import(raw)
    }
}

fn is_super_spec(spec: &str) -> bool {
    spec == "super" || spec.starts_with("super::")
}

fn target_dir_affected(
    target_path: Option<&str>,
    changed: &ChangedRustPaths<'_>,
    default_if_none: bool,
) -> bool {
    match target_path {
        Some(target) => {
            let dir = parent_dir(target);
            any_changed_in_dir(&dir, changed)
        }
        None => default_if_none,
    }
}

pub(super) fn low_conf_edge_affected(
    file_path: &str,
    target_path: Option<&str>,
    raw_import: &str,
    changed: &ChangedRustPaths<'_>,
    view: &RustScopeView<'_>,
) -> bool {
    let Some(specs) = parse_super_aware_import(raw_import) else {
        return target_dir_affected(target_path, changed, false);
    };

    let mut has_non_super = false;
    let mut has_segmented_super = false;
    for spec in &specs {
        let trimmed = spec.trim();
        if is_super_spec(trimmed) {
            if super_spec_targets_any_changed_path(file_path, trimmed, &[], changed, view) {
                return true;
            }
            has_segmented_super |= split_leading_supers(trimmed)
                .is_some_and(|(_, rest)| !strip_trailing_glob(rest).is_empty());
        } else {
            has_non_super = true;
        }
    }
    if has_non_super {
        return target_dir_affected(target_path, changed, true);
    }
    if has_segmented_super {
        return target_path
            .is_some_and(|target| any_changed_in_dir(&rust_module_dir(target), changed));
    }
    // Bare `use super::*;` checks exact roots above; skip `parent_dir(crate_root_fallback)`.
    false
}

fn any_changed_in_dir(dir: &str, changed: &ChangedRustPaths<'_>) -> bool {
    if dir.is_empty() {
        !changed.exact_paths.is_empty()
    } else {
        changed.exact_paths.contains(dir) || changed.ancestor_dirs.contains(dir)
    }
}

pub(super) fn super_import_targets_any_changed_path(
    file_path: &str,
    source: &str,
    inline_segments: &[String],
    changed: &ChangedRustPaths<'_>,
    view: &RustScopeView<'_>,
) -> bool {
    let Some(specs) = parse_super_aware_import(source) else {
        return false;
    };
    specs.into_iter().any(|spec| {
        let trimmed = spec.trim();
        is_super_spec(trimmed)
            && super_spec_targets_any_changed_path(
                file_path,
                trimmed,
                inline_segments,
                changed,
                view,
            )
    })
}

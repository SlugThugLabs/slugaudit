//! Path-based Rust module and crate stem derivation for incremental edge
//! invalidation.

use super::rust_super_paths::strip_raw_prefix;
use crate::graph::file_stem;
use std::collections::HashSet;
use std::path::Path;

pub(super) fn add_path_module_stems(path: &str, stems: &mut HashSet<String>) {
    add_path_module_stems_with_boundary(path, |_| false, stems);
}

pub(super) fn add_path_module_stems_with_boundary(
    path: &str,
    is_pkg_boundary: impl Fn(&str) -> bool,
    stems: &mut HashSet<String>,
) {
    add_leaf_module_stem(path, stems);
    add_ancestor_module_stems(path, is_pkg_boundary, stems);
}

fn parent_dir_name(path: &Path) -> Option<&str> {
    path.parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
}

fn insert_normalized_ident(raw: &str, stems: &mut HashSet<String>) {
    let normalized = strip_raw_prefix(raw).replace('-', "_");
    if !normalized.is_empty() {
        stems.insert(normalized);
    }
}

fn add_leaf_module_stem(path: &str, stems: &mut HashSet<String>) {
    let stem = file_stem(path);
    let clean_stem = strip_raw_prefix(&stem);
    if clean_stem.is_empty() {
        return;
    }
    let path_ref = Path::new(path);
    if clean_stem == "mod" {
        if let Some(dir_name) = parent_dir_name(path_ref) {
            let clean_dir = strip_raw_prefix(dir_name);
            if !clean_dir.is_empty() {
                stems.insert(clean_dir.to_owned());
            }
        }
    } else if clean_stem == "lib" || clean_stem == "main" {
        if let Some(src_dir) = path_ref.parent()
            && src_dir.file_name().and_then(|name| name.to_str()) == Some("src")
            && let Some(crate_dir) = parent_dir_name(src_dir)
        {
            insert_normalized_ident(crate_dir, stems);
        }
    } else {
        stems.insert(clean_stem.to_owned());
    }
}

fn add_ancestor_module_stems(
    path: &str,
    is_pkg_boundary: impl Fn(&str) -> bool,
    stems: &mut HashSet<String>,
) {
    let mut ancestor = Path::new(path).parent();
    while let Some(dir) = ancestor {
        let Some(name) = dir.file_name().and_then(|part| part.to_str()) else {
            break;
        };
        if name == "src" || dir.to_str().is_none_or(&is_pkg_boundary) {
            break;
        }
        insert_normalized_ident(name, stems);
        ancestor = dir.parent();
    }
}

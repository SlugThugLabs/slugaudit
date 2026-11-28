// slugaudit-line-exception: approved-by=split-judge; reason=Rust mod/#[path] declaration evidence extraction and SQLite rust_mod_path metadata queries
//! Rust `mod` / `#[path = "..."]` declaration evidence extraction and SQLite
//! `rust_mod_path` metadata queries for incremental edge resolution.

use crate::graph::{RustModuleParents, WorkspaceCrates};
use crate::model::{EvidenceItem, EvidencePayload};
use rusqlite::{Transaction, params};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// External Rust `mod <name>;` declaration metadata extracted from a
/// `Structure` `Module` evidence item carrying `rust_mod_path`.
#[derive(Debug, Clone)]
pub(super) struct RustModDecl {
    pub(super) name: String,
    pub(super) path_attr: Option<String>,
    pub(super) inline_segments: Vec<String>,
}

/// Canonical `(name, path_attr, inline_dir, inline_segments)` tuple for a Rust
/// `mod` declaration with an explicit `#[path = "..."]` attribute, sorted to
/// detect actual `#[path]` changes across revisions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct RustPathAttrTuple {
    pub(super) name: String,
    pub(super) path_attr: String,
    pub(super) inline_dir: String,
    pub(super) inline_segments: Vec<String>,
}

pub(super) fn is_rust_path(relative_path: &str) -> bool {
    Path::new(relative_path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
}

pub(super) fn extract_rust_mod_info(
    evidence: &[EvidenceItem],
) -> (Vec<RustModDecl>, Vec<RustPathAttrTuple>) {
    let mut decls = Vec::new();
    let mut path_attrs = Vec::new();
    for item in evidence {
        let EvidencePayload::Structure(structure) = &item.payload else {
            continue;
        };
        let Some(mod_path) = &structure.rust_mod_path else {
            continue;
        };
        let Some(name) = &structure.name else {
            continue;
        };
        let inline_segments =
            resolve_inline_segments(&mod_path.inline_dir, Some(&mod_path.inline_segments));
        if mod_path.path_attr.is_none() && mod_path.extra_path_attrs.is_empty() {
            decls.push(RustModDecl {
                name: name.clone(),
                path_attr: None,
                inline_segments,
            });
            continue;
        }
        for attr in mod_path
            .path_attr
            .iter()
            .chain(mod_path.extra_path_attrs.iter())
        {
            path_attrs.push(RustPathAttrTuple {
                name: name.clone(),
                path_attr: attr.clone(),
                inline_dir: mod_path.inline_dir.clone(),
                inline_segments: inline_segments.clone(),
            });
            decls.push(RustModDecl {
                name: name.clone(),
                path_attr: Some(attr.clone()),
                inline_segments: inline_segments.clone(),
            });
        }
    }
    path_attrs.sort();
    (decls, path_attrs)
}

fn resolve_inline_segments(inline_dir: &str, explicit: Option<&[String]>) -> Vec<String> {
    if let Some(segments) = explicit
        && (!segments.is_empty() || inline_dir.is_empty())
    {
        return segments.to_vec();
    }
    inline_dir
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn decode_inline_segments(inline_dir: &str, inline_segments_json: Option<&str>) -> Vec<String> {
    let parsed = match inline_segments_json {
        Some(raw) => match serde_json::from_str::<Vec<String>>(raw) {
            Ok(segments) => Some(segments),
            Err(error) => {
                tracing::warn!(%error, "ignoring malformed rust_mod_path inline_segments JSON");
                None
            }
        },
        None => None,
    };
    resolve_inline_segments(inline_dir, parsed.as_deref())
}

fn decode_extra_path_attrs(extra_json: &str) -> Vec<String> {
    if extra_json == "[]" {
        return Vec::new();
    }
    match serde_json::from_str::<Vec<String>>(extra_json) {
        Ok(paths) => paths,
        Err(error) => {
            tracing::warn!(%error, "ignoring malformed rust_mod_path extra_path_attrs JSON");
            Vec::new()
        }
    }
}

/// Fast check via `idx_evidence_rust_mod_path` for whether `evidence` stores
/// any explicit Rust `#[path = "..."]` / `#[cfg_attr(..., path = "...")]`
/// metadata.
pub(super) fn any_stored_rust_path_attr_exists(
    tx: &Transaction<'_>,
) -> Result<bool, rusqlite::Error> {
    tx.prepare_cached(
        "SELECT EXISTS(\
            SELECT 1 FROM evidence \
            WHERE kind = 'Structure' AND instr(payload, 'rust_mod_path') > 0 \
              AND (json_extract(payload, '$.rust_mod_path.path_attr') IS NOT NULL \
                   OR json_array_length(COALESCE(json_extract(payload, '$.rust_mod_path.extra_path_attrs'), '[]')) > 0)\
         )",
    )?
    .query_row([], |row| row.get(0))
}

/// Loads sorted stored `#[path]` tuples grouped by declaring `files.path`
/// BEFORE `apply_deletions`, `copy_staged_files_to_tx`, or `upsert_skipped_files`
/// mutate `evidence`.
pub(super) fn load_all_stored_rust_path_attrs(
    tx: &Transaction<'_>,
) -> Result<HashMap<String, Vec<RustPathAttrTuple>>, rusqlite::Error> {
    let mut stmt = tx.prepare_cached(
        "SELECT f.path, \
                COALESCE(json_extract(e.payload, '$.name'), ''), \
                json_extract(e.payload, '$.rust_mod_path.path_attr'), \
                COALESCE(json_extract(e.payload, '$.rust_mod_path.extra_path_attrs'), '[]'), \
                COALESCE(json_extract(e.payload, '$.rust_mod_path.inline_dir'), ''), \
                json_extract(e.payload, '$.rust_mod_path.inline_segments') \
         FROM files f \
         JOIN evidence e ON e.file_id = f.id \
         WHERE f.language = 'rust' AND e.kind = 'Structure' \
           AND instr(e.payload, 'rust_mod_path') > 0 \
           AND (json_extract(e.payload, '$.rust_mod_path.path_attr') IS NOT NULL \
                OR json_array_length(COALESCE(json_extract(e.payload, '$.rust_mod_path.extra_path_attrs'), '[]')) > 0)",
    )?;
    let rows = stmt.query_map([], |row| {
        let file_path: String = row.get(0)?;
        let name: String = row.get(1)?;
        let primary: Option<String> = row.get(2)?;
        let extra_json: String = row.get(3)?;
        let inline_dir: String = row.get(4)?;
        let seg_json: Option<String> = row.get(5)?;
        let inline_segments = decode_inline_segments(&inline_dir, seg_json.as_deref());
        Ok((
            file_path,
            name,
            primary,
            extra_json,
            inline_dir,
            inline_segments,
        ))
    })?;
    let mut by_file: HashMap<String, Vec<RustPathAttrTuple>> = HashMap::new();
    for row in rows {
        let (file_path, name, primary, extra_json, inline_dir, inline_segments) = row?;
        let entry = by_file.entry(file_path).or_default();
        for path_attr in primary
            .into_iter()
            .chain(decode_extra_path_attrs(&extra_json))
        {
            entry.push(RustPathAttrTuple {
                name: name.clone(),
                path_attr,
                inline_dir: inline_dir.clone(),
                inline_segments: inline_segments.clone(),
            });
        }
    }
    for tuples in by_file.values_mut() {
        tuples.sort();
    }
    Ok(by_file)
}

pub(super) fn load_rust_module_parents(
    tx: &Transaction<'_>,
    known_paths: &HashSet<&str>,
    workspace_crates: &WorkspaceCrates,
) -> Result<RustModuleParents, rusqlite::Error> {
    let mut statement = tx.prepare_cached(
        "SELECT f.path, \
                json_extract(e.payload, '$.rust_mod_path.path_attr'), \
                COALESCE(json_extract(e.payload, '$.rust_mod_path.extra_path_attrs'), '[]'), \
                COALESCE(json_extract(e.payload, '$.rust_mod_path.inline_dir'), ''), \
                json_extract(e.payload, '$.rust_mod_path.inline_segments') \
         FROM evidence e JOIN files f ON f.id = e.file_id \
         WHERE f.language = 'rust' AND e.kind = 'Structure' \
           AND instr(e.payload, 'rust_mod_path') > 0 \
           AND (json_extract(e.payload, '$.rust_mod_path.path_attr') IS NOT NULL \
                OR json_array_length(COALESCE(json_extract(e.payload, '$.rust_mod_path.extra_path_attrs'), '[]')) > 0) \
         ORDER BY f.path, e.start_byte",
    )?;
    let rows = statement.query_map([], |row| {
        let declaring_file: String = row.get(0)?;
        let primary: Option<String> = row.get(1)?;
        let extra_json: String = row.get(2)?;
        let inline_dir: String = row.get(3)?;
        let seg_json: Option<String> = row.get(4)?;
        Ok((
            declaring_file,
            primary,
            extra_json,
            decode_inline_segments(&inline_dir, seg_json.as_deref()),
        ))
    })?;
    let mut parents = RustModuleParents::new();
    for row in rows {
        let (declaring_file, primary, extra_json, segments) = row?;
        for path_attr in primary
            .into_iter()
            .chain(decode_extra_path_attrs(&extra_json))
        {
            crate::graph::insert_rust_mod_parent_segments_in_context(
                &mut parents,
                &declaring_file,
                &path_attr,
                segments.clone(),
                known_paths,
                workspace_crates,
            );
        }
    }
    Ok(parents)
}

pub(super) fn load_file_rust_mod_decls(
    tx: &Transaction<'_>,
    file_id: i64,
) -> Result<Vec<RustModDecl>, rusqlite::Error> {
    let mut mod_stmt = tx.prepare_cached(
        "SELECT json_extract(payload, '$.name'), \
                json_extract(payload, '$.rust_mod_path.path_attr'), \
                COALESCE(json_extract(payload, '$.rust_mod_path.extra_path_attrs'), '[]'), \
                COALESCE(json_extract(payload, '$.rust_mod_path.inline_dir'), ''), \
                json_extract(payload, '$.rust_mod_path.inline_segments') \
         FROM evidence \
         WHERE file_id = ?1 AND kind = 'Structure' AND instr(payload, 'rust_mod_path') > 0 \
         ORDER BY id",
    )?;
    let rows = mod_stmt.query_map(params![file_id], |row| {
        let name: Option<String> = row.get(0)?;
        let primary: Option<String> = row.get(1)?;
        let extra_json: String = row.get(2)?;
        let inline_dir: String = row.get(3)?;
        let seg_json: Option<String> = row.get(4)?;
        Ok((
            name,
            primary,
            extra_json,
            decode_inline_segments(&inline_dir, seg_json.as_deref()),
        ))
    })?;
    let mut decls = Vec::new();
    for row in rows {
        let (Some(name), primary, extra_json, inline_segments) = row? else {
            continue;
        };
        let extras = decode_extra_path_attrs(&extra_json);
        if primary.is_none() && extras.is_empty() {
            decls.push(RustModDecl {
                name,
                path_attr: None,
                inline_segments,
            });
            continue;
        }
        for attr in primary.into_iter().chain(extras) {
            decls.push(RustModDecl {
                name: name.clone(),
                path_attr: Some(attr),
                inline_segments: inline_segments.clone(),
            });
        }
    }
    Ok(decls)
}

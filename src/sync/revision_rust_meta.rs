// slugaudit-line-exception: approved-by=split-judge; reason=Rust mod/#[path] declaration evidence extraction and SQLite rust_mod_path metadata queries
//! Rust `mod` / `#[path = "..."]` declaration evidence extraction and SQLite
//! `rust_mod_path` metadata queries for incremental edge resolution.

use crate::graph::RustModuleParents;
use crate::model::{EvidenceItem, EvidencePayload};
use rusqlite::{Transaction, params};
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
        if let Some(path_attr) = &mod_path.path_attr {
            path_attrs.push(RustPathAttrTuple {
                name: name.clone(),
                path_attr: path_attr.clone(),
                inline_dir: mod_path.inline_dir.clone(),
                inline_segments: inline_segments.clone(),
            });
        }
        decls.push(RustModDecl {
            name: name.clone(),
            path_attr: mod_path.path_attr.clone(),
            inline_segments,
        });
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

/// Reads the sorted list of previously stored `#[path = "..."]` module tuples
/// for `relative_path` BEFORE its `evidence` rows are replaced.
pub(super) fn stored_rust_path_attrs(
    tx: &Transaction<'_>,
    relative_path: &str,
) -> Result<Vec<RustPathAttrTuple>, rusqlite::Error> {
    let mut stmt = tx.prepare_cached(
        "SELECT COALESCE(json_extract(e.payload, '$.name'), ''), \
                json_extract(e.payload, '$.rust_mod_path.path_attr'), \
                COALESCE(json_extract(e.payload, '$.rust_mod_path.inline_dir'), ''), \
                json_extract(e.payload, '$.rust_mod_path.inline_segments') \
         FROM files f \
         JOIN evidence e ON e.file_id = f.id \
         WHERE f.path = ?1 AND e.kind = 'Structure' \
           AND instr(e.payload, 'rust_mod_path') > 0 \
           AND json_extract(e.payload, '$.rust_mod_path.path_attr') IS NOT NULL",
    )?;
    let mut tuples: Vec<RustPathAttrTuple> = stmt
        .query_map(params![relative_path], |row| {
            let name: String = row.get(0)?;
            let path_attr: String = row.get(1)?;
            let inline_dir: String = row.get(2)?;
            let seg_json: Option<String> = row.get(3)?;
            Ok(RustPathAttrTuple {
                name,
                path_attr,
                inline_segments: decode_inline_segments(&inline_dir, seg_json.as_deref()),
                inline_dir,
            })
        })?
        .collect::<Result<_, _>>()?;
    tuples.sort();
    Ok(tuples)
}

/// Returns `true` if any path in `paths` previously stored a non-null Rust
/// `#[path = "..."]` attribute.
pub(super) fn any_path_has_stored_rust_path_attr<'a>(
    tx: &Transaction<'_>,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<bool, rusqlite::Error> {
    let mut exists_stmt = tx.prepare_cached(
        "SELECT EXISTS(\
            SELECT 1 FROM files f \
            JOIN evidence e ON e.file_id = f.id \
            WHERE f.path = ?1 AND e.kind = 'Structure' \
              AND instr(e.payload, 'rust_mod_path') > 0 \
              AND json_extract(e.payload, '$.rust_mod_path.path_attr') IS NOT NULL\
         )",
    )?;
    for path in paths {
        if is_rust_path(path)
            && exists_stmt.query_row(params![path], |row| row.get::<_, bool>(0))?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn load_rust_module_parents(
    tx: &Transaction<'_>,
) -> Result<RustModuleParents, rusqlite::Error> {
    let mut statement = tx.prepare_cached(
        "SELECT f.path, json_extract(e.payload, '$.rust_mod_path.path_attr'), \
                COALESCE(json_extract(e.payload, '$.rust_mod_path.inline_dir'), ''), \
                json_extract(e.payload, '$.rust_mod_path.inline_segments') \
         FROM evidence e JOIN files f ON f.id = e.file_id \
         WHERE f.language = 'rust' AND e.kind = 'Structure' \
           AND instr(e.payload, 'rust_mod_path') > 0 \
           AND json_extract(e.payload, '$.rust_mod_path.path_attr') IS NOT NULL \
         ORDER BY f.path, e.start_byte",
    )?;
    let rows = statement.query_map([], |row| {
        let declaring_file: String = row.get(0)?;
        let path_attr: String = row.get(1)?;
        let inline_dir: String = row.get(2)?;
        let seg_json: Option<String> = row.get(3)?;
        Ok((
            declaring_file,
            path_attr,
            decode_inline_segments(&inline_dir, seg_json.as_deref()),
        ))
    })?;
    let mut parents = RustModuleParents::new();
    for row in rows {
        let (declaring_file, path_attr, segments) = row?;
        crate::graph::insert_rust_mod_parent_segments(
            &mut parents,
            &declaring_file,
            &path_attr,
            segments,
        );
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
                COALESCE(json_extract(payload, '$.rust_mod_path.inline_dir'), ''), \
                json_extract(payload, '$.rust_mod_path.inline_segments') \
         FROM evidence \
         WHERE file_id = ?1 AND kind = 'Structure' AND instr(payload, 'rust_mod_path') > 0 \
         ORDER BY id",
    )?;
    let rows = mod_stmt.query_map(params![file_id], |row| {
        let name: Option<String> = row.get(0)?;
        let path_attr: Option<String> = row.get(1)?;
        let inline_dir: String = row.get(2)?;
        let seg_json: Option<String> = row.get(3)?;
        Ok(name.map(|name| RustModDecl {
            name,
            path_attr,
            inline_segments: decode_inline_segments(&inline_dir, seg_json.as_deref()),
        }))
    })?;
    let mut decls = Vec::new();
    for row in rows {
        if let Some(decl) = row? {
            decls.push(decl);
        }
    }
    Ok(decls)
}

//! Enclosing `Cargo.toml` `[package]`/`[lib]` stem resolution for added Rust
//! files during incremental edge invalidation.

use crate::graph::join_dir_file;
use crate::graph::resolver::{normalize_join, parent_dir};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
struct ParsedCargoLib {
    crate_ident: Option<String>,
    lib_path: Option<String>,
}

pub(super) fn add_cargo_lib_stems(
    tx: &Transaction<'_>,
    path_to_id: &HashMap<String, i64>,
    paths: &[String],
    stems: &mut HashSet<String>,
) -> Result<(), rusqlite::Error> {
    let mut toml_cache: HashMap<String, ParsedCargoLib> = HashMap::new();
    let mut read_content = tx.prepare_cached("SELECT content FROM files WHERE id = ?1")?;
    for path in paths {
        add_enclosing_cargo_lib_stems(path, path_to_id, &mut toml_cache, &mut read_content, stems)?;
    }
    Ok(())
}

fn load_cached_cargo_lib<'a>(
    dir: &str,
    toml_path: &str,
    toml_id: i64,
    toml_cache: &'a mut HashMap<String, ParsedCargoLib>,
    read_content: &mut rusqlite::Statement<'_>,
) -> Result<&'a ParsedCargoLib, rusqlite::Error> {
    if !toml_cache.contains_key(toml_path) {
        let parsed = match read_content
            .query_row(params![toml_id], |row| row.get::<_, Option<String>>(0))
            .optional()?
            .flatten()
        {
            Some(content) => parse_cargo_lib_for_dir(dir, &content),
            None => ParsedCargoLib::default(),
        };
        toml_cache.insert(toml_path.to_owned(), parsed);
    }
    Ok(toml_cache
        .get(toml_path)
        .map_or(&EMPTY_CARGO_LIB, |parsed| parsed))
}

static EMPTY_CARGO_LIB: ParsedCargoLib = ParsedCargoLib {
    crate_ident: None,
    lib_path: None,
};

fn add_enclosing_cargo_lib_stems(
    path: &str,
    path_to_id: &HashMap<String, i64>,
    toml_cache: &mut HashMap<String, ParsedCargoLib>,
    read_content: &mut rusqlite::Statement<'_>,
    stems: &mut HashSet<String>,
) -> Result<(), rusqlite::Error> {
    let mut dir = parent_dir(path);
    loop {
        let toml_path = join_dir_file(&dir, "Cargo.toml");
        if let Some(&toml_id) = path_to_id.get(&toml_path) {
            let parsed =
                load_cached_cargo_lib(&dir, &toml_path, toml_id, toml_cache, read_content)?;
            if parsed.lib_path.as_deref() == Some(path)
                && let Some(ident) = parsed.crate_ident.as_deref()
            {
                stems.insert(ident.to_owned());
            }
        }
        if dir.is_empty() {
            break;
        }
        dir = parent_dir(&dir);
    }
    Ok(())
}

fn toml_non_empty_str<'a>(table: Option<&'a toml::Table>, key: &str) -> Option<&'a str> {
    table?
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|s| !s.is_empty())
}

fn parse_cargo_lib_for_dir(pkg_dir: &str, content: &str) -> ParsedCargoLib {
    let Ok(table) = toml::from_str::<toml::Table>(content) else {
        return ParsedCargoLib::default();
    };
    let pkg_table = table.get("package").and_then(toml::Value::as_table);
    let lib_table = table.get("lib").and_then(toml::Value::as_table);
    let crate_ident = toml_non_empty_str(lib_table, "name")
        .or_else(|| toml_non_empty_str(pkg_table, "name"))
        .map(|s| s.replace('-', "_"));
    let rel_lib = toml_non_empty_str(lib_table, "path").map_or("src/lib.rs", |custom| custom);
    let lib_path = if rel_lib.starts_with('/') {
        None
    } else {
        normalize_join(pkg_dir, rel_lib).filter(|joined| !joined.is_empty())
    };
    ParsedCargoLib {
        crate_ident,
        lib_path,
    }
}

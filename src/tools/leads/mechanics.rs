//! Built-in `leads` mechanic catalogue, SQL family fragments, and mechanic-id
//! resolution.

use crate::error_codes::{ErrorCode, invalid_params_with_code};
use crate::tools::mechanic::catalog::Catalog;
use rmcp::ErrorData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Evidence,
    Files,
}

pub(super) struct FamilySql {
    pub(super) from: &'static str,
    pub(super) path: &'static str,
    pub(super) line: &'static str,
    pub(super) order: &'static str,
    pub(super) row_id: &'static str,
    pub(super) file_key: &'static str,
    pub(super) pattern: &'static str,
}

const EVIDENCE_SQL: FamilySql = FamilySql {
    from: "evidence e JOIN files f ON e.file_id = f.id",
    path: "f.path",
    line: "COALESCE(e.start_line, 0)",
    order: "f.path, e.start_line, e.id",
    row_id: "e.id",
    file_key: "e.file_id",
    pattern: "slugaudit_path_match(f.path, ?)",
};

const FILES_SQL: FamilySql = FamilySql {
    from: "files",
    path: "path",
    line: "0",
    order: "path",
    row_id: "id",
    file_key: "path",
    pattern: "slugaudit_path_match(path, ?)",
};

impl Family {
    pub(super) const fn sql(self) -> &'static FamilySql {
        match self {
            Family::Evidence => &EVIDENCE_SQL,
            Family::Files => &FILES_SQL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reads {
    Calls,
    ParsedFiles,
    Paths,
}

pub(in crate::tools) struct Mechanic {
    pub(in crate::tools) id: &'static str,
    pub(super) family: Family,
    pub(super) reads: Reads,
    pub(super) predicate: &'static str,
    pub(super) detail: Option<&'static str>,
}

macro_rules! call_mechanic {
    ($id:literal, $match_sql:literal) => {
        Mechanic {
            id: $id,
            family: Family::Evidence,
            reads: Reads::Calls,
            predicate: concat!(
                "e.kind = 'Call' AND json_extract(e.payload, '$.callee_name') ",
                $match_sql
            ),
            detail: Some("json_extract(e.payload, '$.callee_text')"),
        }
    };
}

macro_rules! marker_glob {
    ($word:literal) => {
        concat!(
            "(' ' || json_extract(e.payload, '$.text') || ' ') GLOB '*[^A-Za-z0-9_]",
            $word,
            "[^A-Za-z0-9_]*'"
        )
    };
}

pub(in crate::tools) const MECHANICS: &[Mechanic] = &[
    call_mechanic!("unwrap_calls", "= 'unwrap'"),
    call_mechanic!("expect_calls", "= 'expect'"),
    call_mechanic!("panic_calls", "IN ('panic', 'unreachable')"),
    call_mechanic!("todo_calls", "IN ('todo', 'unimplemented')"),
    call_mechanic!(
        "assert_calls",
        "IN ('assert', 'assert_eq', 'assert_ne', 'debug_assert', 'debug_assert_eq', 'debug_assert_ne')"
    ),
    Mechanic {
        id: "diagnostics",
        family: Family::Evidence,
        reads: Reads::ParsedFiles,
        predicate: "e.kind = 'Diagnostic'",
        detail: Some("json_extract(e.payload, '$.message')"),
    },
    Mechanic {
        id: "comment_markers",
        family: Family::Evidence,
        reads: Reads::ParsedFiles,
        predicate: concat!(
            "e.kind = 'Comment' AND (",
            marker_glob!("TODO"),
            " OR ",
            marker_glob!("FIXME"),
            " OR ",
            marker_glob!("HACK"),
            " OR ",
            marker_glob!("XXX"),
            ")"
        ),
        detail: Some("json_extract(e.payload, '$.text')"),
    },
    Mechanic {
        id: "credential_files",
        family: Family::Files,
        reads: Reads::Paths,
        predicate: crate::tools::CREDENTIAL_PATH_PREDICATE,
        detail: None,
    },
];

#[derive(Clone, Copy)]
pub(in crate::tools) enum Selected {
    Builtin(&'static Mechanic),
    Registered(usize),
}

fn comma_join<'a>(ids: impl IntoIterator<Item = &'a str>) -> String {
    ids.into_iter().collect::<Vec<_>>().join(", ")
}

pub(super) fn resolve_selected(
    requested: Option<&str>,
    catalog: &Catalog,
) -> Result<Option<Selected>, ErrorData> {
    if let Some(index) = requested.and_then(|id| catalog.position(id)) {
        return Ok(Some(Selected::Registered(index)));
    }
    resolve_mechanic(requested)
        .map(|found| found.map(Selected::Builtin))
        .map_err(|mut error| {
            if !catalog.custom.is_empty() {
                let registered = comma_join(catalog.custom.iter().map(|r| r.entry.id.as_str()));
                error.message = format!("{}; registered ids: {registered}", error.message).into();
            }
            error
        })
}

pub(in crate::tools) fn resolve_mechanic(
    requested: Option<&str>,
) -> Result<Option<&'static Mechanic>, ErrorData> {
    let Some(id) = requested else {
        return Ok(None);
    };
    MECHANICS
        .iter()
        .find(|mechanic| mechanic.id == id)
        .map(Some)
        .ok_or_else(|| {
            let valid = comma_join(MECHANICS.iter().map(|mechanic| mechanic.id));
            invalid_params_with_code(
                format!("unknown mechanic `{id}`; valid ids: {valid}"),
                ErrorCode::UnknownMechanic,
            )
        })
}

impl Mechanic {
    pub(super) const fn sql(&self) -> &'static FamilySql {
        self.family.sql()
    }

    #[cfg(test)]
    pub(super) const fn order_expr(&self) -> &'static str {
        self.family.sql().order
    }

    pub(super) fn detail_expr(&self) -> String {
        match self.detail {
            Some(expr) => format!("COALESCE({expr}, '')"),
            None => "''".to_owned(),
        }
    }

    pub(super) fn where_clause(&self, matched: bool) -> String {
        if matched {
            format!(" WHERE ({}) AND {}", self.predicate, self.sql().pattern)
        } else {
            format!(" WHERE {}", self.predicate)
        }
    }
}

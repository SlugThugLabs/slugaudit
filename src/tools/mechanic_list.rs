//! `mechanic list` (and its cursor continuation) over the result store.
//!
//! The first call stores the catalogue as it is after the sync, one row per
//! mechanic in listing order, and the invalid entries beside it; a call with
//! a `cursor` reads the stored copy and nothing else, so a mechanic added or
//! removed between pages changes neither the rows nor `invalid`. No
//! continuation can register or remove anything: only the call without a
//! cursor ever reads the registry.

use super::{
    InvalidMechanic, MechanicAction, MechanicListing, MechanicRequest, MechanicResponse,
    MechanicSource, mechanic_catalog,
};
use crate::progress::ProgressSink;
use crate::store::{Cell, Materialize, ResultStoreError, StoredRow, Take};
use crate::sync::SourceSyncManager;
use crate::tools::context::internal_error;
use crate::tools::page_size;
use crate::tools::paginate::paginate;
use crate::tools::{
    Completeness, PageLimit, PageRequest, PageSpec, Produced, Served, decode_rows, page_request,
};
use rmcp::ErrorData;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

const TOOL: &str = "mechanic";

/// Stored with the result and returned unchanged on every page.
#[derive(Serialize, Deserialize)]
struct ListExtra {
    invalid: Vec<InvalidMechanic>,
    /// Whether page one's budget was the ceiling; a continuation without
    /// `page_bytes` reports the same `limited_by`.
    at_ceiling: bool,
}

impl MechanicSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::ProjectConfig => "project_config",
            Self::Builtin => "builtin",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [Self::Session, Self::ProjectConfig, Self::Builtin]
            .into_iter()
            .find(|source| source.as_str() == name)
    }
}

fn optional_text(value: &Option<String>) -> Cell {
    value
        .as_ref()
        .map_or(Cell::Null, |text| Cell::Text(text.clone()))
}

impl Materialize for MechanicListing {
    fn columns() -> &'static [&'static str] {
        &["id", "source", "note", "language", "query"]
    }

    fn to_values(&self) -> Vec<Cell> {
        vec![
            Cell::Text(self.id.clone()),
            Cell::Text(self.source.as_str().to_owned()),
            optional_text(&self.note),
            optional_text(&self.language),
            optional_text(&self.query),
        ]
    }

    fn from_values(values: &[Cell]) -> Result<Self, ResultStoreError> {
        const LAYOUT: ResultStoreError = ResultStoreError::Invalid {
            reason: "mechanic listing row layout",
        };
        let text = |cell: &Cell| match cell {
            Cell::Null => Ok(None),
            Cell::Text(text) => Ok(Some(text.clone())),
            _ => Err(LAYOUT),
        };
        let [Cell::Text(id), Cell::Text(source), note, language, query] = values else {
            return Err(LAYOUT);
        };
        Ok(Self {
            id: id.clone(),
            source: MechanicSource::parse(source).ok_or(LAYOUT)?,
            note: text(note)?,
            language: text(language)?,
            query: text(query)?,
        })
    }
}

/// The serialized JSON size of one listing, the unit `page_bytes` counts.
fn json_len(listing: &MechanicListing) -> Result<usize, ErrorData> {
    serde_json::to_vec(listing)
        .map(|bytes| bytes.len())
        .map_err(|error| internal_error("measuring a mechanic listing", error))
}

/// Serves one page of the catalogue: page one when `request` has no cursor,
/// the stored page it names otherwise.
///
/// The first row of a page is always taken, so a page can exceed
/// `page_bytes` by at most that row; `bytes_used` reports the true size. A
/// continuation without `page_bytes` keeps the budget of page one.
///
/// # Errors
///
/// As `paginate`, plus `invalid_params` for a missing `path` or `page_bytes`
/// of 0.
pub(super) fn list(
    request: &MechanicRequest,
    sink: &dyn ProgressSink,
    manager: &SourceSyncManager,
) -> Result<MechanicResponse, ErrorData> {
    let which = page_request(request.cursor.as_deref(), request.path.as_deref(), sink)?;
    let (root, limits, size) = match &which {
        PageRequest::Fresh { path, .. } => {
            let root = crate::tools::project_root(path)?;
            let limits = crate::tools::limits_for_root(&root)?;
            let size = page_size::resolve(request.page_bytes, &limits)?;
            (Some(root), limits, Some(size))
        }
        // The stored budget serves a continuation; the process limits only
        // clamp an explicit `page_bytes`, as no project is named.
        PageRequest::Cursor(_) => {
            let limits = crate::tools::process_limits_for_tool()?;
            let size = request
                .page_bytes
                .map(|bytes| page_size::resolve(Some(bytes), &limits))
                .transpose()?;
            (None, limits, size)
        }
    };
    let mut spec = PageSpec::typed::<MechanicListing>(TOOL, size.map_or(0, |size| size.bytes));
    spec.requested_page_bytes = size.map(|size| size.bytes);
    let failure = RefCell::new(None);
    let served: Served<ListExtra> = paginate(
        manager,
        &spec,
        which,
        |_tx, rows| {
            let root = root.as_deref().ok_or_else(|| {
                internal_error("listing mechanics", "a fresh list has no project root")
            })?;
            let catalog = mechanic_catalog::load(root, manager.session_mechanics(), &limits);
            for listing in catalog.listings() {
                rows.push(&listing)?;
            }
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: ListExtra {
                    invalid: catalog.invalid,
                    at_ceiling: size.is_some_and(|size| size.at_ceiling),
                },
            })
        },
        |ctx| {
            let budget = ctx.page_bytes;
            let mut used = 0usize;
            let failure = &failure;
            move |row: &StoredRow| match MechanicListing::from_values(&row.cells)
                .map_err(|error| internal_error("reading a stored mechanic listing", error))
                .and_then(|listing| json_len(&listing))
            {
                // The first row of a page is offered too and taken regardless.
                Ok(len) if used == 0 || used.saturating_add(len) <= budget => {
                    used = used.saturating_add(len);
                    Take::Yes
                }
                Ok(_) => Take::No,
                Err(error) => {
                    *failure.borrow_mut() = Some(error);
                    Take::No
                }
            }
        },
    )?;
    if let Some(error) = failure.into_inner() {
        return Err(error);
    }
    respond(served, size)
}

fn respond(
    served: Served<ListExtra>,
    requested: Option<page_size::PageSize>,
) -> Result<MechanicResponse, ErrorData> {
    let mechanics = decode_rows::<MechanicListing>(&served.rows)?;
    let bytes_used = mechanics.iter().try_fold(0usize, |sum, listing| {
        json_len(listing).map(|len| sum.saturating_add(len))
    })?;
    let mut page = served.page;
    page.bytes_used = bytes_used;
    page.limited_by = if page.complete {
        PageLimit::None
    } else if requested.map_or(served.meta.at_ceiling, |size| size.at_ceiling) {
        PageLimit::Ceiling
    } else {
        PageLimit::PageBytes
    };
    let invalid = served.meta.invalid;
    tracing::info!(
        action = "list",
        returned = page.returned,
        mechanics_invalid = invalid.len(),
        complete = page.complete,
        "mechanic call completed"
    );
    Ok(MechanicResponse {
        action: MechanicAction::List,
        id: None,
        source: None,
        mechanics,
        mechanics_invalid: invalid.len(),
        invalid,
        first_count: None,
        page: Some(page),
    })
}

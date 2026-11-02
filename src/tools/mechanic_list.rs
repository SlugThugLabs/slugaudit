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
use std::cell::{Cell as FlagCell, RefCell};

const TOOL: &str = "mechanic";

/// Stored with the result and returned unchanged on every page.
#[derive(Serialize, Deserialize)]
struct ListExtra {
    invalid: Vec<InvalidMechanic>,
}

impl MechanicSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Builtin => "builtin",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [Self::Session, Self::Builtin]
            .into_iter()
            .find(|source| source.as_str() == name)
    }
}

fn optional_text(value: Option<&str>) -> Cell {
    value.map_or(Cell::Null, |text| Cell::Text(text.to_owned()))
}

impl Materialize for MechanicListing {
    fn columns() -> &'static [&'static str] {
        &["id", "source", "note", "language", "query"]
    }

    fn to_values(&self) -> Vec<Cell> {
        vec![
            Cell::Text(self.id.clone()),
            Cell::Text(self.source.as_str().to_owned()),
            optional_text(self.note.as_deref()),
            optional_text(self.language.as_deref()),
            optional_text(self.query.as_deref()),
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
    // Only a new request names a project, so only it reads limits. A
    // continuation uses the budget and the ceiling stored with the result;
    // an explicit `page_bytes` is clamped to that ceiling.
    let mut spec = PageSpec::typed::<MechanicListing>(TOOL, 0);
    spec.requested_page_bytes = request.page_bytes;
    let fresh = match &which {
        PageRequest::Fresh { path, .. } => {
            let root = crate::tools::project_root(path)?;
            let limits = crate::tools::limits_for_root(&root)?;
            spec.page_bytes = page_size::resolve(request.page_bytes, &limits)?.bytes;
            spec.ceiling = limits.max_query_response_bytes;
            Some(limits)
        }
        PageRequest::Cursor(_) => {
            page_size::refuse_zero(request.page_bytes)?;
            None
        }
    };
    let failure = RefCell::new(None);
    // The bytes of the rows the page took, measured as they were offered.
    let used = FlagCell::new(0usize);
    let served: Served<ListExtra> = paginate(
        manager,
        &spec,
        which,
        |_tx, rows| {
            let Some(limits) = fresh.as_ref() else {
                return Err(internal_error(
                    "serving a mechanic list continuation",
                    "a continuation produces no rows",
                ));
            };
            let catalog = mechanic_catalog::load(manager.session_mechanics(), limits)?;
            for listing in catalog.listings() {
                rows.push(&listing)?;
            }
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: ListExtra {
                    invalid: catalog.invalid,
                },
            })
        },
        |ctx| {
            let budget = ctx.page_bytes;
            let (used, failure) = (&used, &failure);
            move |row: &StoredRow| match MechanicListing::from_values(&row.cells)
                .map_err(|error| internal_error("reading a stored mechanic listing", error))
                .and_then(|listing| json_len(&listing))
            {
                // The first row of a page is offered too and taken regardless.
                Ok(len) if used.get() == 0 || used.get().saturating_add(len) <= budget => {
                    used.set(used.get().saturating_add(len));
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
    respond(served, used.get())
}

fn respond(served: Served<ListExtra>, bytes_used: usize) -> Result<MechanicResponse, ErrorData> {
    let mechanics = decode_rows::<MechanicListing>(&served.rows)?;
    let mut page = served.page;
    page.bytes_used = bytes_used;
    page.limited_by = if page.complete {
        PageLimit::None
    } else if served.at_ceiling {
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

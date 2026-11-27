//! `mechanic list` (and its cursor continuation) over the result store.
//!
//! The first call stores the project's catalogue as it is after the sync, one
//! row per mechanic in listing order; a call with a `cursor` reads the stored
//! copy and nothing else, so a mechanic added or removed between pages does not
//! alter the stored listing.

use super::{
    MechanicAction, MechanicListing, MechanicRequest, MechanicResponse, MechanicSource,
    catalog as mechanic_catalog,
};
use crate::progress::ProgressSink;
use crate::store::{Cell, Materialize, ResultStoreError, StoredRow};
use crate::sync::SourceSyncManager;
use crate::tools::context::internal_error;
use crate::tools::page::PageSize;
use crate::tools::paginate::{ServedRows, byte_cut_step, paginate_rows};
use crate::tools::{Completeness, PageRequest, PageSpec, Produced, page_request};
use rmcp::ErrorData;

const TOOL: &str = "mechanic";

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
    match value {
        Some(text) => Cell::Text(text.to_owned()),
        None => Cell::Null,
    }
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
pub(super) fn list(
    request: &MechanicRequest,
    sink: &dyn ProgressSink,
    manager: &SourceSyncManager,
) -> Result<MechanicResponse, ErrorData> {
    let which = page_request(request.cursor.as_deref(), request.path.as_deref(), sink)?;
    let mut spec = PageSpec::typed::<MechanicListing>(TOOL, 0);
    spec.requested_page_bytes = request.page_bytes;
    let fresh_root = match &which {
        PageRequest::Fresh { path, .. } => {
            let root = crate::tools::project_root(path)?;
            let limits = crate::tools::limits_for_root(&root)?;
            spec.page_bytes = PageSize::resolve(request.page_bytes, &limits)?.bytes;
            spec.ceiling = limits.max_query_response_bytes;
            Some(root)
        }
        PageRequest::Cursor(_) => {
            PageSize::refuse_zero(request.page_bytes)?;
            None
        }
    };
    let served: ServedRows<MechanicListing, ()> = paginate_rows(
        manager,
        &spec,
        which,
        |_tx, rows| {
            let Some(root) = fresh_root.as_ref() else {
                return Err(internal_error(
                    "serving a mechanic list continuation",
                    "a continuation produces no rows",
                ));
            };
            let catalog = mechanic_catalog::load(manager.session_mechanics(), root.as_path())?;
            for listing in catalog.listings() {
                rows.push(&listing)?;
            }
            Ok(Produced {
                completeness: Completeness::Complete,
                extra: (),
            })
        },
        |ctx| {
            let budget = ctx.page_bytes;
            Ok(move |row: &StoredRow, taken: usize, bytes_so_far: usize| {
                let listing = MechanicListing::from_values(&row.cells)
                    .map_err(|error| internal_error("reading a stored mechanic listing", error))?;
                let len = json_len(&listing)?;
                Ok(byte_cut_step(listing, len, taken, bytes_so_far, budget))
            })
        },
    )?;
    Ok(respond(served))
}

fn respond(served: ServedRows<MechanicListing, ()>) -> MechanicResponse {
    let mechanics = served.rows;
    let page = served.page;
    tracing::info!(
        action = "list",
        returned = page.returned,
        complete = page.complete,
        "mechanic call completed"
    );
    MechanicResponse {
        action: MechanicAction::List,
        id: None,
        source: None,
        mechanics,
        first_count: None,
        page: Some(page),
    }
}

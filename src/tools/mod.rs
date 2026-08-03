//! Tool handlers orchestrate: they resolve the active project, ensure it's
//! synced, and format a response. They own no parsing, SQL, or storage
//! logic themselves — that lives in `sync`, `store`, and `evidence`.

mod context;
mod finding;
mod query;
mod query_value;
mod report;
mod structure;

pub use context::{SyncRecencyCache, ensure_synced};
pub use finding::{FindingRequest, FindingResponse, finding};
pub use query::{QueryRequest, QueryResponse, query};
pub use report::{ReportRequest, ReportResponse, report};
pub use structure::{StructureRequest, StructureResponse, structure};

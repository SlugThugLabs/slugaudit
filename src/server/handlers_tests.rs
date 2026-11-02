use super::input_schema;
use crate::tools;
use rmcp::handler::server::common::schema_for_input;
use slugaudit_test_check::{Check, TestResult};

/// The schema of `T` equals rmcp's checked `schema_for_input`, which
/// fails unless the root type is `object`.
fn matches_rmcp<T: schemars::JsonSchema + std::any::Any>() -> TestResult {
    let checked = schema_for_input::<T>().check(std::any::type_name::<T>())?;
    assert_eq!(
        input_schema::<T>(),
        checked,
        "{}",
        std::any::type_name::<T>()
    );
    Ok(())
}

#[test]
fn every_input_schema_matches_rmcp_and_has_an_object_root() -> TestResult {
    matches_rmcp::<tools::ReportRequest>()?;
    matches_rmcp::<tools::LeadsRequest>()?;
    matches_rmcp::<tools::MechanicRequest>()?;
    matches_rmcp::<tools::QueryRequest>()?;
    matches_rmcp::<tools::StructureRequest>()?;
    matches_rmcp::<tools::FindingRequest>()?;
    matches_rmcp::<tools::FindingReadRequest>()?;
    matches_rmcp::<tools::FindingClearRequest>()?;
    matches_rmcp::<tools::ProjectControlRequest>()?;
    matches_rmcp::<tools::HealthRequest>()?;
    Ok(())
}

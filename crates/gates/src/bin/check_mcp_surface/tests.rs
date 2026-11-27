use super::diff::differences;
use super::listing;
use super::surface;
use serde_json::json;
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

#[test]
fn identical_surfaces_have_no_differences() -> TestResult {
    let surface = json!({
        "tools": [{"name": "report", "title": "Project snapshot"}],
        "resources": [{"name": "query-cookbook", "uri": "slugaudit://queries/README.md"}],
    });

    ensure(
        differences(&surface, &surface).is_empty(),
        "identical surfaces report zero differences",
    )?;
    Ok(())
}

#[test]
fn a_removed_and_an_added_surface_entry_are_both_reported_by_name() -> TestResult {
    let committed = json!({"tools": [{"name": "report"}, {"name": "leads"}]});
    let live = json!({"tools": [{"name": "report"}, {"name": "health"}]});

    ensure_eq(
        &differences(&committed, &live),
        &vec![
            "tools[leads]: in the snapshot, absent now",
            "tools[health]: absent from the snapshot, present now",
        ],
        "removed and added named entries are reported by name",
    )?;
    Ok(())
}

#[test]
fn a_changed_annotation_is_reported_with_its_path_and_both_values() -> TestResult {
    let committed = json!({
        "tools": [{"name": "project_control", "annotations": {"destructiveHint": false}}],
    });
    let live = json!({
        "tools": [{"name": "project_control", "annotations": {"destructiveHint": true}}],
    });

    ensure_eq(
        &differences(&committed, &live),
        &vec!["tools[project_control].annotations.destructiveHint: snapshot false, live true"],
        "annotation value drift reports path, snapshot value, and live value",
    )?;
    Ok(())
}

#[test]
fn reordering_named_surface_entries_is_not_drift() -> TestResult {
    let committed = json!({"tools": [{"name": "report"}, {"name": "leads"}]});
    let live = json!({"tools": [{"name": "leads"}, {"name": "report"}]});

    ensure(
        differences(&committed, &live).is_empty(),
        "reordering name-keyed tool entries produces no drift",
    )?;
    Ok(())
}

#[test]
fn schema_arrays_are_compared_by_position() -> TestResult {
    let committed = json!({"tools": [{"name": "x", "inputSchema": {"required": ["a", "b"]}}]});
    let live = json!({"tools": [{"name": "x", "inputSchema": {"required": ["a"]}}]});

    ensure_eq(
        &differences(&committed, &live),
        &vec!["tools[x].inputSchema.required: snapshot has 2 entries, live has 1"],
        "positional schema arrays report length mismatches",
    )?;
    Ok(())
}

#[test]
fn every_live_tool_carries_a_title_a_description_and_both_schemas() -> TestResult {
    let tools = slugaudit_mcp_rust::server::tool_surface();

    ensure_eq(&tools.len(), &10, "the surface is the ten advertised tools")?;
    for tool in &tools {
        tool.title.as_ref().check("`tool.title` is present")?;
        tool.description
            .as_ref()
            .check("`tool.description` is present")?;
        ensure(
            !tool.input_schema.is_empty(),
            format!("{} has no input schema", tool.name),
        )?;
        let schema = tool
            .output_schema
            .as_ref()
            .check("every tool declares its output schema")?;
        ensure(
            schema.contains_key("properties"),
            format!("{} output schema has no properties", tool.name),
        )?;
    }
    Ok(())
}

#[test]
fn every_live_tool_matches_the_documented_annotation_table() -> TestResult {
    let expected = [
        ("finding", false, false),
        ("finding_clear", false, true),
        ("finding_read", true, false),
        ("health", true, false),
        ("leads", true, false),
        ("mechanic", false, false),
        ("project_control", false, true),
        ("query", true, false),
        ("report", true, false),
        ("structure", true, false),
    ];
    let tools = slugaudit_mcp_rust::server::tool_surface();

    for (name, read_only, destructive) in expected {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .check(&format!("tool `{name}` is not advertised"))?;
        let annotations = tool
            .annotations
            .as_ref()
            .check(&format!("tool `{name}` has no annotations"))?;
        ensure_eq(
            &annotations.read_only_hint,
            &Some(read_only),
            format!("`{name}` readOnlyHint"),
        )?;
        ensure_eq(
            &annotations.destructive_hint,
            &Some(destructive),
            format!("`{name}` destructiveHint"),
        )?;
    }
    Ok(())
}

#[test]
fn one_sided_lines_for_keys_and_named_entries_read_the_same_way() -> TestResult {
    let committed = json!({"a": 1, "tools": [{"name": "x"}]});
    let live = json!({"b": 1, "tools": [{"name": "y"}]});

    let mut found = differences(&committed, &live);
    found.sort();

    ensure_eq(
        &found,
        &vec![
            "a: in the snapshot, absent now",
            "b: absent from the snapshot, present now",
            "tools[x]: in the snapshot, absent now",
            "tools[y]: absent from the snapshot, present now",
        ],
        "one-sided keys and named entries use consistent phrasing",
    )?;
    Ok(())
}

#[test]
fn lhm_plugin_listing_matches_cargo_version_and_live_tool_surface() -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let live = surface::build().check("build live MCP surface")?;
    let problems = listing::check(&root, &live);

    ensure(
        problems.is_empty(),
        format!("lhm.plugin.json drifted from Cargo.toml or live surface: {problems:?}"),
    )?;
    Ok(())
}

#[test]
fn lhm_plugin_listing_detects_version_and_tool_drift() -> TestResult {
    let manifest = "[package]\nname = \"slugaudit-mcp-rust\"\nversion = \"1.1.0\"\n";
    let live = json!({
        "tools": [
            {"name": "query"},
            {"name": "report"},
        ],
        "resources": [],
    });
    let drifted = json!({
        "version": "1.0.9",
        "tools": [
            {"name": "query", "description": "SQL"},
            {"name": "query", "description": "dup"},
            {"name": "obsolete", "description": ""},
        ]
    });

    let problems = listing::check_contents(manifest, &drifted.to_string(), &live);

    ensure(
        problems
            .iter()
            .any(|p| p.contains("lhm.plugin.json.version")),
        format!("expected version mismatch in {problems:?}"),
    )?;
    ensure(
        problems.iter().any(|p| p.contains("duplicate tool entry")),
        format!("expected duplicate tool check in {problems:?}"),
    )?;
    ensure(
        problems
            .iter()
            .any(|p| p.contains("missing or empty description")),
        format!("expected empty-description check in {problems:?}"),
    )?;
    ensure(
        problems.iter().any(|p| p.contains("tools[report]")),
        format!("expected missing live tool `report` in {problems:?}"),
    )?;
    ensure(
        problems.iter().any(|p| p.contains("tools[obsolete]")),
        format!("expected stale tool `obsolete` in {problems:?}"),
    )?;
    Ok(())
}

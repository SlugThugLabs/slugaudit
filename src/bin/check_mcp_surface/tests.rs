use super::diff::differences;
use serde_json::json;

#[test]
fn identical_surfaces_have_no_differences() {
    let surface = json!({
        "tools": [{"name": "report", "title": "Project snapshot"}],
        "resources": [{"name": "query-cookbook", "uri": "slugaudit://queries/README.md"}],
    });

    assert!(differences(&surface, &surface).is_empty());
}

#[test]
fn a_removed_and_an_added_surface_entry_are_both_reported_by_name() {
    let committed = json!({"tools": [{"name": "report"}, {"name": "leads"}]});
    let live = json!({"tools": [{"name": "report"}, {"name": "health"}]});

    assert_eq!(
        differences(&committed, &live),
        vec![
            "tools[leads]: in the snapshot, absent now",
            "tools[health]: absent from the snapshot, present now",
        ]
    );
}

#[test]
fn a_changed_annotation_is_reported_with_its_path_and_both_values() {
    let committed = json!({
        "tools": [{"name": "project_control", "annotations": {"destructiveHint": false}}],
    });
    let live = json!({
        "tools": [{"name": "project_control", "annotations": {"destructiveHint": true}}],
    });

    assert_eq!(
        differences(&committed, &live),
        vec!["tools[project_control].annotations.destructiveHint: snapshot false, live true"]
    );
}

#[test]
fn reordering_named_surface_entries_is_not_drift() {
    let committed = json!({"tools": [{"name": "report"}, {"name": "leads"}]});
    let live = json!({"tools": [{"name": "leads"}, {"name": "report"}]});

    assert!(differences(&committed, &live).is_empty());
}

#[test]
fn schema_arrays_are_compared_by_position() {
    let committed = json!({"tools": [{"name": "x", "inputSchema": {"required": ["a", "b"]}}]});
    let live = json!({"tools": [{"name": "x", "inputSchema": {"required": ["a"]}}]});

    assert_eq!(
        differences(&committed, &live),
        vec!["tools[x].inputSchema.required: snapshot has 2 entries, live has 1"]
    );
}

#[test]
fn every_live_tool_carries_a_title_a_description_and_both_schemas() {
    let tools = slugaudit_mcp_rust::server::tool_surface();

    assert_eq!(tools.len(), 8, "the surface is the eight advertised tools");
    for tool in &tools {
        assert!(tool.title.is_some(), "{} has no title", tool.name);
        assert!(
            tool.description.is_some(),
            "{} has no description",
            tool.name
        );
        assert!(
            !tool.input_schema.is_empty(),
            "{} has no input schema",
            tool.name
        );
        let schema = tool
            .output_schema
            .as_ref()
            .expect("every tool declares its output schema");
        assert!(
            schema.contains_key("properties"),
            "{} output schema has no properties",
            tool.name
        );
    }
}

#[test]
fn every_live_tool_matches_the_documented_annotation_table() {
    let expected = [
        ("finding", false, false),
        ("finding_read", true, false),
        ("health", true, false),
        ("leads", true, false),
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
            .unwrap_or_else(|| panic!("tool `{name}` is not advertised"));
        let annotations = tool
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("tool `{name}` has no annotations"));
        assert_eq!(
            annotations.read_only_hint,
            Some(read_only),
            "`{name}` readOnlyHint"
        );
        assert_eq!(
            annotations.destructive_hint,
            Some(destructive),
            "`{name}` destructiveHint"
        );
    }
}

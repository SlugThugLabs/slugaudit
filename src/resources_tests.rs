use super::*;

#[test]
fn lists_every_served_resource_with_a_distinct_uri() {
    let resources = list();
    assert_eq!(
        resources.len(),
        8,
        "catalog is the cookbook, the payload-key reference, five recipes, and the schema"
    );

    let mut uris: Vec<&str> = resources
        .iter()
        .map(|resource| resource.uri.as_str())
        .collect();
    assert!(uris.contains(&"slugaudit://queries/README.md"));
    assert!(uris.contains(&"slugaudit://queries/evidence_payload_keys.md"));
    assert!(uris.contains(&"slugaudit://queries/blast_radius.sql"));
    assert!(uris.contains(&"slugaudit://schema.sql"));

    uris.sort_unstable();
    let count = uris.len();
    uris.dedup();
    assert_eq!(uris.len(), count, "uris are unique");
}

#[test]
fn serves_the_payload_key_reference_with_every_evidence_kind() {
    let contents = read("slugaudit://queries/evidence_payload_keys.md")
        .expect("the payload-key reference is served");
    let text = match contents {
        ResourceContents::TextResourceContents { text, .. } => text,
        other => panic!("expected text contents, got {other:?}"),
    };

    // The reference replaces the payload-key prose that used to sit in
    // `INSTRUCTIONS`, so it has to cover every kind that can occur.
    for kind in [
        "Structure",
        "Import",
        "Export",
        "Comment",
        "Docstring",
        "Symbol",
        "Diagnostic",
        "Call",
    ] {
        assert!(
            text.contains(&format!("## {kind}")),
            "the reference has no section for the `{kind}` kind"
        );
    }
}

#[test]
fn every_sql_recipe_compiles_and_runs_against_the_schema() {
    let connection = rusqlite::Connection::open_in_memory().expect("in-memory database");
    connection
        .execute_batch(include_str!("store/schema.sql"))
        .expect("the canonical schema applies");

    let mut executed = 0;
    for resource in list() {
        let uri = resource.uri.as_str();
        // The schema resource is the DDL applied above, not a query recipe.
        if !uri.ends_with(".sql") || uri == "slugaudit://schema.sql" {
            continue;
        }
        let Some(ResourceContents::TextResourceContents { text, .. }) = read(uri) else {
            panic!("{uri} is not served as text");
        };
        for statement in statements(&text) {
            let mut prepared = connection
                .prepare(&statement)
                .unwrap_or_else(|error| panic!("{uri} does not compile: {error}\n{statement}"));
            let mut rows = prepared
                .query([])
                .unwrap_or_else(|error| panic!("{uri} does not run: {error}\n{statement}"));
            while rows.next().expect("stepping a recipe row").is_some() {}
            executed += 1;
        }
    }
    assert!(executed >= 5, "the cookbook ships SQL recipes to run");
}

/// Splits a recipe file into statements on lines that end with `;`. A
/// recipe is one `query` call per statement, so each one has to stand
/// alone.
fn statements(text: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        current.push_str(line);
        current.push('\n');
        if line.trim_end().ends_with(';') {
            statements.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        statements.push(current);
    }
    statements
}

#[test]
fn read_contents_carry_the_mime_type_the_catalogue_declares() {
    for resource in list() {
        let contents = read(resource.uri.as_str()).expect("a listed resource is readable");
        let ResourceContents::TextResourceContents { mime_type, .. } = contents else {
            panic!("{} is not served as text", resource.uri);
        };
        assert_eq!(
            mime_type.as_deref(),
            resource.mime_type.as_deref(),
            "{} is advertised with one MIME type and read as another",
            resource.uri
        );
    }
}

#[test]
fn reads_the_schema_resource() {
    let contents = read("slugaudit://schema.sql").expect("schema is served");
    let text = match contents {
        ResourceContents::TextResourceContents { text, .. } => text,
        other => panic!("expected text contents, got {other:?}"),
    };
    assert!(
        text.starts_with("-- SlugAudit per-project schema"),
        "schema resource serves the canonical schema, not an empty stub"
    );
}

#[test]
fn unknown_uris_are_not_served() {
    assert!(read("slugaudit://nope").is_none());
    assert!(read("file:///etc/passwd").is_none());
}

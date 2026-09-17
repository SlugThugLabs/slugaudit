use super::*;

#[test]
fn lists_every_served_resource_with_a_distinct_uri() {
    let resources = list();
    assert_eq!(
        resources.len(),
        6,
        "catalog is the cookbook plus the schema"
    );

    let mut uris: Vec<&str> = resources
        .iter()
        .map(|resource| resource.uri.as_str())
        .collect();
    assert!(uris.contains(&"slugaudit://queries/README.md"));
    assert!(uris.contains(&"slugaudit://schema.sql"));

    uris.sort_unstable();
    let count = uris.len();
    uris.dedup();
    assert_eq!(uris.len(), count, "uris are unique");
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

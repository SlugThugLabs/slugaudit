use super::tests::extract_rust_evidence;
use crate::model::{EvidenceItem, EvidenceOrigin, EvidencePayload};
use slugaudit_test_check::{TestResult, ensure_eq};

fn generic_walker_vars(items: &[EvidenceItem]) -> Vec<(&str, Option<&str>)> {
    items
        .iter()
        .filter(|item| item.origin == EvidenceOrigin::GenericWalker)
        .filter_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) if s.kind == "Variable" => {
                Some((s.name.as_str(), s.type_annotation.as_deref()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn rust_destructuring_let_and_parameters_extract_every_bound_identifier_and_types() -> TestResult {
    let source = concat!(
        "struct Pair { x: i32, y: i32 }\n",
        "fn unpack(&self, (left, right): (u32, &str), Pair { x, y: renamed }: Pair) {\n",
        "    let (first, mut second): (usize, String) = make_pair();\n",
        "    let Pair { x: px, y } = build();\n",
        "    let Some(unwrapped) = maybe_val else { return; };\n",
        "    let _ = ignored_rhs_ident;\n",
        "}\n",
    );
    let items = extract_rust_evidence(source)?;
    let vars = generic_walker_vars(&items);

    ensure_eq(
        &vars,
        &vec![
            ("left", Some("(u32, &str)")),
            ("right", Some("(u32, &str)")),
            ("x", Some("Pair")),
            ("renamed", Some("Pair")),
            ("first", Some("(usize, String)")),
            ("second", Some("(usize, String)")),
            ("px", None),
            ("y", None),
            ("unwrapped", None),
        ],
        "every destructured identifier is extracted without struct types, field labels, or wildcard RHS",
    )?;
    Ok(())
}

#[test]
fn rust_match_arm_bindings_skip_paths_consts_variants_and_wildcards() -> TestResult {
    let source = concat!(
        "enum Kind { Empty, Pair(i32, i32) }\n",
        "const LIMIT: i32 = 10;\n",
        "fn inspect(opt: Option<i32>, kind: Kind, n: i32) -> i32 {\n",
        "    match opt { Some(val) if val > LIMIT => val, None => 0 };\n",
        "    match kind { Kind::Empty => 0, Kind::Pair(first, second) => first + second };\n",
        "    match n { LIMIT => 1, other @ 1..=5 => other, _ => 2 }\n",
        "}\n",
    );
    let items = extract_rust_evidence(source)?;
    let vars: Vec<&str> = generic_walker_vars(&items)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    ensure_eq(
        &vars,
        &vec!["opt", "kind", "n", "val", "first", "second", "other"],
        "match arms extract bound identifiers while skipping None, paths, consts, guards, and _",
    )?;
    Ok(())
}

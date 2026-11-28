use super::tests::extract_rust_evidence;
use crate::model::EvidencePayload;
use slugaudit_test_check::{Check, TestResult, ensure_eq};

#[test]
fn rust_import_inline_segments_tracks_enclosing_inline_mods() -> TestResult {
    let src = concat!(
        "use std::io;\n",
        "#[cfg(test)] mod tests { use super::*; }\n",
        "mod outer { mod inner { use super::super::*; } }\n",
    );
    let mut got = Vec::new();
    for item in extract_rust_evidence(src)? {
        if let EvidencePayload::Import(imp) = item.payload {
            let raw = serde_json::to_string(&imp).check("serialize import")?;
            got.push((
                imp.source,
                imp.inline_segments,
                raw.contains("inline_segments"),
            ));
        }
    }
    let want = vec![
        ("use std::io;".into(), vec![], false),
        ("use super::*;".into(), vec!["tests".into()], true),
        (
            "use super::super::*;".into(),
            vec!["outer".into(), "inner".into()],
            true,
        ),
    ];
    ensure_eq(&got, &want, "inline_segments recorded for inline mods only")
}

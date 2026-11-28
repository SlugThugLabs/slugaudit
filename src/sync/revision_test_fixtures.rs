use crate::model::{
    EvidenceItem, EvidenceKind, EvidenceOrigin, EvidencePayload, ExtractionCompleteness,
    ImportPayload, ParseOutcome, ParserAvailability, ParserRun, RustModPathPayload, SourceIdentity,
    SpanAvailability, StructurePayload,
};
use crate::sync::FileRecord;

pub(super) fn rust_run() -> ParserRun {
    ParserRun {
        availability: ParserAvailability::Available,
        outcome: ParseOutcome::Succeeded,
        completeness: ExtractionCompleteness::Full,
    }
}

pub(super) fn file_record(
    path: &str,
    hash: &str,
    content: &str,
    language: Option<&str>,
    run: ParserRun,
    evidence: Vec<EvidenceItem>,
) -> FileRecord {
    FileRecord {
        relative_path: path.to_owned(),
        is_binary: false,
        content: Some(content.to_owned()),
        identity: SourceIdentity {
            relative_path: path.to_owned(),
            content_hash: hash.to_owned(),
            hash_algorithm: "blake3".to_owned(),
        },
        byte_len: content.len() as u64,
        modified_unix_seconds: None,
        language: language.map(str::to_owned),
        language_detected: language.is_some(),
        run,
        evidence,
    }
}

pub(super) fn unparsed_record(
    path: &str,
    hash: &str,
    content: &str,
    lang: Option<&str>,
) -> FileRecord {
    let completeness = if lang.is_some() {
        ExtractionCompleteness::ContentOnly
    } else {
        ExtractionCompleteness::Unavailable
    };
    file_record(
        path,
        hash,
        content,
        lang,
        ParserRun {
            availability: ParserAvailability::Unavailable,
            outcome: ParseOutcome::NotAttempted,
            completeness,
        },
        Vec::new(),
    )
}

pub(super) fn rust_record(path: &str, hash: &str, evidence: Vec<EvidenceItem>) -> FileRecord {
    file_record(path, hash, "", Some("rust"), rust_run(), evidence)
}

pub(super) fn lib_record() -> FileRecord {
    rust_record("src/lib.rs", "hl1", vec![])
}

pub(super) fn import_evidence(index: usize, source: &str) -> EvidenceItem {
    import_evidence_in_inline_mod(&format!("import:{index}"), source, &[])
}

pub(super) fn import_evidence_in_inline_mod(
    key: &str,
    source: &str,
    inline_segments: &[&str],
) -> EvidenceItem {
    EvidenceItem {
        key: key.to_owned(),
        kind: EvidenceKind::Import,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: EvidencePayload::Import(ImportPayload {
            source: source.to_owned(),
            items: Vec::new(),
            alias: None,
            is_wildcard: source.contains('*'),
            inline_segments: inline_segments.iter().map(|s| (*s).to_owned()).collect(),
        }),
    }
}

pub(super) fn mod_evidence(index: usize, name: &str, path_attr: Option<&str>) -> EvidenceItem {
    EvidenceItem {
        key: format!("structure:{index}"),
        kind: EvidenceKind::Structure,
        origin: EvidenceOrigin::PackStructure,
        span: SpanAvailability::PackOmitted,
        payload: EvidencePayload::Structure(StructurePayload {
            kind: "Module".to_owned(),
            name: Some(name.to_owned()),
            visibility: None,
            decorators: Vec::new(),
            signature: None,
            doc_comment: None,
            rust_mod_path: Some(RustModPathPayload {
                path_attr: path_attr.map(str::to_owned),
                extra_path_attrs: Vec::new(),
                inline_dir: String::new(),
                inline_depth: 0,
                inline_segments: Vec::new(),
            }),
        }),
    }
}

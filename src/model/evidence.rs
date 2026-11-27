//! Typed source identity and extracted evidence contracts shared across
//! parsing, SQL storage, dependency resolution, and read tools.

use super::{EvidenceOrigin, Span};
use serde::{Deserialize, Serialize};

/// The `hash_algorithm` tag for a file hashed with BLAKE3.
pub(crate) const CONTENT_HASH_ALGORITHM: &str = "blake3-v1";

/// Identity and content hash for a single discovered source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    /// Relative path from the project root.
    pub relative_path: String,
    /// BLAKE3 digest (or stat-identity digest for skipped files).
    pub content_hash: String,
    /// Algorithm identifier used to compute `content_hash`.
    pub hash_algorithm: String,
}

impl SourceIdentity {
    /// Constructs a new [`SourceIdentity`] using `CONTENT_HASH_ALGORITHM`.
    #[must_use]
    pub fn new(relative_path: String, content_hash: String) -> Self {
        Self {
            relative_path,
            content_hash,
            hash_algorithm: CONTENT_HASH_ALGORITHM.into(),
        }
    }
}

/// Whether an evidence item's span was computed, or why it was not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanAvailability {
    /// A source span was computed and validated.
    Present(Span),
    /// The language pack supplied no span for the item.
    PackOmitted,
    /// A span failed local range validation and was logged as unavailable.
    NormalizerUnavailable,
    /// Derived evidence with no single source span of its own.
    DerivedEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
/// Categories of syntactic evidence stored in `evidence.kind`.
///
/// Observed syntax only — no category carries a judgment.
pub enum EvidenceKind {
    /// Structural syntax declaration (`fn`, `struct`, `enum`, `trait`, `impl`, `mod`, ...).
    Structure,
    /// Import, `use`, `include`, or equivalent dependency statement.
    Import,
    /// Exported symbol or declaration (`export_statement` in JS/TS/TSX).
    Export,
    /// Source comment (`//`, `/* */`, `///`, `//!`).
    Comment,
    /// String-literal docstring (Python module/class/function docstrings).
    Docstring,
    /// Symbol definition or binding (`let`, parameter, `const`, `static`, field, ...).
    Symbol,
    /// Parser syntax error or evidence-truncation diagnostic.
    Diagnostic,
    /// Function, method, or macro call site.
    Call,
}

/// Module-tree placement metadata attached to external Rust `mod foo;` items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustModPathPayload {
    /// Explicit `#[path = "..."]` / `#[cfg_attr(..., path = "...")]` value, or
    /// `None` for standard Rust module lookup.
    pub path_attr: Option<String>,
    /// Joined ancestor inline `mod` directory prefix (empty at file top level).
    pub inline_dir: String,
    /// Number of enclosing inline `mod` blocks.
    pub inline_depth: usize,
    /// Enclosing inline `mod` names from outermost to innermost.
    pub inline_segments: Vec<String>,
}

/// Typed `evidence.payload` for [`EvidenceKind::Structure`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructurePayload {
    /// Structural item kind (`Function`, `Method`, `Struct`, `Enum`, `Trait`, `Impl`, `Module`, ...).
    pub kind: String,
    /// Declared item name, or `None` for anonymous items.
    pub name: Option<String>,
    /// Visibility modifier (`pub`, `pub(crate)`, ...), or `None` when private/omitted.
    pub visibility: Option<String>,
    /// Outer attributes/decorators (`#[...]` in Rust), or `[]` when absent.
    pub decorators: Vec<String>,
    /// Declaration header up to the body start when available.
    pub signature: Option<String>,
    /// Preceding outer documentation comment when present.
    pub doc_comment: Option<String>,
    /// Present only on external (`mod foo;`) Rust module declarations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_mod_path: Option<RustModPathPayload>,
}

/// Typed `evidence.payload` for [`EvidenceKind::Import`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPayload {
    /// Module or path text being imported.
    pub source: String,
    /// Named imported items when extracted separately.
    pub items: Vec<String>,
    /// Local import alias when present.
    pub alias: Option<String>,
    /// `true` for wildcard/glob imports (`use foo::*`, `import *`).
    pub is_wildcard: bool,
}

/// Typed `evidence.payload` for [`EvidenceKind::Export`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPayload {
    /// Exported symbol name.
    pub name: String,
    /// Export kind (`Named`, `Default`, `ReExport`).
    pub kind: String,
}

/// Typed `evidence.payload` for [`EvidenceKind::Comment`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommentPayload {
    /// Raw comment text.
    pub text: String,
    /// Comment kind (`Line`, `Block`, `Doc`).
    pub kind: String,
    /// Immediately following syntax node kind when available.
    pub associated_node: Option<String>,
}

/// Typed `evidence.payload` for [`EvidenceKind::Docstring`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocstringPayload {
    /// Raw docstring text.
    pub text: String,
    /// Docstring delimiter format.
    pub format: String,
    /// Associated item name when available.
    pub associated_item: Option<String>,
}

/// Typed `evidence.payload` for [`EvidenceKind::Symbol`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolPayload {
    /// Bound or declared symbol name.
    pub name: String,
    /// Symbol kind (`Function`, `Struct`, `Enum`, `Trait`, `Type`, `Constant`, `Variable`, `Other("Field")`, ...).
    pub kind: String,
    /// Visibility modifier (`pub`, `pub(crate)`, ...), or `None` when private/local.
    pub visibility: Option<String>,
    /// Explicit type annotation slice when written in source.
    pub type_annotation: Option<String>,
    /// Preceding outer doc comment when present.
    pub doc: Option<String>,
}

/// Typed `evidence.payload` for [`EvidenceKind::Diagnostic`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticPayload {
    /// Diagnostic message text.
    pub message: String,
    /// Diagnostic severity (`error`, `warning`, `info`).
    pub severity: String,
    /// Original pre-cap evidence count when emitted by per-file evidence truncation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_item_count: Option<usize>,
    /// Kept evidence count when emitted by per-file evidence truncation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_item_count: Option<usize>,
}

/// Typed `evidence.payload` for [`EvidenceKind::Call`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallPayload {
    /// File language (`rust`, `python`, `javascript`, `typescript`, `tsx`, `go`, `java`, `c`).
    pub language: String,
    /// Callee expression text as written.
    pub callee_text: String,
    /// Terminal identifier of the callee.
    pub callee_name: String,
    /// Enclosing named function or method, or `None` at file/closure scope.
    pub caller: Option<String>,
    /// Syntactic call classification (`macro`, `qualified-syntactic`, `unresolved-syntactic`).
    pub resolution: String,
    /// Syntactic confidence tag (`high` or `medium`).
    pub confidence: String,
}

/// Category-specific payload serialized directly to `evidence.payload` JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EvidencePayload {
    /// Payload for [`EvidenceKind::Structure`].
    Structure(StructurePayload),
    /// Payload for [`EvidenceKind::Import`].
    Import(ImportPayload),
    /// Payload for [`EvidenceKind::Symbol`] (ordered before `Export` because
    /// `Symbol` is a strict key superset of `{name, kind}`).
    Symbol(SymbolPayload),
    /// Payload for [`EvidenceKind::Export`].
    Export(ExportPayload),
    /// Payload for [`EvidenceKind::Comment`].
    Comment(CommentPayload),
    /// Payload for [`EvidenceKind::Docstring`].
    Docstring(DocstringPayload),
    /// Payload for [`EvidenceKind::Diagnostic`].
    Diagnostic(DiagnosticPayload),
    /// Payload for [`EvidenceKind::Call`].
    Call(CallPayload),
}

impl EvidencePayload {
    /// Minimal valid payload matching `kind` for schema and storage tests.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn sample_for_kind(kind: &EvidenceKind) -> Self {
        match kind {
            EvidenceKind::Structure => Self::Structure(StructurePayload {
                kind: "Function".into(),
                name: Some("f".into()),
                visibility: None,
                decorators: Vec::new(),
                signature: None,
                doc_comment: None,
                rust_mod_path: None,
            }),
            EvidenceKind::Import => Self::Import(ImportPayload {
                source: "std::io".into(),
                items: Vec::new(),
                alias: None,
                is_wildcard: false,
            }),
            EvidenceKind::Export => Self::Export(ExportPayload {
                name: "f".into(),
                kind: "Named".into(),
            }),
            EvidenceKind::Comment => Self::Comment(CommentPayload {
                text: "// c".into(),
                kind: "Line".into(),
                associated_node: None,
            }),
            EvidenceKind::Docstring => Self::Docstring(DocstringPayload {
                text: "doc".into(),
                format: "PythonTripleQuote".into(),
                associated_item: None,
            }),
            EvidenceKind::Symbol => Self::Symbol(SymbolPayload {
                name: "x".into(),
                kind: "Variable".into(),
                visibility: None,
                type_annotation: None,
                doc: None,
            }),
            EvidenceKind::Diagnostic => Self::Diagnostic(DiagnosticPayload {
                message: "syntax error".into(),
                severity: "error".into(),
                original_item_count: None,
                kept_item_count: None,
            }),
            EvidenceKind::Call => Self::Call(CallPayload {
                language: "rust".into(),
                callee_text: "f".into(),
                callee_name: "f".into(),
                caller: None,
                resolution: "unresolved-syntactic".into(),
                confidence: "medium".into(),
            }),
        }
    }
}

/// One extracted evidence item before SQL conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceItem {
    /// Per-file item key (`<kind>:<index>`), unique within the file.
    pub key: String,
    /// Evidence category.
    pub kind: EvidenceKind,
    /// Extractor that produced the item.
    pub origin: EvidenceOrigin,
    /// Source span coordinates, or why none are available.
    pub span: SpanAvailability,
    /// Typed category-specific payload serialized to `evidence.payload`.
    pub payload: EvidencePayload,
}

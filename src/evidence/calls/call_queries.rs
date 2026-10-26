//! The embedded call queries (`examples/calls/<language>.scm`): one entry
//! per language with the grammar facts the call-site builder needs around
//! its single `@callee` capture, and the compile-once cache.
//!
//! Kept apart from `query_calls.rs` — which runs a query and turns each
//! capture into a call site — so adding a language stays a one-entry edit to
//! this table plus its `.scm`. Node kinds and field names are checked against
//! the shipped grammars by `query_calls_tests.rs`.
//!
//! A query compiles once per process. A compile failure is a defect in an
//! embedded `.scm` against the grammar the pack ships, logged once; every
//! file of that language is then reported partial, never as complete.

use std::sync::OnceLock;
use tree_sitter::{Language, Query};

/// One language's embedded call query and the grammar facts the shared
/// builder needs around its `@callee` capture.
pub(super) struct CallQuery {
    /// The pack's language name, as `files.language` stores it.
    pub(super) language: &'static str,
    /// The grammar's call node kind; the capture sits at most
    /// [`MAX_CALLEE_DEPTH`] levels below it.
    pub(super) call_kind: &'static str,
    /// Node kinds that open a caller scope, named or anonymous. The nearest
    /// one above a call names its `caller` (`null` for an anonymous one).
    pub(super) function_kinds: &'static [&'static str],
    pub(super) text: &'static str,
    /// The query compiled against the pack's grammar, filled on first use.
    compiled: OnceLock<Result<Query, CallQueryError>>,
}

/// How far above the `@callee` capture its call node may sit: `f` in `f()`
/// is a child of the call, `m` in `obj.m()` a grandchild.
pub(super) const MAX_CALLEE_DEPTH: usize = 2;

const JS_FUNCTION_KINDS: &[&str] = &[
    "function_declaration",
    "function_expression",
    "generator_function_declaration",
    "generator_function",
    "arrow_function",
    "method_definition",
];

/// Every language with an embedded call query, in a fixed order.
pub(super) static CALL_QUERIES: [CallQuery; 7] = [
    CallQuery {
        language: "python",
        call_kind: "call",
        function_kinds: &["function_definition", "lambda"],
        text: include_str!("../../../examples/calls/python.scm"),
        compiled: OnceLock::new(),
    },
    CallQuery {
        language: "javascript",
        call_kind: "call_expression",
        function_kinds: JS_FUNCTION_KINDS,
        text: include_str!("../../../examples/calls/javascript.scm"),
        compiled: OnceLock::new(),
    },
    CallQuery {
        language: "typescript",
        call_kind: "call_expression",
        function_kinds: JS_FUNCTION_KINDS,
        text: include_str!("../../../examples/calls/typescript.scm"),
        compiled: OnceLock::new(),
    },
    CallQuery {
        language: "tsx",
        call_kind: "call_expression",
        function_kinds: JS_FUNCTION_KINDS,
        text: include_str!("../../../examples/calls/tsx.scm"),
        compiled: OnceLock::new(),
    },
    CallQuery {
        language: "go",
        call_kind: "call_expression",
        function_kinds: &["function_declaration", "method_declaration", "func_literal"],
        text: include_str!("../../../examples/calls/go.scm"),
        compiled: OnceLock::new(),
    },
    CallQuery {
        language: "java",
        call_kind: "method_invocation",
        function_kinds: &[
            "method_declaration",
            "constructor_declaration",
            "compact_constructor_declaration",
            "lambda_expression",
        ],
        text: include_str!("../../../examples/calls/java.scm"),
        compiled: OnceLock::new(),
    },
    CallQuery {
        language: "c",
        call_kind: "call_expression",
        function_kinds: &["function_definition"],
        text: include_str!("../../../examples/calls/c.scm"),
        compiled: OnceLock::new(),
    },
];

/// An embedded call query the shipped grammar refused.
#[derive(Debug, thiserror::Error)]
#[error("the embedded {language} call query does not compile against the shipped grammar")]
pub(super) struct CallQueryError {
    language: &'static str,
    #[source]
    source: tree_sitter::QueryError,
}

impl CallQuery {
    /// This language's compiled query, compiling it on first use; a failure
    /// is logged here, once per process, and kept.
    pub(super) fn compiled(
        &'static self,
        grammar: &Language,
    ) -> Result<&'static Query, &'static CallQueryError> {
        self.compiled
            .get_or_init(|| {
                Query::new(grammar, self.text).map_err(|source| {
                    let error = CallQueryError {
                        language: self.language,
                        source,
                    };
                    tracing::error!(%error, detail = %error.source, "call evidence disabled for this language");
                    error
                })
            })
            .as_ref()
    }
}

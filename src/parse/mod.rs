//! Tree-sitter language-pack boundary.

pub(crate) mod budget;
pub(crate) mod fetch_gate;
mod language;
pub(crate) mod local_load_failures;

pub(crate) use language::PACK_ALIASES;
pub use language::{
    PACK_VERSION, PROSE_LANGUAGE, TESTED_LANGUAGES, detect_language, is_prose_language_name,
    is_tested_language, language_available,
};

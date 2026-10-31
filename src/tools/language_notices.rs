//! The `notices` every `report`, `structure` and `leads` response carries:
//! one line per code language the answer covers whose grammar is untested.
//! One function builds the text so the three tools never word it differently.

use crate::parse::{PROSE_LANGUAGE, is_tested_language};

/// One notice per untested code language in `languages`, in input order.
/// Each item is a stored language name and, when the caller knows it, how
/// many files of that language the answer covers. Prose is not a code
/// language and tested languages need no notice, so both are skipped.
pub(super) fn untested_language_notices<'a>(
    languages: impl IntoIterator<Item = (&'a str, Option<i64>)>,
) -> Vec<String> {
    languages
        .into_iter()
        .filter(|(language, _)| *language != PROSE_LANGUAGE && !is_tested_language(language))
        .map(|(language, files)| {
            let subject = match files {
                Some(1) => "1 file uses".to_owned(),
                Some(files) => format!("{files} files use"),
                None => "uses".to_owned(),
            };
            format!(
                "{language}: {subject} an untested grammar (downloaded on first use). \
                 Only Rust is tested; treat {language} results as best-effort."
            )
        })
        .collect()
}

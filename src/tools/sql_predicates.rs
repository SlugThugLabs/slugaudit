//! Shared SQL `WHERE` predicates and GLOB compilation across `report`, `leads`,
//! and `structure` so all three tools evaluate file-set rules identically.

/// Single definition of "this path looks like a credential file", shared by
/// `report`'s count and `leads`' `credential_files` rows.
pub(crate) const CREDENTIAL_PATH_PREDICATE: &str = "lower(path) LIKE '%.env' \
    OR lower(path) LIKE '%.env.%' \
    OR lower(path) LIKE '%.pem' \
    OR lower(path) LIKE '%.key' \
    OR lower(path) LIKE '%.p12' \
    OR lower(path) LIKE '%.pfx' \
    OR lower(path) LIKE '%.p8' \
    OR lower(path) LIKE '%.jks' \
    OR lower(path) LIKE '%.keystore' \
    OR lower(path) LIKE '%.tfstate' \
    OR lower(path) LIKE '%.netrc' \
    OR lower(path) LIKE '%.npmrc' \
    OR lower(path) LIKE '%.git-credentials' \
    OR lower(path) LIKE 'service-account%.json' \
    OR lower(path) LIKE '%/service-account%.json' \
    OR ((lower(path) LIKE '%id\\_rsa%' ESCAPE '\\' \
        OR lower(path) LIKE '%id\\_dsa%' ESCAPE '\\' \
        OR lower(path) LIKE '%id\\_ecdsa%' ESCAPE '\\' \
        OR lower(path) LIKE '%id\\_ed25519%' ESCAPE '\\') \
        AND lower(path) NOT LIKE '%.pub')";

/// The `files` predicate for a parser-failure row, shared by `report` and
/// `leads` (`files_not_parsed`): an indexed non-prose file whose parser was
/// unavailable or whose parse failed.
pub(crate) const PARSER_FAILURE_PREDICATE: &str = "file_kind = 'indexed' \
    AND language IS NOT 'prose' \
    AND (parser_availability != 'Available' OR parse_outcome = 'Failed')";

/// Files whose grammar ran to a syntax tree, so evidence walkers read them.
pub(in crate::tools) const PARSED_FILES: &str = "file_kind = 'indexed' AND language IS NOT NULL \
    AND parse_outcome IN ('Succeeded', 'SyntaxErrors')";

/// Compiles a caller's `pattern` into a SQLite `GLOB` pattern (always bound as
/// a parameter, never interpolated into SQL). A pattern without `*` or `?` is
/// treated as a substring match with `[` escaped; `**` globstars collapse to `*`.
pub(in crate::tools) fn pattern_to_glob(pattern: &str) -> String {
    if pattern.contains(['*', '?']) {
        let collapsed = pattern.replace("/**/", "/");
        let collapsed = match collapsed.strip_prefix("**/") {
            Some(rest) => rest,
            None => &collapsed,
        };
        collapsed.replace("**", "*")
    } else {
        let escaped = pattern.replace('[', "[[]");
        format!("*{escaped}*")
    }
}

#[cfg(test)]
#[path = "sql_predicates_tests.rs"]
mod tests;

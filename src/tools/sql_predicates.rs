//! The SQL `WHERE` fragments more than one tool must evaluate identically:
//! `report` counts with them and `leads` reads rows with them, so each is
//! defined once here and the two surfaces cannot disagree.

/// Single definition of "this path looks like a credential file". Shared by
/// `report`'s count and `leads`' `credential_files` rows so the two surfaces
/// can never disagree about what they are counting.
///
/// Both families of rule match against `lower(path)`, so case never decides:
///
/// * **Ending rules** — `.env`, `.env.*`, `.pem`, `.key`, `.p12`, `.pfx`,
///   `.p8`, `.jks`, `.keystore`, `.tfstate`, `.netrc`, `.npmrc`,
///   `.git-credentials`. `LIKE` anchors nothing, so a leading `%` lets these
///   match at the repository root as well as below it, and the suffix is the
///   whole test: `notes.key.md` and `keyboard.rs` do **not** match `%.key`,
///   only a path ending in `.key` does.
/// * **Name rules** — `service-account*.json` at the root or under a
///   directory, and the SSH private keys `id_rsa`, `id_dsa`, `id_ecdsa` and
///   `id_ed25519` anywhere in a path *except* a `.pub` public key:
///   `id_rsa.pub` is not a secret and must not be counted.
///
/// `_` is a single-character wildcard in `LIKE`, so the SSH name patterns
/// escape it and carry `ESCAPE '\'` — without that, `sub/id-rsa` would match
/// `%/id_rsa%`. The fragment assumes an unaliased `path` column (the `files`
/// table in `report`, `files` for a `Family::Files` mechanic in `leads`),
/// and `Mechanic::where_clause` wraps it in parentheses before a path glob
/// is AND-ed onto it.
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

/// The `files` predicate for a parser-failure row, shared by every `report`
/// count and list so they cannot disagree: an indexed file whose parser was
/// unavailable or whose parse failed. Prose (`language = 'prose'`, see
/// `parse::PROSE_LANGUAGE`) is excluded: it is deliberately never
/// parsed, so it is not a failure. A file with no detected language
/// (`language IS NULL`) still counts. The literal is checked against the
/// constant by a unit test.
pub(crate) const PARSER_FAILURE_PREDICATE: &str = "file_kind = 'indexed' \
    AND language IS NOT 'prose' \
    AND (parser_availability != 'Available' OR parse_outcome = 'Failed')";

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::{Check, TestResult};

    /// Runs the shared fragment — the literal SQL text, not a Rust copy of
    /// it — against one path with the real SQLite engine both surfaces use,
    /// binding the path as a parameter.
    fn credential_path_predicate_matches(path: &str) -> TestResult<bool> {
        let connection = rusqlite::Connection::open_in_memory().check("in-memory database")?;
        let matched: i64 = connection
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM (SELECT ?1 AS path) WHERE {CREDENTIAL_PATH_PREDICATE})"
                ),
                rusqlite::params![path],
                |row| row.get(0),
            )
            .check("evaluate credential predicate")?;
        Ok(matched != 0)
    }

    /// Asserts each `(path, expected)` verdict through the real SQLite engine,
    /// so the test cannot pass while `report` and `leads` behave differently.
    fn assert_predicate_cases(cases: &[(&str, bool)]) -> TestResult {
        for &(path, expected) in cases {
            assert_eq!(
                credential_path_predicate_matches(path)?,
                expected,
                "predicate verdict for {path}"
            );
        }
        Ok(())
    }

    #[test]
    fn credential_path_predicate_matches_credential_shaped_paths() -> TestResult {
        assert_predicate_cases(&[
            // The baseline rules.
            (".env", true),
            (".env.local", true),
            ("config/.env", true),
            (".env.production", true),
            (".ENV", true),
            ("server.pem", true),
            ("keys/cred.pem", true),
            ("keys/tls.key", true),
            ("main.key", true),
            ("cert.p12", true),
            ("cert.pfx", true),
            // Dotfiles, key stores and state files.
            (".netrc", true),
            ("home/.netrc", true),
            (".npmrc", true),
            ("proj/.npmrc", true),
            (".git-credentials", true),
            ("home/.git-credentials", true),
            ("AuthKey_ABC123.p8", true),
            ("debug.jks", true),
            ("release.keystore", true),
            ("terraform.tfstate", true),
            ("prod.terraform.tfstate", true),
            ("service-account.json", true),
            ("keys/service-account.json", true),
            ("service-account-prod.json", true),
        ])?;
        Ok(())
    }

    #[test]
    fn credential_path_predicate_matches_ssh_private_keys() -> TestResult {
        assert_predicate_cases(&[
            // Root-level SSH keys match as well as nested ones.
            ("id_rsa", true),
            ("id_ed25519", true),
            ("id_ecdsa", true),
            ("id_dsa", true),
            (".ssh/id_rsa", true),
            ("home/.ssh/id_ecdsa", true),
            ("backup/id_rsa_bak", true),
            ("ID_RSA", true),
        ])?;
        Ok(())
    }

    #[test]
    fn credential_path_predicate_rejects_public_keys_and_lookalikes() -> TestResult {
        assert_predicate_cases(&[
            // A public key is not a secret.
            ("id_rsa.pub", false),
            ("id_ed25519.pub", false),
            (".ssh/id_rsa.pub", false),
            (".ssh/id_ecdsa.pub", false),
            // `_` is a single-character `LIKE` wildcard, so the escaped
            // SSH patterns must not match a hyphen or any other character
            // in its place.
            ("sub/id-rsa", false),
            ("sub/idXrsa", false),
            // Ordinary source that shares letters with a rule.
            ("keyboard.rs", false),
            ("notes.key.md", false),
            ("pem_util.rs", false),
            ("keystore.rs", false),
            ("src/lib.rs", false),
            ("README.md", false),
            ("environment.rs", false),
            (".envrc", false),
            // Public SSH material.
            ("authorized_keys", false),
            ("known_hosts", false),
        ])?;
        Ok(())
    }
}

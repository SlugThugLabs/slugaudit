//! Which `#[cfg(...)]` attributes gate an item to test builds only.

/// Whether `line` starts a `#[cfg(...)]` attribute whose item exists only
/// when `test` is set: `test` alone, or `test` as a conjunct of `all(...)`.
/// `not(test)` and `any(..., test)` items are production code in a non-test
/// build, so they stay. A string literal such as `feature = "test"` is not the
/// `test` predicate.
pub(super) fn is_test_cfg(line: &str) -> bool {
    let Some(predicate) = line.trim_start().strip_prefix("#[cfg(") else {
        return false;
    };
    // One entry per open parenthesis: whether `test` is required to satisfy
    // the predicate at that depth. The attribute's own `cfg(` is the first.
    let mut required: Vec<bool> = vec![true];
    let mut word = String::new();
    let mut in_string = false;
    for character in predicate.chars() {
        if in_string {
            in_string = character != '"';
            continue;
        }
        if character.is_ascii_alphanumeric() || character == '_' {
            word.push(character);
            continue;
        }
        let here = required.last().is_some_and(|test_required| *test_required);
        match character {
            '(' => required.push(here && word == "all"),
            _ if word == "test" && here => return true,
            _ => {}
        }
        word.clear();
        match character {
            '"' => in_string = true,
            ')' => {
                required.pop();
                if required.is_empty() {
                    return false;
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::is_test_cfg;

    #[test]
    fn test_alone_and_test_inside_all_gate_to_test_builds() {
        for line in [
            "#[cfg(test)]",
            "  #[cfg(test)] mod tests;",
            "#[cfg(all(test, unix))]",
            "#[cfg(all(unix, test))]",
            "#[cfg(all(unix, all(test)))]",
        ] {
            assert!(is_test_cfg(line), "{line}");
        }
    }

    #[test]
    fn items_that_exist_in_a_non_test_build_are_not_test_only() {
        for line in [
            "#[cfg(not(test))]",
            "#[cfg(any(target_os = \"macos\", test))]",
            "#[cfg(all(unix, not(test)))]",
            "#[cfg(feature = \"test\")]",
            "#[cfg(unix)]",
            "#[cfg_attr(test, derive(Debug))]",
            "#[test]",
            "fn test() {}",
        ] {
            assert!(!is_test_cfg(line), "{line}");
        }
    }
}

//! Minimal ANSI styling for the human-facing CLI surface.

use std::io::IsTerminal;

/// Minimal ANSI styling for the human-facing CLI surface (`menu`, `help`).
///
/// Detects once, at construction, whether stdout is a real terminal. When
/// it is not (piped to a file, captured by a test harness, or fed into a
/// CI log), every style method returns its input unchanged, so no escape
/// sequences ever leak into redirected output or downstream tools. When
/// stdout IS a terminal, the methods wrap the text in the named ANSI
/// SGR code and reset afterwards.
///
/// `help`, `menu`, and `connect` are the only paths that build with these
/// — the stdio MCP server (`serve`) must never touch stdout for humans,
/// and the server's diagnostics on stderr already disable ANSI
/// unconditionally in `main`. This helper is deliberately the single
/// place ANSI codes are produced.
pub(crate) struct Style {
    enabled: bool,
}

impl Style {
    /// A `Style` that colors only if stdout is a terminal at call time.
    pub(crate) fn stdout() -> Self {
        Self::detect(std::io::stdout().is_terminal(), |name| {
            std::env::var_os(name)
        })
    }

    /// Colour only on a terminal, and never when `NO_COLOR` is set non-empty
    /// (no-color.org) or `TERM` is `dumb`. The environment lookup is injected
    /// so tests never mutate the process environment.
    fn detect(is_terminal: bool, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Self {
        let no_color = var("NO_COLOR").is_some_and(|value| !value.is_empty());
        let dumb = var("TERM").is_some_and(|value| value == "dumb");
        Self {
            enabled: is_terminal && !no_color && !dumb,
        }
    }

    /// A `Style` that never colors. Test-only: enables assertions that a
    /// rendering produces the exact plain text regardless of the
    /// destination terminal.
    #[cfg(test)]
    pub(crate) const fn plain() -> Self {
        Self { enabled: false }
    }

    #[cfg(test)]
    pub(crate) const fn colored() -> Self {
        Self { enabled: true }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if !self.enabled {
            return text.to_owned();
        }
        format!("\x1b[{code}m{text}\x1b[0m")
    }

    pub(crate) fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }

    pub(crate) fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }

    pub(crate) fn cyan(&self, text: &str) -> String {
        self.wrap("36", text)
    }

    pub(crate) fn green(&self, text: &str) -> String {
        self.wrap("32", text)
    }

    pub(crate) fn yellow(&self, text: &str) -> String {
        self.wrap("33", text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn style_colours_a_terminal_with_a_clean_environment() {
        assert!(Style::detect(true, env(&[("TERM", "xterm-256color")])).enabled());
    }

    #[test]
    fn style_is_disabled_off_a_terminal() {
        assert!(!Style::detect(false, env(&[])).enabled());
    }

    #[test]
    fn style_is_disabled_by_a_non_empty_no_color() {
        assert!(!Style::detect(true, env(&[("NO_COLOR", "1")])).enabled());
    }

    #[test]
    fn style_ignores_an_empty_no_color() {
        assert!(Style::detect(true, env(&[("NO_COLOR", "")])).enabled());
    }

    #[test]
    fn style_is_disabled_by_term_dumb() {
        assert!(!Style::detect(true, env(&[("TERM", "dumb")])).enabled());
    }
}

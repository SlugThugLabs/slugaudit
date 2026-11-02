//! Dev-only test helper: turn a failed step into a [`TestResult`] error that
//! names the caller's `file:line`, the step, and the cause, instead of a panic.
//!
//! A test returns [`TestResult`] and writes `thing().check("what this step
//! does")?`. When the step fails, the harness prints the error's `Debug` form,
//! which is exactly `<file>:<line>: <what>: <cause>`.
//!
//! The crate is a development dependency of SlugAudit's unit tests,
//! integration tests and benches; it is never part of the shipped binary.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::fmt;
use std::panic::Location;

/// The result type of a test that can fail at a step.
pub type TestResult<T = ()> = Result<T, TestError>;

/// A failed test step: where it happened, what the step was, and why it failed.
pub struct TestError {
    /// Source location of the `check` or `check_err` call.
    location: &'static Location<'static>,
    /// What the step was doing.
    what: String,
    /// The underlying failure.
    cause: String,
}

impl TestError {
    /// Builds an error for the step `what` at `location`.
    fn new(location: &'static Location<'static>, what: &str, cause: String) -> Self {
        Self {
            location,
            what: what.to_owned(),
            cause,
        }
    }
}

impl fmt::Debug for TestError {
    /// Writes `<file>:<line>: <what>: <cause>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}: {}",
            self.location.file(),
            self.location.line(),
            self.what,
            self.cause
        )
    }
}

impl fmt::Display for TestError {
    /// Writes the same text as `Debug`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for TestError {}

/// Converts a `Result` or `Option` into a [`TestResult`], recording the
/// caller's location.
pub trait Check<T> {
    /// Returns the value, or a [`TestError`] naming the step `what`, the
    /// caller's `file:line`, and the cause.
    ///
    /// # Errors
    ///
    /// Returns the [`TestError`] when the receiver is `Err` or `None`.
    fn check(self, what: &str) -> TestResult<T>;
}

impl<T, E: fmt::Display> Check<T> for Result<T, E> {
    #[track_caller]
    fn check(self, what: &str) -> TestResult<T> {
        let location = Location::caller();
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(TestError::new(location, what, error.to_string())),
        }
    }
}

impl<T> Check<T> for Option<T> {
    #[track_caller]
    fn check(self, what: &str) -> TestResult<T> {
        let location = Location::caller();
        match self {
            Some(value) => Ok(value),
            None => Err(TestError::new(
                location,
                what,
                "value was absent".to_owned(),
            )),
        }
    }
}

/// [`Check`] for a real [`std::error::Error`]: the cause carries the whole
/// `source()` chain, not only the top error's `Display`.
///
/// `check` accepts any `Display` error (a `String` included), which cannot
/// reach `source()`. A step whose error type implements `Error` uses this to
/// see the root cause.
pub trait CheckSources<T> {
    /// Returns the value, or a [`TestError`] whose cause is the error's text
    /// followed by `: caused by: <source>` for each source in the chain that
    /// the text does not already contain.
    ///
    /// # Errors
    ///
    /// Returns the [`TestError`] when the receiver is `Err`.
    fn check_sources(self, what: &str) -> TestResult<T>;
}

impl<T, E: std::error::Error> CheckSources<T> for Result<T, E> {
    #[track_caller]
    fn check_sources(self, what: &str) -> TestResult<T> {
        let location = Location::caller();
        match self {
            Ok(value) => Ok(value),
            Err(error) => Err(TestError::new(location, what, cause_with_sources(&error))),
        }
    }
}

/// The error's `Display` text plus each distinct `source()` text below it.
fn cause_with_sources(error: &dyn std::error::Error) -> String {
    let mut cause = error.to_string();
    let mut next = error.source();
    while let Some(source) = next {
        let text = source.to_string();
        if !cause.contains(&text) {
            cause.push_str(": caused by: ");
            cause.push_str(&text);
        }
        next = source.source();
    }
    cause
}

/// Converts an expected failure into a value: the replacement for
/// `unwrap_err` and `expect_err`.
pub trait CheckErr<T, E> {
    /// Returns the error value, or a [`TestError`] when the result was `Ok`.
    ///
    /// # Errors
    ///
    /// Returns the [`TestError`] when the receiver is `Ok`.
    fn check_err(self, what: &str) -> TestResult<E>;
}

impl<T: fmt::Debug, E> CheckErr<T, E> for Result<T, E> {
    #[track_caller]
    fn check_err(self, what: &str) -> TestResult<E> {
        let location = Location::caller();
        match self {
            Ok(value) => Err(TestError::new(
                location,
                what,
                format!("expected an error, got Ok({value:?})"),
            )),
            Err(error) => Ok(error),
        }
    }
}

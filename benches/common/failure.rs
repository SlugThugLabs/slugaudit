//! Failure plumbing shared by every bench binary. Criterion's `iter` closures
//! cannot return a `Result`, so a failing step inside one is recorded here
//! (first failure wins) and reported once, at the bench entry point, with the
//! step's `file:line`.

use slugaudit_test_check::{TestError, TestResult};
use std::cell::RefCell;
use std::process::ExitCode;

/// The first failure seen inside timed iterations.
pub(crate) struct FirstFailure(RefCell<Option<TestError>>);

impl FirstFailure {
    /// An empty record: nothing has failed yet.
    pub(crate) fn new() -> Self {
        Self(RefCell::new(None))
    }

    /// Keeps `result`'s error if it is the first one; passes success through
    /// as `()` so an iteration closure can end with it.
    pub(crate) fn record<T>(&self, result: TestResult<T>) {
        if let Err(error) = result {
            self.0.borrow_mut().get_or_insert(error);
        }
    }

    /// Returns the first recorded failure, if any.
    ///
    /// # Errors
    ///
    /// Returns the first error passed to [`FirstFailure::record`].
    pub(crate) fn finish(&self) -> TestResult {
        match self.0.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// Bench entry point tail: prints a failure once to stderr as
/// `Error: <file>:<line>: <what>: <cause>` and maps it to a nonzero exit code.
pub(crate) fn exit_code(result: TestResult) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

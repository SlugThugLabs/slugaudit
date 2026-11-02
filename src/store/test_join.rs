//! Test helper: join a thread and turn its panic into a [`TestResult`] error
//! (a panic payload is not `Display`, so `Check` cannot take it directly).
//! `#[cfg(test)]`-only; shared by the store and sync tests.

use slugaudit_test_check::{Check, TestResult};
use std::any::Any;

/// The text of a panic payload: the message for `panic!("..")` and
/// `panic!("{..}")`, a fixed description for any other payload type.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "the thread panicked with a non-text payload".to_owned()
}

/// Joins `handle`; a panicked thread becomes a [`TestResult`] error naming
/// `what`, the caller's `file:line` and the panic message.
#[track_caller]
pub(crate) fn join_checked<T>(handle: std::thread::JoinHandle<T>, what: &str) -> TestResult<T> {
    handle
        .join()
        .map_err(|payload| panic_message(payload.as_ref()))
        .check(what)
}

/// Same as [`join_checked`] for a scoped thread.
#[track_caller]
pub(crate) fn join_scoped_checked<T>(
    handle: std::thread::ScopedJoinHandle<'_, T>,
    what: &str,
) -> TestResult<T> {
    handle
        .join()
        .map_err(|payload| panic_message(payload.as_ref()))
        .check(what)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slugaudit_test_check::CheckErr;

    #[test]
    fn a_finished_thread_returns_its_value() -> TestResult {
        let handle = std::thread::spawn(|| 7);

        assert_eq!(join_checked(handle, "join")?, 7);
        Ok(())
    }

    #[test]
    fn a_text_panic_is_reported_with_its_message() -> TestResult {
        let handle = std::thread::spawn(|| {
            std::panic::resume_unwind(Box::new("boom".to_owned()));
        });

        let error = join_checked(handle, "join").check_err("a panicked thread is an error")?;

        assert!(error.to_string().ends_with("join: boom"), "{error}");
        Ok(())
    }

    #[test]
    fn a_non_text_panic_gets_a_fixed_description() {
        assert_eq!(
            panic_message(&42_u8),
            "the thread panicked with a non-text payload"
        );
        assert_eq!(panic_message(&"static text"), "static text");
    }
}

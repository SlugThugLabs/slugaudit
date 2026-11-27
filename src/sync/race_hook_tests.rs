use super::*;
use slugaudit_test_check::{Check, CheckErr, TestResult, ensure, ensure_eq};

#[cfg(unix)]
#[test]
fn a_hook_armed_through_a_symlinked_root_fires_for_either_spelling() -> TestResult {
    let real = tempfile::tempdir().check("real root")?;
    let links = tempfile::tempdir().check("link dir")?;
    let link = links.path().join("root");
    std::os::unix::fs::symlink(real.path(), &link).check("symlink root")?;
    let fired = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let first = std::sync::Arc::clone(&fired);
    let first_handle = set(&link, RacePoint::BeforeCommit, move || {
        first.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })?;
    fire(real.path(), RacePoint::BeforeCommit);
    let second = std::sync::Arc::clone(&fired);
    let second_handle = set(real.path(), RacePoint::BeforeCommit, move || {
        second.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })?;
    fire(&link, RacePoint::BeforeCommit);

    ensure_eq(
        &fired.load(std::sync::atomic::Ordering::SeqCst),
        &2,
        "set and fire must agree on the root whichever spelling each side uses",
    )?;
    first_handle.outcome()?;
    second_handle.outcome()
}

#[test]
fn a_hook_fires_only_at_its_own_point() -> TestResult {
    let root = tempfile::tempdir().check("root")?;
    let handle = set(root.path(), RacePoint::AfterDiscovery, || Ok(()))?;

    fire(root.path(), RacePoint::BeforeCommit);
    ensure(
        handle.outcome().is_err(),
        "a hook must not fire at another point",
    )?;
    fire(root.path(), RacePoint::AfterDiscovery);
    handle.outcome()
}

#[test]
fn a_failed_hook_reaches_its_handle() -> TestResult {
    let root = tempfile::tempdir().check("root")?;
    let handle = set(root.path(), RacePoint::BeforeCommit, || {
        Err::<(), _>("hook step failed").check("the probe step")
    })?;

    fire(root.path(), RacePoint::BeforeCommit);

    let error = handle
        .outcome()
        .check_err("a failed hook must fail its handle")?;
    ensure(
        error.to_string().contains("hook step failed"),
        format!("the handle must carry the hook's own error: {error}"),
    )?;
    Ok(())
}

#[test]
fn a_hook_that_never_fired_fails_its_handle() -> TestResult {
    let root = tempfile::tempdir().check("root")?;
    let handle = set(root.path(), RacePoint::BeforeCommit, || Ok(()))?;

    let error = handle
        .outcome()
        .check_err("an unfired hook must fail its handle")?;
    ensure(
        error.to_string().contains("the race hook fired"),
        format!("got: {error}"),
    )?;
    Ok(())
}

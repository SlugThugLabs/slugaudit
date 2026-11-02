use super::harness_tests::*;
use super::*;
use slugaudit_test_check::{Check, TestResult};
use uuid::Uuid;

#[test]
fn result_store_a_cursor_round_trips_in_the_exact_documented_form() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 10, 3).check("write")?;

    let cursor = fx.store.cursor(handle.id, 7);
    let token = cursor.to_string();
    let decoded = fx.store.decode_cursor(&token).check("decode")?;

    let prefix = "v1.";
    assert!(
        token.starts_with(prefix) && token.ends_with(&format!(".{}.7", handle.id)),
        "{token}"
    );
    assert_eq!(token.split('.').count(), 4);
    assert_eq!(decoded, cursor);
    Ok(())
}

#[test]
fn result_store_a_cursor_reads_the_page_it_names() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 10, 3).check("write")?;
    let token = fx.store.cursor(handle.id, 4).to_string();

    let page = read_token(&fx.store, &token).check("read")?;

    assert_eq!(page.rows.first().map(|row| row.ord), Some(4));
    assert_eq!(page.rows.len(), 6);
    Ok(())
}

#[test]
fn result_store_every_malformed_cursor_is_unknown() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 10, 3).check("write")?;
    let good = fx.store.cursor(handle.id, 1).to_string();
    let parts: Vec<&str> = good.split('.').collect();
    let (nonce, result) = (parts[1], parts[2]);
    let cases = [
        String::new(),
        "v2.".to_owned() + &good[3..],
        format!("v1.{nonce}.{result}"),
        format!("{good}.0"),
        format!("v1.{nonce}.{result}.01"),
        format!("v1.{nonce}.{result}.+1"),
        format!("v1.{nonce}.{result}.-1"),
        format!("v1.{nonce}.{result}."),
        format!("v1.{nonce}.not-a-uuid.1"),
        format!("v1.{nonce}.{}.1", result.to_uppercase()),
        format!("v1.{nonce}.{result}.99999999999999999999999"),
        format!("v1.{nonce}.{result}.{}", "9".repeat(200)),
        format!("v1.{nonce}.{result}.\u{661}"),
        format!("v1.{nonce}.{result}.1 "),
    ];

    for token in &cases {
        let outcome = fx.store.decode_cursor(token);

        assert!(
            matches!(outcome, Err(ResultStoreError::Unknown)),
            "{token:?}: {outcome:?}"
        );
    }
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .unknown_cursors,
        cases.len() as u64
    );
    Ok(())
}

#[test]
fn result_store_a_cursor_from_another_process_is_unknown() -> TestResult {
    let (one, two) = (fixture(1 << 30)?, fixture(1 << 30)?);
    let handle = write_rows(&one.store, 10, 3).check("write")?;
    let foreign = one.store.cursor(handle.id, 0).to_string();

    let outcome = read_token(&two.store, &foreign);

    assert!(
        matches!(outcome, Err(ResultStoreError::Unknown)),
        "{outcome:?}"
    );
    Ok(())
}

#[test]
fn result_store_a_valid_cursor_for_a_reclaimed_result_is_expired() -> TestResult {
    let fx = fixture(1 << 30)?;
    let stranger = ResultId(Uuid::new_v4());
    let token = fx.store.cursor(stranger, 0).to_string();

    let outcome = read_token(&fx.store, &token);

    assert!(
        matches!(outcome, Err(ResultStoreError::Expired)),
        "{outcome:?}"
    );
    assert_eq!(
        fx.store
            .stats()
            .check("read the store counters")?
            .expired_cursors,
        1
    );
    Ok(())
}

#[test]
fn result_store_a_cursor_past_the_row_count_is_unknown() -> TestResult {
    let fx = fixture(1 << 30)?;
    let handle = write_rows(&fx.store, 10, 3).check("write")?;
    let token = fx.store.cursor(handle.id, 11).to_string();

    let outcome = read_token(&fx.store, &token);

    assert!(
        matches!(outcome, Err(ResultStoreError::Unknown)),
        "{outcome:?}"
    );
    Ok(())
}

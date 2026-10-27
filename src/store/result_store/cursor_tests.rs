use super::harness_tests::*;
use super::*;
use uuid::Uuid;

#[test]
fn result_store_a_cursor_round_trips_in_the_exact_documented_form() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");

    let cursor = fx.store.cursor(handle.id, 7);
    let token = cursor.to_string();
    let decoded = fx.store.decode_cursor(&token).expect("decode");

    let prefix = "v1.";
    assert!(
        token.starts_with(prefix) && token.ends_with(&format!(".{}.7", handle.id)),
        "{token}"
    );
    assert_eq!(token.split('.').count(), 4);
    assert_eq!(decoded, cursor);
}

#[test]
fn result_store_a_cursor_reads_the_page_it_names() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");
    let token = fx.store.cursor(handle.id, 4).to_string();

    let page = read_token(&fx.store, &token).expect("read");

    assert_eq!(page.rows.first().map(|row| row.ord), Some(4));
    assert_eq!(page.rows.len(), 6);
}

#[test]
fn result_store_every_malformed_cursor_is_unknown() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");
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
    assert_eq!(fx.store.stats().unknown_cursors, cases.len() as u64);
}

#[test]
fn result_store_a_cursor_from_another_process_is_unknown() {
    let (one, two) = (fixture(1 << 30), fixture(1 << 30));
    let handle = write_rows(&one.store, 10, 3).expect("write");
    let foreign = one.store.cursor(handle.id, 0).to_string();

    let outcome = read_token(&two.store, &foreign);

    assert!(
        matches!(outcome, Err(ResultStoreError::Unknown)),
        "{outcome:?}"
    );
}

#[test]
fn result_store_a_valid_cursor_for_a_reclaimed_result_is_expired() {
    let fx = fixture(1 << 30);
    let stranger = ResultId(Uuid::new_v4());
    let token = fx.store.cursor(stranger, 0).to_string();

    let outcome = read_token(&fx.store, &token);

    assert!(
        matches!(outcome, Err(ResultStoreError::Expired)),
        "{outcome:?}"
    );
    assert_eq!(fx.store.stats().expired_cursors, 1);
}

#[test]
fn result_store_a_cursor_past_the_row_count_is_unknown() {
    let fx = fixture(1 << 30);
    let handle = write_rows(&fx.store, 10, 3).expect("write");
    let token = fx.store.cursor(handle.id, 11).to_string();

    let outcome = read_token(&fx.store, &token);

    assert!(
        matches!(outcome, Err(ResultStoreError::Unknown)),
        "{outcome:?}"
    );
}

use crate::util::now_unix;

#[test]
fn now_unix_tracks_real_wall_clock_time() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the epoch")
        .as_secs();
    let value = now_unix();
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the epoch")
        .as_secs();

    let value = u64::try_from(value).expect("a real current timestamp is never negative");
    assert!(
        (before..=after).contains(&value),
        "now_unix() must return the real current time, not a fixed constant: \
         expected within [{before}, {after}], got {value}"
    );
}

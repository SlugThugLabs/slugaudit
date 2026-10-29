use super::*;

fn key(revision: &str, id: &str) -> ScanKey {
    ScanKey {
        project: "p".to_owned(),
        revision: revision.to_owned(),
        glob: None,
        id: id.to_owned(),
        language: "python".to_owned(),
        query: "(call) @c".to_owned(),
        source: "session",
    }
}

fn rows(count: usize) -> Arc<Vec<CachedRow>> {
    Arc::new(
        (0..count)
            .map(|line| CachedRow {
                file: "a.py".to_owned(),
                line: i64::try_from(line).expect("small"),
                text: "x".to_owned(),
            })
            .collect(),
    )
}

#[test]
fn a_lookup_at_a_new_revision_misses_and_drops_the_old_revision() {
    let cache = MechanicScanCache::default();
    cache.insert(key("r1", "a"), rows(1));

    let miss = cache.get(&key("r2", "a"));
    let stats = cache.stats();

    assert!(miss.is_none(), "another revision is a miss");
    assert_eq!(stats.entries, 0, "the stale revision's entry was dropped");
    assert_eq!(stats.bytes, 0);
}

#[test]
fn a_changed_identity_under_the_same_id_misses() {
    let cache = MechanicScanCache::default();
    cache.insert(key("r1", "a"), rows(1));
    let mut other = key("r1", "a");
    other.query = "(call function: (identifier) @c)".to_owned();

    assert!(cache.get(&other).is_none(), "a different query misses");
    assert!(cache.get(&key("r1", "a")).is_some(), "the original hits");
}

#[test]
fn insert_evicts_oldest_first_and_counts_each_eviction() {
    let entry_bytes = key("r1", "a").estimated_bytes() + rows(1)[0].estimated_bytes();
    let cache = MechanicScanCache::with_budget(entry_bytes * 2);
    cache.insert(key("r1", "a"), rows(1));
    cache.insert(key("r1", "b"), rows(1));

    cache.insert(key("r1", "c"), rows(1));

    let stats = cache.stats();
    assert_eq!(stats.evictions, 1);
    assert_eq!(stats.entries, 2);
    assert!(cache.get(&key("r1", "a")).is_none(), "the oldest went");
    assert!(cache.get(&key("r1", "b")).is_some());
    assert!(cache.get(&key("r1", "c")).is_some());
}

#[test]
fn an_entry_larger_than_the_budget_is_skipped_and_counted() {
    let cache = MechanicScanCache::with_budget(100);

    cache.insert(key("r1", "a"), rows(1));

    let stats = cache.stats();
    assert_eq!(stats.skipped_too_large, 1);
    assert_eq!(stats.entries, 0);
    assert_eq!(stats.evictions, 0, "nothing was evicted for it");
}

#[test]
fn remove_id_drops_every_entry_of_that_id_only() {
    let cache = MechanicScanCache::default();
    cache.insert(key("r1", "a"), rows(1));
    cache.insert(key("r1", "b"), rows(1));

    cache.remove_id("a");

    assert!(cache.get(&key("r1", "a")).is_none());
    assert!(cache.get(&key("r1", "b")).is_some());
}

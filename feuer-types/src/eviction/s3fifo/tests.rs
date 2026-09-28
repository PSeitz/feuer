use super::*;

fn range(bytes: u64) -> ByteRange {
    ByteRange::new(0, bytes).unwrap()
}

fn insert(policy: &mut S3Fifo, id: u64, key: &str, bytes: u64) {
    policy.insert(id, key.to_owned(), range(bytes));
}

fn victim(policy: &mut S3Fifo, work: usize) -> Option<u64> {
    policy.candidate(work, |_, _| true).map(|(id, _, _)| id)
}

#[test]
fn small_filters_unaccessed_ranges_and_promotes_after_one_access() {
    let mut policy = S3Fifo::new(100);
    insert(&mut policy, 1, "hot", 20);
    insert(&mut policy, 2, "cold", 20);
    policy.record_access(1);
    assert_eq!(victim(&mut policy, 1), None); // Promotion is one bounded step.
    assert!(!policy.residents[&1].in_small);
    assert_eq!(policy.residents[&1].frequency, 1);
    assert_eq!(policy.small_bytes, 20);
    assert_eq!(victim(&mut policy, 1), Some(2));
    policy.evict(2);
    assert_eq!(victim(&mut policy, 1), None); // Preserved credit gives a second chance.
    assert_eq!(policy.residents[&1].frequency, 0);
    assert_eq!(victim(&mut policy, 1), Some(1));
    policy.evict(1);
    assert_eq!(policy.ghosts.len(), 1); // Main eviction creates no ghost.
    assert_eq!(victim(&mut policy, 10), None);
}

#[test]
fn small_eviction_finishes_after_promotion_drops_it_below_target() {
    let mut policy = S3Fifo::new(100);
    insert(&mut policy, 1, "hot", 10);
    insert(&mut policy, 2, "cold", 1);
    policy.record_access(1);
    policy.record_access(1);
    assert_eq!(victim(&mut policy, 1), None);
    assert_eq!(policy.small_bytes, 1);
    assert_eq!(victim(&mut policy, 1), Some(2));
}

#[test]
fn main_rotates_and_decrements_saturating_frequency_without_hit_reordering() {
    let mut policy = S3Fifo::new(100);
    insert(&mut policy, 1, "a", 20);
    insert(&mut policy, 2, "b", 20);
    policy.evict(1);
    policy.evict(2);
    insert(&mut policy, 3, "a", 20);
    insert(&mut policy, 4, "b", 20);
    for _ in 0..10 {
        policy.record_access(3);
    }
    assert_eq!(policy.residents[&3].frequency, 3);
    assert_eq!(victim(&mut policy, 1), None);
    assert_eq!(policy.residents[&3].frequency, 2);
    assert_eq!(victim(&mut policy, 1), Some(4));
    policy.evict(4);
    assert_eq!(victim(&mut policy, 2), None);
    assert_eq!(victim(&mut policy, 1), Some(3));
}

#[test]
fn ghost_history_is_byte_bounded_and_matches_full_key_and_exact_range() {
    let mut policy = S3Fifo::new(100);
    for id in 1..=3 {
        insert(&mut policy, id, &id.to_string(), 40);
        policy.evict(id);
    }
    assert_eq!(policy.ghost_bytes, 80);
    assert_eq!(policy.ghosts.len(), 2);
    insert(&mut policy, 4, "1", 40);
    assert!(policy.residents[&4].in_small); // Oldest ghost aged out.
    insert(&mut policy, 5, "2", 39);
    assert!(policy.residents[&5].in_small); // Same key, different range.
    insert(&mut policy, 6, "3", 40);
    assert!(!policy.residents[&6].in_small);
    assert_eq!(policy.ghost_bytes, 40);
    assert_eq!(policy.residents[&6].frequency, 0);
    policy.remove(6);
    insert(&mut policy, 7, "3", 40);
    assert!(policy.residents[&7].in_small); // Removal did not create a ghost.
}

#[test]
fn tiny_capacities_and_oversized_entries_can_be_evicted() {
    for capacity in [0, 1, 9] {
        let mut policy = S3Fifo::new(capacity);
        insert(&mut policy, 1, "large", 100);
        policy.record_access(1);
        policy.record_access(1);
        assert_eq!(victim(&mut policy, 3), None); // Promotion and two second chances.
        assert_eq!(victim(&mut policy, 1), Some(1));
        policy.evict(1);
        assert!(policy.residents.is_empty());
        assert!(policy.ghosts.is_empty());
        insert(&mut policy, 2, "cold-large", 100);
        policy.evict(2);
        assert!(policy.ghosts.is_empty());
    }
}

#[test]
fn ineligible_heads_do_not_starve_either_queue_with_one_step_budgets() {
    for small_bytes in [5, 20] {
        let mut policy = S3Fifo::new(100);
        insert(&mut policy, 1, "main", 20);
        policy.evict(1);
        insert(&mut policy, 2, "main", 20);
        insert(&mut policy, 3, "small", small_bytes);
        let eligible = if small_bytes > 10 { "main" } else { "small" };
        assert!(policy.candidate(1, |key, _| key == eligible).is_none());
        let (_, key, _) = policy.candidate(1, |key, _| key == eligible).unwrap();
        assert_eq!(key, eligible);
    }
}

#[test]
fn replacement_and_removal_leave_no_stale_queue_nodes() {
    let mut policy = S3Fifo::new(100);
    for id in 1..=1000 {
        insert(&mut policy, id, "replaced", 4);
        policy.remove(id);
        policy.record_access(id); // A completed read of a removed ID is ignored.
        assert!(policy.residents.is_empty());
        assert!(policy.small.is_empty());
        assert!(policy.main.is_empty());
        assert!(policy.ghosts.is_empty());
        assert_eq!(policy.small_bytes, 0);
    }
}

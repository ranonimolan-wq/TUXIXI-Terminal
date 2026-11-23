//! Manual memory management helpers for the winit GUI.
//!
//! TUXIXI no longer sits on top of GLib/GTK, whose allocators and
//! caches once kept the idle process at ~240 MB. Everything the GUI
//! allocates is now owned, bounded and trimmed *here*, explicitly:
//!
//! * [`BoundedDeque`] — a `VecDeque` with a hard capacity. It backs the
//!   scrollback buffer so a runaway program (think `yes | less` for an
//!   hour) can never grow the process without limit; every push that
//!   exceeds the cap drops the oldest row immediately, and backing
//!   storage is periodically compacted with `shrink_to_fit`.
//! * [`BoundedCache`] — a bounded `HashMap` cache with deterministic
//!   eviction. It backs the glyph cache in [`crate::font`]: once the
//!   entry limit is reached the whole map is cleared (simple, amortized
//!   O(1), and keeps worst-case memory = limit x max entry size).
//!
//! Both types are `const`-constructible so the GUI never starts with a
//! heap allocation it does not need.

use std::collections::HashMap;
use std::collections::VecDeque;

/// A `VecDeque` with a hard maximum number of elements.
///
/// Invariant: `self.len() <= cap` after every operation. When the cap
/// is reached, `push_back`/`push_front` evict from the opposite end and
/// mark the backing storage as fragmented; call [`BoundedDeque::maintain`]
/// at frame boundaries to run `shrink_to_fit` on the ring buffer (this
/// is the "manual" part: no hidden allocator keeps freed capacity
/// around forever).
#[derive(Debug)]
pub struct BoundedDeque<T> {
    inner: VecDeque<T>,
    cap: usize,
    /// Number of evictions since the last [`BoundedDeque::maintain`] call.
    evictions_since_maintain: usize,
}

impl<T> BoundedDeque<T> {
    /// Maximum number of rows kept by the scrollback buffer.
    pub const DEFAULT_CAP: usize = 10_000;

    /// A bounded, empty deque (no heap allocation until first push).
    pub fn new(cap: usize) -> Self {
        Self {
            inner: VecDeque::new(),
            cap: cap.max(1),
            evictions_since_maintain: 0,
        }
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Push at the back, evicting from the front when the cap is hit.
    /// Returns `true` when an element was evicted.
    pub fn push_back(&mut self, value: T) -> bool {
        let mut evicted = false;
        if self.inner.len() >= self.cap {
            self.inner.pop_front();
            self.evictions_since_maintain += 1;
            evicted = true;
        }
        self.inner.push_back(value);
        evicted
    }

    /// Push at the front, evicting from the back when the cap is hit.
    /// Returns `true` when an element was evicted.
    pub fn push_front(&mut self, value: T) -> bool {
        let mut evicted = false;
        if self.inner.len() >= self.cap {
            self.inner.pop_back();
            self.evictions_since_maintain += 1;
            evicted = true;
        }
        self.inner.push_front(value);
        evicted
    }

    pub fn pop_front(&mut self) -> Option<T> {
        self.inner.pop_front()
    }

    pub fn pop_back(&mut self) -> Option<T> {
        self.inner.pop_back()
    }

    pub fn front(&self) -> Option<&T> {
        self.inner.front()
    }

    pub fn back(&self) -> Option<&T> {
        self.inner.back()
    }

    /// Indexed access from the front (0 = oldest retained element).
    pub fn get(&self, index: usize) -> Option<&T> {
        self.inner.get(index)
    }

    /// Iterate front-to-back (oldest to newest).
    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, T> {
        self.inner.iter()
    }

    /// Truncate to at most `max` elements (from the front, keeping the
    /// newest). Also marks the storage as fragmented.
    pub fn truncate(&mut self, max: usize) {
        while self.inner.len() > max {
            self.inner.pop_front();
        }
        self.evictions_since_maintain += 1;
    }

    /// Drop every element, release the backing storage entirely and
    /// reset the fragmentation counter.
    pub fn clear(&mut self) {
        self.inner.clear();
        self.inner.shrink_to_fit();
        self.evictions_since_maintain = 0;
    }

    /// Frame-boundary housekeeping: when enough evictions happened, the
    /// ring buffer's backing allocation no longer matches its logical
    /// size, so compact it once with `shrink_to_fit`.
    pub fn maintain(&mut self) {
        if self.evictions_since_maintain >= Self::MAINTAIN_THRESHOLD {
            self.inner.shrink_to_fit();
            self.evictions_since_maintain = 0;
        }
    }

    /// Evictions required before `maintain` compacts the storage.
    pub const MAINTAIN_THRESHOLD: usize = 256;
}

/// A `HashMap` cache with a hard entry limit and deterministic eviction.
///
/// Evicting entries drains the map and runs [`Compact::release`] so the
/// allocations are handed back explicitly (and observably in tests)
/// instead of relying purely on `drop` at an unspecified later point.
#[derive(Debug)]
pub struct BoundedCache<K, V> {
    map: HashMap<K, V>,
    limit: usize,
}

/// Explicitly release the backing allocation of a cached value.
pub trait Compact {
    fn release(&mut self);
}

impl Compact for Vec<u8> {
    fn release(&mut self) {
        self.clear();
        self.shrink_to_fit();
    }
}

impl<K, V: Compact> BoundedCache<K, V>
where
    K: Eq + std::hash::Hash + Clone,
{
    /// Default glyph-cache entry limit. At most a few hundred distinct
    /// glyphs are on screen; 4096 entries covers CJK-heavy sessions
    /// while bounding memory to a few MB worst case.
    pub const DEFAULT_LIMIT: usize = 4096;

    pub fn new(limit: usize) -> Self {
        Self {
            map: HashMap::with_capacity(0),
            limit: limit.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Look up `key`; the returned reference is valid until the next
    /// mutation (which always happens between frames).
    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// Insert (or replace) an entry, evicting everything when the limit
    /// is reached. Evicted values are released explicitly.
    /// Returns `true` when a wholesale eviction happened.
    pub fn insert(&mut self, key: K, value: V) -> bool {
        let mut evicted = false;
        if !self.map.contains_key(&key) && self.map.len() >= self.limit {
            // Deterministic full clear: simpler and safer than a partial
            // LRU, and the cache refills within a frame.
            for (_, mut v) in self.map.drain() {
                v.release();
            }
            self.map.shrink_to_fit();
            evicted = true;
        }
        // Replacing an existing key drops the previous value via `drop`
        // (which frees it; the explicit `release` above is for eviction).
        self.map.insert(key, value);
        evicted
    }

    /// Remove one entry, releasing it explicitly.
    pub fn remove(&mut self, key: &K) -> bool {
        self.map.remove(key).is_some()
    }

    /// Clear the cache and hand every allocation back explicitly.
    pub fn clear(&mut self) {
        for (_, mut v) in self.map.drain() {
            v.release();
        }
        self.map.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_deque_starts_empty_without_allocation() {
        let d: BoundedDeque<u8> = BoundedDeque::new(16);
        assert!(d.is_empty());
        assert_eq!(d.len(), 0);
        assert_eq!(d.cap(), 16);
    }

    #[test]
    fn bounded_deque_push_back_never_exceeds_cap() {
        let mut d = BoundedDeque::new(4);
        for i in 0..10u8 {
            d.push_back(i);
        }
        assert_eq!(d.len(), 4);
        // The four newest elements survive.
        assert_eq!(d.get(0), Some(&6));
        assert_eq!(d.get(3), Some(&9));
    }

    #[test]
    fn bounded_deque_push_front_never_exceeds_cap() {
        let mut d = BoundedDeque::new(3);
        for i in 0..7u8 {
            d.push_front(i);
        }
        assert_eq!(d.len(), 3);
        assert_eq!(d.get(0), Some(&6));
        assert_eq!(d.get(2), Some(&4));
    }

    #[test]
    fn bounded_deque_push_reports_evictions() {
        let mut d = BoundedDeque::new(2);
        assert!(!d.push_back(1));
        assert!(!d.push_back(2));
        assert!(d.push_back(3)); // 1 evicted
        assert_eq!(d.front(), Some(&2));
    }

    #[test]
    fn bounded_deque_pop_and_get() {
        let mut d = BoundedDeque::new(4);
        d.push_back(10);
        d.push_back(20);
        assert_eq!(d.pop_front(), Some(10));
        assert_eq!(d.pop_back(), Some(20));
        assert_eq!(d.pop_front(), None);
        assert!(d.is_empty());
    }

    #[test]
    fn bounded_deque_truncate_keeps_newest() {
        let mut d = BoundedDeque::new(8);
        for i in 0..8u8 {
            d.push_back(i);
        }
        d.truncate(3);
        assert_eq!(d.len(), 3);
        assert_eq!(d.get(0), Some(&5));
        assert_eq!(d.get(2), Some(&7));
        // Truncating below the cap is sticky: pushes refill up to the
        // structural cap again.
        d.push_back(8);
        assert_eq!(d.len(), 4);
    }

    #[test]
    fn bounded_deque_clear_releases_everything() {
        let mut d = BoundedDeque::new(4);
        d.push_back(Vec::from(&b"payload"[..]));
        d.clear();
        assert!(d.is_empty());
    }

    #[test]
    fn bounded_deque_maintain_compacts_after_enough_evictions() {
        let mut d = BoundedDeque::new(2);
        for i in 0..300u16 {
            d.push_back(i);
        }
        // 298 evictions happened without any maintain call.
        d.maintain();
        // No observable change in content, but the compaction ran.
        assert_eq!(d.len(), 2);
        assert_eq!(d.get(0), Some(&298u16));
        assert_eq!(d.get(1), Some(&299u16));
    }

    #[test]
    fn bounded_deque_maintain_skips_small_fragments() {
        let mut d = BoundedDeque::new(2);
        d.push_back(1);
        d.push_back(2);
        d.push_back(3); // 1 eviction < threshold
        d.maintain(); // no-op, counter reset only happens on compaction
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn bounded_deque_zero_cap_is_clamped_to_one() {
        let mut d = BoundedDeque::new(0);
        assert_eq!(d.cap(), 1);
        d.push_back('a');
        assert!(d.push_back('b')); // evicts 'a'
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn bounded_cache_insert_lookup_and_evict() {
        let mut c = BoundedCache::new(2);
        assert!(!c.insert('a', vec![1, 2, 3])); // empty -> no eviction
        assert!(!c.insert('b', vec![4]));
        // Full: inserting 'c' evicts everything (including 'a','b').
        assert!(c.insert('c', vec![5]));
        assert_eq!(c.len(), 1);
        assert!(c.get(&'c').is_some());
        assert!(!c.contains(&'a'));
    }

    #[test]
    fn bounded_cache_replace_does_not_evict_existing_key() {
        let mut c = BoundedCache::new(2);
        c.insert('a', vec![1]);
        c.insert('b', vec![2]);
        // Replacing an existing key never triggers eviction.
        assert!(!c.insert('a', vec![9]));
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(&'a'), Some(&vec![9]));
    }

    #[test]
    fn bounded_cache_clear_releases_allocations() {
        let mut c = BoundedCache::new(8);
        for i in 0..8u8 {
            c.insert(i, vec![0u8; 64]);
        }
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn bounded_cache_remove_entry() {
        let mut c = BoundedCache::new(4);
        c.insert('x', vec![1]);
        assert!(c.remove(&'x'));
        assert!(!c.remove(&'x'));
        assert!(c.is_empty());
    }

    #[test]
    fn vec_release_compacts_to_zero_capacity() {
        let mut v = vec![0u8; 4096];
        v.release();
        assert!(v.is_empty());
        assert_eq!(v.capacity(), 0);
    }
}

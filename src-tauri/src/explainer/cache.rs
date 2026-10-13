//! Small in-memory LRU cache: when full, the entry used least recently is dropped.

use std::collections::HashMap;
use std::hash::Hash;

pub struct LruCache<K, V> {
    /// Value and the time it was last used (`clock` at that moment).
    entries: HashMap<K, (V, u64)>,
    capacity: usize,
    clock: u64,
}

impl<K: Eq + Hash + Clone, V> LruCache<K, V> {
    /// `capacity` is at least 1.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self { entries: HashMap::with_capacity(capacity), capacity, clock: 0 }
    }

    /// Returns the value and marks it as just used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let now = self.tick();
        let (value, used) = self.entries.get_mut(key)?;
        *used = now;
        Some(value)
    }

    /// Inserts or replaces a value. When full, drops the least recently used entry first.
    pub fn put(&mut self, key: K, value: V) {
        let now = self.tick();
        if !self.entries.contains_key(&key) && self.entries.len() >= self.capacity {
            // A linear scan is fine for a few hundred entries and keeps this simple.
            let oldest = self.entries.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (value, now));
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_and_put() {
        let mut cache = LruCache::new(2);
        assert_eq!(cache.get(&"a"), None);
        cache.put("a", 1);
        assert_eq!(cache.get(&"a"), Some(&1));
    }

    #[test]
    fn least_recently_used_is_evicted_when_full() {
        let mut cache = LruCache::new(2);
        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("c", 3);
        assert_eq!(cache.get(&"a"), None);
        assert_eq!(cache.get(&"b"), Some(&2));
        assert_eq!(cache.get(&"c"), Some(&3));
    }

    #[test]
    fn get_refreshes_recency() {
        let mut cache = LruCache::new(2);
        cache.put("a", 1);
        cache.put("b", 2);
        cache.get(&"a");
        cache.put("c", 3);
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(cache.get(&"a"), Some(&1));
    }

    #[test]
    fn replacing_a_key_evicts_nothing() {
        let mut cache = LruCache::new(2);
        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("a", 10);
        assert_eq!(cache.get(&"a"), Some(&10));
        assert_eq!(cache.get(&"b"), Some(&2));
    }

    #[test]
    fn capacity_one_and_zero() {
        let mut cache = LruCache::new(0);
        cache.put("a", 1);
        cache.put("b", 2);
        assert_eq!(cache.get(&"a"), None);
        assert_eq!(cache.get(&"b"), Some(&2));
    }
}

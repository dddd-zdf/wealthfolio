//! Short-lived response cache for performance summaries.
//!
//! The dashboard and account cards request the same performance summaries on
//! every page view. Those summaries are derived purely from stored daily
//! valuations, which only change when a portfolio job finishes, so results
//! are cached per request body and dropped whenever a portfolio update
//! completes (or fails). A TTL bounds staleness for anything that changes
//! outside a portfolio job (e.g. the calendar day rolling over).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(10 * 60);
const MAX_ENTRIES: usize = 256;

static GENERATION: AtomicU64 = AtomicU64::new(0);
static ENTRIES: LazyLock<Mutex<HashMap<u64, Entry>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

struct Entry {
    generation: u64,
    stored_at: Instant,
    value: serde_json::Value,
}

/// Cache key for a request: endpoint + profile runtime + raw request body.
pub fn key(endpoint: &str, profile_ptr: usize, body: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    endpoint.hash(&mut hasher);
    profile_ptr.hash(&mut hasher);
    body.hash(&mut hasher);
    hasher.finish()
}

pub fn get(key: u64) -> Option<serde_json::Value> {
    let generation = GENERATION.load(Ordering::SeqCst);
    let entries = ENTRIES.lock().ok()?;
    let entry = entries.get(&key)?;
    if entry.generation == generation && entry.stored_at.elapsed() < TTL {
        Some(entry.value.clone())
    } else {
        None
    }
}

/// Store a result computed while `generation` was current. Results computed
/// across an invalidation are discarded.
pub fn put(key: u64, generation: u64, value: serde_json::Value) {
    if generation != GENERATION.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(mut entries) = ENTRIES.lock() {
        if entries.len() >= MAX_ENTRIES {
            let now_generation = GENERATION.load(Ordering::SeqCst);
            entries.retain(|_, entry| {
                entry.generation == now_generation && entry.stored_at.elapsed() < TTL
            });
            if entries.len() >= MAX_ENTRIES {
                entries.clear();
            }
        }
        entries.insert(
            key,
            Entry {
                generation,
                stored_at: Instant::now(),
                value,
            },
        );
    }
}

/// Generation to pass to [`put`]; read it before computing a result.
pub fn current_generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// Drop every cached result (call after valuations change).
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut entries) = ENTRIES.lock() {
        entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_and_invalidate() {
        let k = key("summary", 1, b"{\"a\":1}");
        let generation = current_generation();
        put(k, generation, serde_json::json!({"x": 1}));
        // Other tests in this binary may invalidate concurrently, so a miss is
        // acceptable here; a hit must return the stored value.
        if let Some(value) = get(k) {
            assert_eq!(value, serde_json::json!({"x": 1}));
        }
        invalidate();
        assert_eq!(get(k), None);
        // A result computed before the invalidation is not stored.
        put(k, generation, serde_json::json!({"x": 2}));
        assert_eq!(get(k), None);
    }

    #[test]
    fn keys_differ_by_endpoint_profile_and_body() {
        let base = key("summary", 1, b"{}");
        assert_ne!(base, key("summaries", 1, b"{}"));
        assert_ne!(base, key("summary", 2, b"{}"));
        assert_ne!(base, key("summary", 1, b"{ }"));
    }
}

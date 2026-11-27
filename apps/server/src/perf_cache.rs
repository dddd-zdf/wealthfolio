//! Short-lived response cache for performance summaries.
//!
//! The dashboard and account cards request the same performance summaries on
//! every page view. Those summaries are derived purely from stored daily
//! valuations, which only change when a portfolio job finishes, so results
//! are cached per request and dropped whenever a portfolio update completes
//! (or fails). Dated requests change key when the calendar day rolls over, so
//! the TTL only bounds staleness for anything changed outside a portfolio job.
//! Results that also depend on live prices use [`put_with_ttl`] for a shorter
//! lifetime.
//!
//! Every invalidation also schedules a background warm-up (see
//! [`register_warm_target`]) so the dashboard's standard periods and the
//! health check are ready before the next page view.

use std::collections::HashMap;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

use crate::main_lib::AppState;

const TTL: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_ENTRIES: usize = 256;

static GENERATION: AtomicU64 = AtomicU64::new(0);
static ENTRIES: LazyLock<Mutex<HashMap<u64, Entry>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

type PendingResults = HashMap<(u64, u64), Weak<OnceCell<serde_json::Value>>>;
static PENDING: LazyLock<Mutex<PendingResults>> = LazyLock::new(|| Mutex::new(HashMap::new()));

struct Entry {
    generation: u64,
    stored_at: Instant,
    ttl: Duration,
    value: serde_json::Value,
}

/// Cache key for a request: endpoint + profile runtime + canonical request.
/// Callers pass a canonical serialization of the parsed request so that
/// server-side warm-up and client requests share entries.
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
    if entry.generation == generation && entry.stored_at.elapsed() < entry.ttl {
        Some(entry.value.clone())
    } else {
        None
    }
}

/// Concurrent misses for one request and cache generation share one calculation.
/// After invalidation, new readers start with a new cell. Failed or cancelled
/// calculations leave the cell empty, as they would the response cache.
pub(crate) async fn get_or_calculate<E>(
    key: u64,
    calculate: impl Future<Output = Result<serde_json::Value, E>>,
) -> Result<serde_json::Value, E> {
    get_or_calculate_for_generation(key, current_generation(), calculate).await
}

async fn get_or_calculate_for_generation<E>(
    key: u64,
    generation: u64,
    calculate: impl Future<Output = Result<serde_json::Value, E>>,
) -> Result<serde_json::Value, E> {
    if let Some(value) = get(key) {
        return Ok(value);
    }
    let cell = {
        let mut pending = PENDING.lock().unwrap_or_else(|error| error.into_inner());
        pending.retain(|_, cell| cell.strong_count() > 0);
        let entry = pending.entry((key, generation)).or_default();
        entry.upgrade().unwrap_or_else(|| {
            let cell = Arc::new(OnceCell::new());
            *entry = Arc::downgrade(&cell);
            cell
        })
    };
    cell.get_or_try_init(|| async {
        if let Some(value) = get(key) {
            return Ok(value);
        }
        let value = calculate.await?;
        put(key, generation, value.clone());
        Ok(value)
    })
    .await
    .cloned()
}

/// Store a result computed while `generation` was current. Results computed
/// across an invalidation are discarded.
pub fn put(key: u64, generation: u64, value: serde_json::Value) {
    put_with_ttl(key, generation, value, TTL);
}

/// [`put`] with a lifetime shorter than the default.
pub fn put_with_ttl(key: u64, generation: u64, value: serde_json::Value, ttl: Duration) {
    if generation != GENERATION.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(mut entries) = ENTRIES.lock() {
        if entries.len() >= MAX_ENTRIES {
            let now_generation = GENERATION.load(Ordering::SeqCst);
            entries.retain(|_, entry| {
                entry.generation == now_generation && entry.stored_at.elapsed() < entry.ttl
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
                ttl,
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

/// Invalidate and schedule a warm-up of the standard dashboard requests.
/// Used after data edits only: the update job that runs on every app open
/// would otherwise recompute every period each time, which saturates small hosts.
pub fn invalidate_and_warm() {
    invalidate();
    schedule_warm_up();
}

static WARM_TARGETS: LazyLock<Mutex<Vec<Weak<AppState>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static WARMING: AtomicBool = AtomicBool::new(false);
static WARM_AGAIN: AtomicBool = AtomicBool::new(false);

/// Register a profile runtime whose dashboard data is warmed after each
/// invalidation. Held weakly so a closed profile is simply skipped.
pub fn register_warm_target(state: &Arc<AppState>) {
    if let Ok(mut targets) = WARM_TARGETS.lock() {
        targets.retain(|target| target.strong_count() > 0);
        targets.push(Arc::downgrade(state));
    }
}

fn schedule_warm_up() {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    // Coalesce bursts of invalidations into one running warm-up plus at most
    // one follow-up pass.
    if WARMING.swap(true, Ordering::SeqCst) {
        WARM_AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    handle.spawn(async {
        loop {
            WARM_AGAIN.store(false, Ordering::SeqCst);
            let targets: Vec<Arc<AppState>> = WARM_TARGETS
                .lock()
                .map(|targets| targets.iter().filter_map(Weak::upgrade).collect())
                .unwrap_or_default();
            for state in targets {
                crate::api::health::fork_refresh::warm_health_status(&state).await;
                crate::api::performance::fork_cache::warm_dashboard_summaries(&state).await;
            }
            if !WARM_AGAIN.load(Ordering::SeqCst) {
                break;
            }
        }
        WARMING.store(false, Ordering::SeqCst);
    });
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

    #[tokio::test]
    async fn overlapping_performance_summary_reads_compute_once() {
        let k = key("pending-summary", 1, b"{}");
        let generation = current_generation();
        let calls = std::cell::Cell::new(0);
        let calculate = || async {
            calls.set(calls.get() + 1);
            tokio::task::yield_now().await;
            Ok::<_, ()>(serde_json::json!(42))
        };

        let both = tokio::join!(
            get_or_calculate_for_generation(k, generation, calculate()),
            get_or_calculate_for_generation(k, generation, calculate()),
        );
        assert_eq!(both, (Ok(serde_json::json!(42)), Ok(serde_json::json!(42))));
        assert_eq!(calls.get(), 1);
    }

    #[tokio::test]
    async fn invalidation_does_not_wait_for_an_older_calculation() {
        let k = key("pending-summary-invalidation", 1, b"{}");
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();

        let (old, new) = tokio::join!(
            get_or_calculate(k, async {
                started_tx.send(()).unwrap();
                finish_rx.await.unwrap();
                Ok::<_, ()>(serde_json::json!(1))
            }),
            async {
                started_rx.await.unwrap();
                invalidate();
                let current = tokio::time::timeout(
                    Duration::from_secs(5),
                    get_or_calculate(k, async { Ok::<_, ()>(serde_json::json!(2)) }),
                )
                .await
                .expect("the new generation must not wait for the old one");
                finish_tx.send(()).unwrap();
                current
            },
        );
        assert_eq!(old, Ok(serde_json::json!(1)));
        assert_eq!(new, Ok(serde_json::json!(2)));
        if let Some(value) = get(k) {
            assert_eq!(value, serde_json::json!(2));
        }
    }

    #[test]
    fn short_ttl_entries_expire() {
        let k = key("intraday", 1, b"{}");
        put_with_ttl(
            k,
            current_generation(),
            serde_json::json!(1),
            Duration::ZERO,
        );
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

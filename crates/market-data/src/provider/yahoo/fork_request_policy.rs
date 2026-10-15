//! Process-wide request pacing, cooldown and latest-quote caching for Yahoo.
//!
//! Yahoo providers are rebuilt when provider settings change.  Keeping this
//! state outside `YahooProvider` means those rebuilds cannot reset the request
//! budget or discard the short-lived latest quote cache.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

use crate::errors::MarketDataError;
use crate::models::Quote;

const PROVIDER: &str = "YAHOO";
const DEFAULT_MIN_SPACING: Duration = Duration::from_secs(2);
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(15 * 60);
const DEFAULT_MAX_QUEUE_WAIT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_PENDING: usize = 32;
const LATEST_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const LATEST_CACHE_CAPACITY: usize = 256;

/// Every direct request must finish in bounded time while holding the shared gate.
pub(crate) fn http_client() -> Result<reqwest::Client, MarketDataError> {
    wealthfolio_http::client_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| MarketDataError::ProviderError {
            provider: PROVIDER.to_string(),
            message: "Unable to initialize Yahoo HTTP client".into(),
        })
}

#[derive(Clone, Debug)]
pub struct CachedYahooQuote {
    pub quote: Quote,
    pub fetched_at: DateTime<Utc>,
    pub is_stale: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct YahooPolicyConfig {
    pub min_spacing: Duration,
    pub cooldown: Duration,
    pub max_queue_wait: Duration,
    pub max_pending: usize,
}

impl Default for YahooPolicyConfig {
    fn default() -> Self {
        Self {
            min_spacing: DEFAULT_MIN_SPACING,
            cooldown: DEFAULT_COOLDOWN,
            max_queue_wait: DEFAULT_MAX_QUEUE_WAIT,
            max_pending: DEFAULT_MAX_PENDING,
        }
    }
}

#[derive(Default)]
struct PolicyState {
    next_dispatch: Option<Instant>,
    cooldown_until: Option<Instant>,
}

#[derive(Clone)]
struct CachedEntry {
    quote: Quote,
    fetched_at: DateTime<Utc>,
}

pub(crate) struct YahooRequestPolicy {
    config: YahooPolicyConfig,
    state: Mutex<PolicyState>,
    network_lock: Arc<Mutex<()>>,
    pending: Arc<Semaphore>,
    latest: RwLock<HashMap<String, CachedEntry>>,
    latest_miss_lock: Mutex<()>,
}

impl YahooRequestPolicy {
    pub(crate) fn new(config: YahooPolicyConfig) -> Self {
        let max_pending = config.max_pending;
        Self {
            config,
            state: Mutex::new(PolicyState::default()),
            network_lock: Arc::new(Mutex::new(())),
            pending: Arc::new(Semaphore::new(max_pending)),
            latest: RwLock::new(HashMap::new()),
            latest_miss_lock: Mutex::new(()),
        }
    }

    /// Acquire the serialized physical-request gate. The returned permit must
    /// live until the response and any rate-limit observation are complete.
    pub(crate) async fn acquire(&self) -> Result<YahooRequestPermit, MarketDataError> {
        let pending = self
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| rate_limited_error())?;
        let network = match tokio::time::timeout(
            self.config.max_queue_wait,
            self.network_lock.clone().lock_owned(),
        )
        .await
        {
            Ok(network) => network,
            Err(_) => return Err(rate_limited_error()),
        };
        let scheduled = {
            let mut state = self.state.lock().await;
            let now = Instant::now();
            if state.cooldown_until.is_some_and(|until| until > now) {
                return Err(rate_limited_error());
            }
            let scheduled = state.next_dispatch.map_or(now, |next| next.max(now));
            if scheduled.saturating_duration_since(now) > self.config.max_queue_wait {
                return Err(rate_limited_error());
            }

            state.next_dispatch = Some(scheduled + self.config.min_spacing);
            scheduled
        };

        let wait = scheduled.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }

        let state = self.state.lock().await;
        if state
            .cooldown_until
            .is_some_and(|until| until > Instant::now())
        {
            return Err(rate_limited_error());
        }
        Ok(YahooRequestPermit {
            _network: network,
            _pending: pending,
        })
    }

    pub(crate) async fn note_rate_limited(&self, retry_after: Option<Duration>) {
        let duration =
            retry_after.map_or(self.config.cooldown, |hint| hint.max(self.config.cooldown));
        let until = Instant::now() + duration;
        let mut state = self.state.lock().await;
        state.cooldown_until = Some(
            state
                .cooldown_until
                .map_or(until, |existing| existing.max(until)),
        );
    }

    pub(crate) async fn latest_or_fetch<F>(
        &self,
        symbol: &str,
        fetch: F,
    ) -> Result<Quote, MarketDataError>
    where
        F: Future<Output = Result<Quote, MarketDataError>> + Send,
    {
        if let Some(cached) = self.cached_quote(symbol).filter(|cached| !cached.is_stale) {
            return Ok(cached.quote);
        }
        let _miss = self.latest_miss_lock.lock().await;
        if let Some(cached) = self.cached_quote(symbol).filter(|cached| !cached.is_stale) {
            return Ok(cached.quote);
        }
        let quote = fetch.await?;
        if quote.source != PROVIDER
            || quote.close <= rust_decimal::Decimal::ZERO
            || quote.currency.trim().is_empty()
        {
            return Err(MarketDataError::ValidationFailed {
                message: "Yahoo latest quote has no usable price, currency, or source".into(),
            });
        }
        self.put_latest_quote(symbol, quote.clone(), Utc::now());
        Ok(quote)
    }

    pub(crate) fn cached_quote(&self, symbol: &str) -> Option<CachedYahooQuote> {
        let latest = self
            .latest
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        latest.get(symbol).map(|entry| CachedYahooQuote {
            quote: entry.quote.clone(),
            fetched_at: entry.fetched_at,
            is_stale: is_stale(entry.fetched_at),
        })
    }

    pub(crate) fn put_latest_quote(&self, symbol: &str, quote: Quote, fetched_at: DateTime<Utc>) {
        if quote.source != PROVIDER
            || quote.close <= rust_decimal::Decimal::ZERO
            || quote.currency.trim().is_empty()
        {
            return;
        }
        let mut latest = self
            .latest
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !latest.contains_key(symbol) && latest.len() >= LATEST_CACHE_CAPACITY {
            if let Some(oldest) = latest
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(key, _)| key.clone())
            {
                latest.remove(&oldest);
            }
        }
        if latest
            .get(symbol)
            .is_some_and(|existing| existing.quote.timestamp > quote.timestamp)
        {
            return;
        }
        latest.insert(symbol.to_string(), CachedEntry { quote, fetched_at });
    }

    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.latest
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// Holds the serialized network slot until the response has been observed.
/// Dropping it also releases the bounded pending-request slot on cancellation.
pub(crate) struct YahooRequestPermit {
    _network: OwnedMutexGuard<()>,
    _pending: OwnedSemaphorePermit,
}

fn is_stale(fetched_at: DateTime<Utc>) -> bool {
    Utc::now()
        .signed_duration_since(fetched_at)
        .to_std()
        .is_ok_and(|age| age > LATEST_CACHE_TTL)
}

pub(crate) fn rate_limited_error() -> MarketDataError {
    MarketDataError::RateLimited {
        provider: PROVIDER.to_string(),
    }
}

static GLOBAL_POLICY: OnceLock<Arc<YahooRequestPolicy>> = OnceLock::new();

pub(crate) fn global_policy() -> Arc<YahooRequestPolicy> {
    GLOBAL_POLICY
        .get_or_init(|| Arc::new(YahooRequestPolicy::new(YahooPolicyConfig::default())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use rust_decimal_macros::dec;

    fn quote() -> Quote {
        Quote::new(Utc::now(), dec!(123.45), "USD".into(), PROVIDER.into())
    }

    #[tokio::test]
    async fn cooldown_blocks_following_request_and_retry_hint_extends_it() {
        let policy = YahooRequestPolicy::new(YahooPolicyConfig {
            min_spacing: Duration::ZERO,
            cooldown: Duration::from_millis(30),
            max_queue_wait: Duration::from_secs(1),
            max_pending: 4,
        });
        policy
            .note_rate_limited(Some(Duration::from_millis(60)))
            .await;
        assert!(matches!(
            policy.acquire().await,
            Err(MarketDataError::RateLimited { .. })
        ));
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert!(policy.acquire().await.is_ok());
    }

    #[tokio::test]
    async fn cache_reports_freshness_and_is_bounded() {
        let policy = YahooRequestPolicy::new(YahooPolicyConfig::default());
        for index in 0..(LATEST_CACHE_CAPACITY + 1) {
            policy.put_latest_quote(&format!("S{index}"), quote(), Utc::now());
        }
        assert_eq!(policy.cache_len(), LATEST_CACHE_CAPACITY);

        let old = Utc::now() - ChronoDuration::minutes(6);
        policy.put_latest_quote("OLD", quote(), old);
        assert!(policy.cached_quote("OLD").unwrap().is_stale);
    }

    #[tokio::test]
    async fn latest_miss_lock_coalesces_concurrent_misses() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let policy = Arc::new(YahooRequestPolicy::new(YahooPolicyConfig::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let policy = policy.clone();
            let calls = calls.clone();
            tasks.push(tokio::spawn(async move {
                policy
                    .latest_or_fetch("FINN.NE", async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        Ok(quote())
                    })
                    .await
                    .unwrap()
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap().close, dec!(123.45));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let cached = policy
            .latest_or_fetch("FINN.NE", async { panic!("fresh hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(cached.close, dec!(123.45));
    }

    #[tokio::test]
    async fn request_permit_serializes_until_response_is_observed() {
        let policy = Arc::new(YahooRequestPolicy::new(YahooPolicyConfig {
            min_spacing: Duration::ZERO,
            cooldown: Duration::from_secs(1),
            max_queue_wait: Duration::from_millis(100),
            max_pending: 4,
        }));
        let first = policy.acquire().await.unwrap();
        let waiting = {
            let policy = policy.clone();
            tokio::spawn(async move { policy.acquire().await.is_ok() })
        };
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!waiting.is_finished());
        drop(first);
        assert!(waiting.await.unwrap());
    }

    #[test]
    fn global_policy_is_shared_across_provider_rebuilds() {
        assert!(Arc::ptr_eq(&global_policy(), &global_policy()));
    }

    #[tokio::test]
    async fn queued_request_observes_cooldown_before_dispatch_and_cancellation_releases_slot() {
        let policy = Arc::new(YahooRequestPolicy::new(YahooPolicyConfig {
            min_spacing: Duration::ZERO,
            cooldown: Duration::from_secs(1),
            max_queue_wait: Duration::from_millis(100),
            max_pending: 2,
        }));
        let first = policy.acquire().await.unwrap();
        let waiting = {
            let policy = policy.clone();
            tokio::spawn(async move { policy.acquire().await.is_ok() })
        };
        tokio::task::yield_now().await;
        policy.note_rate_limited(None).await;
        drop(first);
        assert!(!waiting.await.unwrap());
        assert_eq!(policy.pending.available_permits(), 2);
    }

    #[test]
    fn older_or_invalid_quotes_cannot_replace_current_listing_cache() {
        let policy = YahooRequestPolicy::new(YahooPolicyConfig::default());
        let current = quote();
        policy.put_latest_quote("FINN.NE", current.clone(), Utc::now());
        let mut old = current.clone();
        old.timestamp -= ChronoDuration::days(1);
        old.close = dec!(1);
        policy.put_latest_quote("FINN.NE", old, Utc::now());
        assert_eq!(
            policy.cached_quote("FINN.NE").unwrap().quote.close,
            current.close
        );
        let mut invalid = current.clone();
        invalid.close = dec!(0);
        policy.put_latest_quote("INVALID", invalid, Utc::now());
        assert!(policy.cached_quote("INVALID").is_none());
        policy.put_latest_quote(
            "FINN",
            Quote::new(
                current.timestamp,
                dec!(19000),
                "USD".into(),
                PROVIDER.into(),
            ),
            Utc::now(),
        );
        assert_eq!(
            policy.cached_quote("FINN.NE").unwrap().quote.close,
            current.close
        );
        assert_eq!(
            policy.cached_quote("FINN").unwrap().quote.close,
            dec!(19000)
        );
    }
}

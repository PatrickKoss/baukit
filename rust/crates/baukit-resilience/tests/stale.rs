//! `StaleCache` behaviour on a manual clock.

use std::future::{Ready, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use baukit_resilience::{
    CacheRead, Freshness, ManualClock, StaleCache, StaleCacheError, StalePolicy,
};
use tokio::sync::Notify;

const FRESH: Duration = Duration::from_secs(300);
const STALE: Duration = Duration::from_secs(3600);
const COOLDOWN: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Down;

fn cache(clock: &Arc<ManualClock>) -> StaleCache<u32> {
    StaleCache::with_clock(StalePolicy::new(FRESH, STALE, COOLDOWN), clock.clone())
}

fn read(value: u32, freshness: Freshness) -> Result<CacheRead<u32>, StaleCacheError<Down>> {
    Ok(CacheRead {
        value,
        freshness,
        refresh_failed: false,
    })
}

fn failed_refresh(value: u32) -> Result<CacheRead<u32>, StaleCacheError<Down>> {
    Ok(CacheRead {
        value,
        freshness: Freshness::Stale,
        refresh_failed: true,
    })
}

struct Source {
    calls: AtomicUsize,
}

impl Source {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn ok(&self, value: u32) -> Ready<Result<u32, Down>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Ok(value))
    }

    fn down(&self) -> Ready<Result<u32, Down>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Err(Down))
    }
}

#[tokio::test]
async fn fresh_value_is_served_without_fetching() {
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&clock);
    let source = Source::new();
    assert_eq!(
        cache.get_or_refresh(|| source.ok(1)).await,
        read(1, Freshness::Fetched)
    );
    clock.advance(FRESH - Duration::from_millis(1));
    assert_eq!(
        cache.get_or_refresh(|| source.ok(2)).await,
        read(1, Freshness::Fresh)
    );
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn stale_value_is_served_while_the_source_fails() {
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&clock);
    let source = Source::new();
    let _ = cache.get_or_refresh(|| source.ok(1)).await;
    clock.advance(FRESH);
    assert_eq!(
        cache.get_or_refresh(|| source.down()).await,
        failed_refresh(1)
    );
    assert_eq!(
        cache.get_or_refresh(|| source.down()).await,
        read(1, Freshness::Stale)
    );
    clock.advance(COOLDOWN);
    assert_eq!(
        cache.get_or_refresh(|| source.down()).await,
        failed_refresh(1)
    );
    assert_eq!(source.calls(), 3);
    clock.advance(COOLDOWN);
    assert_eq!(
        cache.get_or_refresh(|| source.ok(2)).await,
        read(2, Freshness::Fetched)
    );
}

#[tokio::test]
async fn cooldown_stops_a_fetch_per_request() {
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&clock);
    let source = Source::new();
    assert_eq!(
        cache.get_or_refresh(|| source.down()).await,
        Err(StaleCacheError::Fetch(Down))
    );
    for _ in 0..10 {
        clock.advance(Duration::from_millis(100));
        let result = cache.get_or_refresh(|| source.ok(1)).await;
        assert!(
            matches!(result, Err(StaleCacheError::CoolingDown { retry_after }) if retry_after < COOLDOWN)
        );
    }
    assert_eq!(source.calls(), 1);
    clock.advance(COOLDOWN);
    assert_eq!(
        cache.get_or_refresh(|| source.ok(1)).await,
        read(1, Freshness::Fetched)
    );
}

#[tokio::test]
async fn value_expires_after_the_stale_window() {
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&clock);
    let source = Source::new();
    let _ = cache.get_or_refresh(|| source.ok(1)).await;
    clock.advance(FRESH + STALE - Duration::from_millis(1));
    assert_eq!(
        cache.get_or_refresh(|| source.down()).await,
        failed_refresh(1)
    );
    clock.advance(Duration::from_millis(1));
    assert_eq!(cache.current(), None);
    clock.advance(COOLDOWN);
    assert_eq!(
        cache.get_or_refresh(|| source.down()).await,
        Err(StaleCacheError::Fetch(Down))
    );
}

#[tokio::test]
async fn force_refresh_fetches_a_fresh_value_but_respects_the_cooldown() {
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&clock);
    let source = Source::new();
    let _ = cache.get_or_refresh(|| source.ok(1)).await;
    assert_eq!(
        cache.force_refresh(|| source.ok(2)).await,
        read(2, Freshness::Fetched)
    );
    assert_eq!(
        cache.force_refresh(|| source.ok(3)).await,
        read(2, Freshness::Fresh)
    );
    assert_eq!(source.calls(), 2);
    clock.advance(COOLDOWN);
    assert_eq!(
        cache.force_refresh(|| source.down()).await,
        Ok(CacheRead {
            value: 2,
            freshness: Freshness::Fresh,
            refresh_failed: true,
        })
    );
    assert_eq!(
        cache.force_refresh(|| source.ok(4)).await,
        read(2, Freshness::Fresh)
    );
    assert_eq!(source.calls(), 3);
}

#[tokio::test]
async fn force_refresh_on_an_empty_cache_reports_the_cooldown() {
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&clock);
    let source = Source::new();
    assert_eq!(
        cache.force_refresh(|| source.down()).await,
        Err(StaleCacheError::Fetch(Down))
    );
    let result = cache.force_refresh(|| source.ok(1)).await;
    assert_eq!(
        result,
        Err(StaleCacheError::CoolingDown {
            retry_after: COOLDOWN
        })
    );
    assert_eq!(source.calls(), 1);
}

const READERS: usize = 32;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_readers_of_an_empty_cache_trigger_one_fetch() {
    let clock = Arc::new(ManualClock::new());
    let cache = Arc::new(cache(&clock));
    let source = Arc::new(Source::new());
    let release = Arc::new(Notify::new());
    let mut readers = tokio::task::JoinSet::new();
    for _ in 0..READERS {
        let (cache, source, release) = (cache.clone(), source.clone(), release.clone());
        readers.spawn(async move {
            cache
                .get_or_refresh(|| async move {
                    release.notified().await;
                    source.ok(7).await
                })
                .await
        });
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    release.notify_one();
    let mut freshness = Vec::new();
    while let Some(joined) = readers.join_next().await {
        freshness.push(joined.unwrap().unwrap().freshness);
    }
    assert_eq!(source.calls(), 1);
    assert_eq!(
        freshness
            .iter()
            .filter(|kind| **kind == Freshness::Fetched)
            .count(),
        1
    );
    assert_eq!(freshness.len(), READERS);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn readers_of_a_stale_value_do_not_wait_for_the_refresh() {
    let clock = Arc::new(ManualClock::new());
    let cache = Arc::new(cache(&clock));
    let source = Arc::new(Source::new());
    let _ = cache.get_or_refresh(|| source.ok(1)).await;
    clock.advance(FRESH);
    let release = Arc::new(Notify::new());
    let started = Arc::new(Notify::new());
    let refresher = {
        let (cache, source, release, started) = (
            cache.clone(),
            source.clone(),
            release.clone(),
            started.clone(),
        );
        tokio::spawn(async move {
            cache
                .get_or_refresh(|| async move {
                    started.notify_one();
                    release.notified().await;
                    source.ok(2).await
                })
                .await
        })
    };
    started.notified().await;
    for _ in 0..READERS {
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            cache.get_or_refresh(|| source.ok(3)),
        )
        .await;
        assert_eq!(result.unwrap(), read(1, Freshness::Stale));
    }
    release.notify_one();
    assert_eq!(refresher.await.unwrap(), read(2, Freshness::Fetched));
    assert_eq!(source.calls(), 2);
}

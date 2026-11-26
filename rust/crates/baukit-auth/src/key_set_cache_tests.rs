use std::sync::atomic::AtomicUsize;

use baukit_resilience::ManualClock;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};

use super::*;

const FRESH_FOR: Duration = Duration::from_secs(300);
const STALE_FOR: Duration = Duration::from_secs(3600);
const COOLDOWN: Duration = Duration::from_secs(10);

type Keys = BTreeMap<String, u32>;

#[derive(Default)]
struct ScriptedSource {
    keys: Mutex<Option<Keys>>,
    fetches: AtomicUsize,
}

impl ScriptedSource {
    fn serve(&self, keys: Option<&[(&str, u32)]>) {
        let keys = keys.map(|keys| {
            keys.iter()
                .map(|(id, key)| ((*id).to_owned(), *key))
                .collect()
        });
        *self.keys.lock().unwrap_or_else(PoisonError::into_inner) = keys;
    }

    fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

impl KeySource for Arc<ScriptedSource> {
    type Set = Keys;
    type Error = &'static str;

    fn fetch(&self) -> KeySourceFuture<'_, Keys, &'static str> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        let keys = self
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Box::pin(async move { keys.ok_or("down") })
    }
}

fn cache(
    source: &Arc<ScriptedSource>,
    clock: &Arc<ManualClock>,
) -> KeySetCache<Arc<ScriptedSource>> {
    let policy = StalePolicy::new(FRESH_FOR, STALE_FOR, COOLDOWN);
    KeySetCache::with_clock(
        Arc::clone(source),
        policy,
        "test",
        Arc::clone(clock) as Arc<dyn Clock>,
    )
}

async fn settle(cache: &KeySetCache<Arc<ScriptedSource>>) {
    for _ in 0..100 {
        if !cache.is_refreshing() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("background refresh did not finish");
}

#[tokio::test]
async fn known_ids_survive_an_outage_until_the_stale_window_ends() {
    let source = Arc::new(ScriptedSource::default());
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&source, &clock);
    source.serve(Some(&[("a", 1)]));
    assert_eq!(cache.key_for("a").await.ok(), Some(1));
    assert_eq!(cache.key_for("a").await.ok(), Some(1));
    assert_eq!(source.fetches(), 1);

    source.serve(None);
    clock.advance(FRESH_FOR);
    assert_eq!(cache.key_for("a").await.ok(), Some(1));
    settle(&cache).await;
    assert_eq!(source.fetches(), 2, "one background refresh");
    assert!(matches!(
        cache.key_for("b").await,
        Err(KeyLookupError::UnknownKeyId)
    ));
    assert_eq!(source.fetches(), 2, "unknown id respects the cooldown");

    clock.advance(STALE_FOR);
    assert!(matches!(
        cache.key_for("a").await,
        Err(KeyLookupError::Fetch("down"))
    ));
    assert!(matches!(
        cache.key_for("a").await,
        Err(KeyLookupError::Unavailable { .. })
    ));
}

#[tokio::test]
async fn unknown_ids_refresh_once_and_are_negatively_cached() {
    let source = Arc::new(ScriptedSource::default());
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&source, &clock);
    source.serve(Some(&[("a", 1)]));
    cache.key_for("a").await.ok();

    source.serve(Some(&[("a", 1), ("rotated", 2)]));
    assert_eq!(cache.key_for("rotated").await.ok(), Some(2));
    assert_eq!(source.fetches(), 2);

    clock.advance(COOLDOWN);
    for _ in 0..3 {
        assert!(matches!(
            cache.key_for("missing").await,
            Err(KeyLookupError::UnknownKeyId)
        ));
    }
    assert_eq!(source.fetches(), 3);
}

#[test]
fn unknown_key_cache_has_a_hard_entry_bound() {
    let mut keys = UnknownKeys::default();
    let now = Instant::now();
    for index in 0..=MAX_UNKNOWN_KEYS {
        keys.remember(&format!("unknown-{index}"), now);
    }
    assert_eq!(keys.entries.len(), MAX_UNKNOWN_KEYS);
    assert!(keys.contains("unknown-128", now));
    assert!(!keys.contains("unknown-128", now + UNKNOWN_KEY_TTL));
}

#[tokio::test]
async fn lookups_are_counted_with_the_static_source_label() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let source = Arc::new(ScriptedSource::default());
    let clock = Arc::new(ManualClock::new());
    let cache = cache(&source, &clock);
    source.serve(Some(&[("a", 1)]));
    let guard = metrics::set_default_local_recorder(&recorder);
    cache.key_for("a").await.ok();
    cache.key_for("a").await.ok();
    drop(guard);
    let mut counts = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .map(|(key, _, _, value)| {
            let labels = key
                .key()
                .labels()
                .map(|label| format!("{}={}", label.key(), label.value()))
                .collect::<Vec<_>>()
                .join(",");
            let count = match value {
                DebugValue::Counter(count) => count,
                _ => 0,
            };
            (format!("{}{{{labels}}}", key.key().name()), count)
        })
        .collect::<Vec<_>>();
    counts.sort();
    assert_eq!(
        counts,
        [
            (
                "auth_key_cache_requests_total{source=test,result=fetched}".to_owned(),
                1
            ),
            (
                "auth_key_cache_requests_total{source=test,result=fresh}".to_owned(),
                1
            ),
        ]
    );
}

use std::sync::atomic::AtomicBool;

use proptest::prelude::*;
use tokio::sync::Semaphore;

use super::*;

const TTL: Duration = Duration::from_secs(300);
const COOLDOWN: Duration = Duration::from_secs(30);
const GRACE: Duration = Duration::from_secs(600);
const KNOWN: &str = "known";

struct FakeSource {
    set: JwkSet,
    fetches: AtomicU64,
    down: AtomicBool,
    gate: Option<Arc<Semaphore>>,
}

impl FakeSource {
    fn new() -> Self {
        Self {
            set: key_set(&[KNOWN]),
            fetches: AtomicU64::new(0),
            down: AtomicBool::new(false),
            gate: None,
        }
    }

    fn fetches(&self) -> u64 {
        self.fetches.load(Ordering::SeqCst)
    }
}

impl FetchKeys for FakeSource {
    async fn fetch(&self) -> Result<JwkSet, VerificationError> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            let _permit = gate
                .acquire()
                .await
                .map_err(|_| VerificationError::Unavailable)?;
        }
        if self.down.load(Ordering::SeqCst) {
            return Err(VerificationError::Unavailable);
        }
        Ok(self.set.clone())
    }
}

fn key_set(kids: &[&str]) -> JwkSet {
    let keys = kids
        .iter()
        .map(|kid| serde_json::json!({"kty": "RSA", "kid": kid, "n": "AQAB", "e": "AQAB"}))
        .collect::<Vec<_>>();
    serde_json::from_value(serde_json::json!({ "keys": keys })).expect("key set")
}

fn cache() -> JwksCache {
    JwksCache::new(JwksPolicy {
        ttl: TTL,
        cooldown: COOLDOWN,
        stale_grace: GRACE,
    })
}

fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[tokio::test]
async fn thousand_unknown_kids_within_the_cooldown_fetch_at_most_once() {
    let source = FakeSource::new();
    let cache = cache();
    cache.key_for(&source, KNOWN, at(0)).await.expect("prime");
    for index in 0..1000_u64 {
        let result = cache
            .key_for(&source, &format!("random-{index}"), at(40 + index % 20))
            .await;
        assert!(matches!(result, Err(VerificationError::UnknownKeyId)));
    }
    assert_eq!(source.fetches(), 2);
}

#[tokio::test]
async fn known_kid_verifies_while_an_unknown_kid_fetch_is_in_flight() {
    let gate = Arc::new(Semaphore::new(0));
    let source = Arc::new(FakeSource {
        gate: Some(Arc::clone(&gate)),
        ..FakeSource::new()
    });
    let cache = Arc::new(cache());
    gate.add_permits(1);
    cache.key_for(&*source, KNOWN, at(0)).await.expect("prime");
    let pending = {
        let (source, cache) = (Arc::clone(&source), Arc::clone(&cache));
        tokio::spawn(async move { cache.key_for(&*source, "rotated", at(60)).await })
    };
    while source.fetches() < 2 {
        tokio::task::yield_now().await;
    }
    let known = tokio::time::timeout(
        Duration::from_secs(1),
        cache.key_for(&*source, KNOWN, at(61)),
    )
    .await;
    assert!(matches!(known, Ok(Ok(_))), "known kid waited on the fetch");
    gate.add_permits(1);
    assert!(matches!(
        pending.await.expect("task"),
        Err(VerificationError::UnknownKeyId)
    ));
}

#[tokio::test]
async fn known_kid_survives_an_outage_within_the_grace_only() {
    let source = FakeSource::new();
    let cache = cache();
    cache.key_for(&source, KNOWN, at(0)).await.expect("prime");
    source.down.store(true, Ordering::SeqCst);
    let stale = TTL + at(1);
    assert!(cache.key_for(&source, KNOWN, stale).await.is_ok());
    assert!(cache.key_for(&source, KNOWN, stale + at(1)).await.is_ok());
    assert_eq!(source.fetches(), 2, "stale refreshes respect the cooldown");
    let expired = TTL + GRACE;
    assert!(cache.key_for(&source, KNOWN, expired).await.is_err());
}

#[tokio::test]
async fn refresh_after_the_ttl_replaces_the_set() {
    let source = FakeSource::new();
    let cache = cache();
    cache.key_for(&source, KNOWN, at(0)).await.expect("prime");
    cache.key_for(&source, KNOWN, TTL).await.expect("refresh");
    assert_eq!(source.fetches(), 2);
    cache
        .key_for(&source, KNOWN, TTL + at(1))
        .await
        .expect("fresh");
    assert_eq!(source.fetches(), 2);
}

#[test]
fn one_unparseable_key_does_not_drop_the_set() {
    let set: JwkSet = serde_json::from_value(serde_json::json!({
        "keys": [{"kid": "no-kty"}, {"kty": "RSA", "kid": "good", "n": "AQAB", "e": "AQAB"}, 7]
    }))
    .expect("tolerant parse");
    assert!(set.find("good").is_some());
    assert!(set.find("no-kty").is_none());
    assert!(serde_json::from_str::<JwkSet>(r#"{"keys": 1}"#).is_err());
}

#[derive(Clone, Debug)]
enum Step {
    Lookup { kid: u16, advance: u64 },
    Outage(bool),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        8 => (any::<u16>(), 0..20_u64).prop_map(|(kid, advance)| Step::Lookup { kid, advance }),
        1 => any::<bool>().prop_map(Step::Outage),
    ]
}

proptest! {
    /// P-03: fetches per cooldown window stay bounded for any kid stream, and
    /// the known kid verifies throughout cooldowns and outages within the grace.
    #[test]
    fn fetches_are_bounded_and_known_kids_keep_verifying(steps in prop::collection::vec(step(), 1..200)) {
        let runtime = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        runtime.block_on(async {
            let source = FakeSource::new();
            let cache = cache();
            let mut now = Duration::ZERO;
            cache.key_for(&source, KNOWN, now).await.expect("prime");
            let mut last_success = now;
            for step in steps {
                match step {
                    Step::Outage(down) => source.down.store(down, Ordering::SeqCst),
                    Step::Lookup { kid, advance } => {
                        now += Duration::from_secs(advance);
                        let kid = if kid % 4 == 0 { KNOWN.to_owned() } else { format!("k{kid}") };
                        let before = source.fetches();
                        let result = cache.key_for(&source, &kid, now).await;
                        if source.fetches() > before && !source.down.load(Ordering::SeqCst) {
                            last_success = now;
                        }
                        if kid == KNOWN && now < last_success + TTL + GRACE {
                            prop_assert!(result.is_ok(), "known kid failed at {now:?}");
                        }
                    }
                }
            }
            let windows = now.as_secs() / COOLDOWN.as_secs() + 1;
            prop_assert!(source.fetches() <= 1 + windows, "{} fetches over {windows} windows", source.fetches());
            Ok(())
        })?;
    }
}

#[tokio::test]
async fn expired_set_during_an_outage_refetches_once_per_cooldown() {
    let source = FakeSource::new();
    let cache = cache();
    cache
        .key_for(&source, KNOWN, Duration::ZERO)
        .await
        .expect("prime");
    source.down.store(true, Ordering::SeqCst);
    let expired = TTL + GRACE + Duration::from_secs(1);
    for offset in 0..5 {
        let now = expired + Duration::from_secs(offset);
        assert!(cache.key_for(&source, KNOWN, now).await.is_err());
    }
    assert_eq!(source.fetches(), 2);
}

#[tokio::test]
async fn a_ttl_shorter_than_the_cooldown_still_refreshes_on_expiry() {
    let config = crate::OidcConfig::new("https://issuer.example", "api")
        .expect("config")
        .with_jwks_cache_ttl(Duration::from_secs(1))
        .expect("ttl");
    let source = FakeSource::new();
    let cache = JwksCache::new(JwksPolicy::from_config(&config));
    cache.key_for(&source, KNOWN, at(0)).await.expect("prime");
    cache.key_for(&source, KNOWN, at(2)).await.expect("stale");
    assert_eq!(source.fetches(), 2);
}

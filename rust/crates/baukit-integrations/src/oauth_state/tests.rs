use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use proptest::prelude::*;

use super::*;

fn service() -> OAuthStateService {
    OAuthStateService::new(Arc::new(InMemoryOAuthStateStore::default()))
}

fn input(pkce: Pkce) -> StartOAuthFlow {
    StartOAuthFlow {
        user: "user-1".into(),
        provider: "github".into(),
        redirect: "/settings/integrations".into(),
        pkce,
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(future)
}

#[tokio::test]
async fn completes_once_with_user_provider_and_verifier() {
    let service = service();
    let started = service.start(input(Pkce::S256)).await.expect("start");
    assert_eq!(started.state.expose().len(), TOKEN_CHARS);
    assert_eq!(started.max_age, DEFAULT_OAUTH_STATE_TTL);

    let done = service
        .complete(&started.state, &started.binding)
        .await
        .expect("complete");
    assert_eq!(done.user, "user-1");
    assert_eq!(done.provider, "github");
    assert_eq!(done.redirect, "/settings/integrations");
    let verifier = done.pkce_verifier.expect("verifier");
    assert_eq!(Some(verifier.challenge()), started.pkce_challenge);
}

#[tokio::test]
async fn pkce_is_skipped_for_providers_without_it() {
    let service = service();
    let started = service
        .start(input(Pkce::Unsupported))
        .await
        .expect("start");
    assert!(started.pkce_challenge.is_none());
    let done = service
        .complete(&started.state, &started.binding)
        .await
        .expect("complete");
    assert!(done.pkce_verifier.is_none());
}

#[tokio::test]
async fn second_complete_is_already_used() {
    let service = service();
    let started = service.start(input(Pkce::S256)).await.expect("start");
    service
        .complete(&started.state, &started.binding)
        .await
        .expect("first");
    let error = service
        .complete(&started.state, &started.binding)
        .await
        .expect_err("second");
    assert!(matches!(error, OAuthStateError::AlreadyUsed));
}

#[tokio::test]
async fn unknown_state_is_unknown() {
    let service = service();
    let started = service.start(input(Pkce::S256)).await.expect("start");
    let forged = OAuthState::parse(&"A".repeat(TOKEN_CHARS)).expect("token");
    let error = service
        .complete(&forged, &started.binding)
        .await
        .expect_err("forged");
    assert!(matches!(error, OAuthStateError::Unknown));
}

#[tokio::test]
async fn expired_state_is_expired_and_burned() {
    let service = service();
    let now = Utc::now();
    let started = service
        .start_at(input(Pkce::S256), now)
        .await
        .expect("start");
    let later = now + ChronoDuration::seconds(601);
    let error = service
        .complete_at(&started.state, &started.binding, later)
        .await
        .expect_err("expired");
    assert!(matches!(error, OAuthStateError::Expired));
    let again = service
        .complete_at(&started.state, &started.binding, now)
        .await
        .expect_err("burned");
    assert!(matches!(again, OAuthStateError::AlreadyUsed));
}

#[tokio::test]
async fn another_browser_is_rejected_and_burns_the_state() {
    let service = service();
    let attacker = service.start(input(Pkce::S256)).await.expect("start");
    let victim = service.start(input(Pkce::S256)).await.expect("start");
    let error = service
        .complete(&attacker.state, &victim.binding)
        .await
        .expect_err("mismatch");
    assert!(matches!(error, OAuthStateError::BrowserMismatch));
    let retry = service
        .complete(&attacker.state, &attacker.binding)
        .await
        .expect_err("burned");
    assert!(matches!(retry, OAuthStateError::AlreadyUsed));
}

#[tokio::test]
async fn purge_drops_expired_pending_and_used_records() {
    let store = Arc::new(InMemoryOAuthStateStore::default());
    let service = OAuthStateService::new(store.clone());
    let now = Utc::now();
    let used = service.start_at(input(Pkce::S256), now).await.expect("a");
    service
        .complete_at(&used.state, &used.binding, now)
        .await
        .expect("complete");
    service.start_at(input(Pkce::S256), now).await.expect("b");
    let removed = store
        .purge_expired(now + ChronoDuration::seconds(601))
        .await
        .expect("purge");
    assert_eq!(removed, 2);
}

#[test]
fn parse_rejects_wrong_length_and_alphabet() {
    for raw in [
        "",
        "short",
        &"A".repeat(44),
        &format!("{}+", "A".repeat(42)),
    ] {
        assert!(matches!(
            OAuthState::parse(raw),
            Err(OAuthStateError::Malformed)
        ));
        assert!(matches!(
            BrowserBinding::parse(raw),
            Err(OAuthStateError::Malformed)
        ));
    }
}

#[test]
fn cookie_header_lookup_finds_only_the_binding() {
    let header = format!("a=1; {OAUTH_BINDING_COOKIE}=xyz; b=2");
    assert_eq!(binding_from_cookie_header(&header), Some("xyz"));
    assert_eq!(binding_from_cookie_header("baukit-oauth=xyz"), None);
}

#[test]
fn debug_output_redacts_secrets() {
    let flow = block_on(service().start(input(Pkce::S256))).expect("start");
    let printed = format!("{flow:?}");
    assert!(!printed.contains(flow.state.expose()));
    assert!(!printed.contains(flow.binding.expose()));
    let verifier = PkceVerifier::from_stored("verifier-secret".into());
    assert!(!format!("{verifier:?}").contains("verifier-secret"));
    let completed = CompletedOAuthFlow {
        user: "u".into(),
        provider: "p".into(),
        redirect: "/".into(),
        pkce_verifier: Some(verifier),
    };
    assert!(!format!("{completed:?}").contains("verifier-secret"));
}

fn mutate(token: &str, index: usize, replacement: char) -> String {
    token
        .char_indices()
        .map(|(position, original)| {
            if position == index % token.len() {
                replacement
            } else {
                original
            }
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn mutated_state_or_cookie_never_succeeds(
        index in 0_usize..TOKEN_CHARS,
        replacement in proptest::char::range('-', 'z'),
        mutate_state in any::<bool>(),
    ) {
        let service = service();
        let started = block_on(service.start(input(Pkce::S256))).expect("start");
        let (state_raw, binding_raw) = if mutate_state {
            (mutate(started.state.expose(), index, replacement), started.binding.expose().to_owned())
        } else {
            (started.state.expose().to_owned(), mutate(started.binding.expose(), index, replacement))
        };
        prop_assume!(state_raw != started.state.expose() || binding_raw != started.binding.expose());
        let outcome = OAuthState::parse(&state_raw).and_then(|state| {
            let binding = BrowserBinding::parse(&binding_raw)?;
            block_on(service.complete(&state, &binding))
        });
        prop_assert!(outcome.is_err());
    }
}

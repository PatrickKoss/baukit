#![cfg(feature = "delivery")]
use axum::{Router, http::StatusCode, routing::post};
use baukit_suite::adapters::peer::ReqwestSuitePeerClient;
use baukit_suite::domain::*;
use baukit_suite::ports::{SuitePeerClient, SuitePeerError, SuiteSignedCall};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The default egress breaker opens after this many failed calls.
const BREAKER_MIN_CALLS: usize = 10;
const ATTEMPTS_PER_LINK: usize = 20;

async fn peer(status: StatusCode) -> Result<(ActivePeer, Arc<AtomicUsize>), std::io::Error> {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let app = Router::new().route(
        "/suite/inbound/{id}",
        post(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async move { status }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    let peer = ActivePeer {
        metadata: PeerMetadata {
            id: "beta".into(),
            display_name: "Beta".into(),
            emits: Vec::new(),
            accepts: Vec::new(),
            reward_modes: Vec::new(),
            scheme: "beta".into(),
        },
        api_url: format!("http://{address}"),
        web_url: format!("http://{address}"),
        authorize_url: None,
        sends: Vec::new(),
        receives: Vec::new(),
    };
    Ok((peer, hits))
}

async fn deliver(
    client: &ReqwestSuitePeerClient,
    peer: &ActivePeer,
    link: Uuid,
) -> Result<baukit_suite::ports::SuitePeerResponse, SuitePeerError> {
    client
        .deliver(
            peer,
            SuiteSignedCall {
                remote_link_id: link,
                delivery_id: "delivery",
                timestamp: 0,
                source_app: "alpha",
                secret: b"secret",
                body: b"{}",
                replay: false,
            },
        )
        .await
}

#[tokio::test]
async fn a_dead_peer_gets_at_most_the_breaker_threshold_across_links() -> TestResult {
    let client = ReqwestSuitePeerClient::new(true)?;
    let (dead, dead_hits) = peer(StatusCode::SERVICE_UNAVAILABLE).await?;
    let (healthy, healthy_hits) = peer(StatusCode::NO_CONTENT).await?;
    let dead_links = [Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()];
    let mut deferred = 0;
    for _ in 0..ATTEMPTS_PER_LINK {
        for link in dead_links {
            match deliver(&client, &dead, link).await {
                Err(SuitePeerError::Deferred { retry_after }) => {
                    assert!(retry_after > std::time::Duration::ZERO);
                    deferred += 1;
                }
                Ok(response) => assert_eq!(response.status, 503),
                Err(error) => return Err(error.into()),
            }
        }
        let response = deliver(&client, &healthy, Uuid::now_v7()).await?;
        assert_eq!(response.status, 204);
    }
    assert!(dead_hits.load(Ordering::SeqCst) <= BREAKER_MIN_CALLS);
    assert_eq!(
        deferred + dead_hits.load(Ordering::SeqCst),
        ATTEMPTS_PER_LINK * dead_links.len()
    );
    assert_eq!(healthy_hits.load(Ordering::SeqCst), ATTEMPTS_PER_LINK);
    assert!(matches!(
        map_delivery_result(DeliveryResult::Deferred {
            retry_after: std::time::Duration::from_secs(30)
        }),
        DeliveryAction::Deferred { .. }
    ));
    Ok(())
}

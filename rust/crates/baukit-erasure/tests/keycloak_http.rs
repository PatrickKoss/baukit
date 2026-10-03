#![cfg(feature = "keycloak")]

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use baukit_config::Secret;
use baukit_erasure::{
    IdentityAccountDeleter, IdentityDeletionError, KeycloakAccountDeleter, KeycloakDeletionConfig,
};
use serde_json::json;
use std::{
    collections::VecDeque,
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

#[derive(Default)]
struct Provider {
    token_statuses: Mutex<VecDeque<StatusCode>>,
    delete_statuses: Mutex<VecDeque<StatusCode>>,
    token_calls: AtomicUsize,
    delete_calls: AtomicUsize,
}

struct Server {
    base: String,
    task: JoinHandle<std::io::Result<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn token(State(provider): State<Arc<Provider>>) -> (StatusCode, Json<serde_json::Value>) {
    provider.token_calls.fetch_add(1, Ordering::SeqCst);
    let status = provider
        .token_statuses
        .lock()
        .await
        .pop_front()
        .unwrap_or(StatusCode::OK);
    (
        status,
        Json(json!({"access_token": "test-token", "expires_in": 3600})),
    )
}

async fn delete(State(provider): State<Arc<Provider>>) -> StatusCode {
    provider.delete_calls.fetch_add(1, Ordering::SeqCst);
    provider
        .delete_statuses
        .lock()
        .await
        .pop_front()
        .expect("scripted deletion response")
}

async fn server(provider: Arc<Provider>) -> Result<Server, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let router = Router::new()
        .route("/realms/test/protocol/openid-connect/token", post(token))
        .route(
            "/admin/realms/test/users/subject",
            axum::routing::delete(delete),
        )
        .with_state(provider);
    Ok(Server {
        base,
        task: tokio::spawn(async move { axum::serve(listener, router).await }),
    })
}

fn adapter(server: &Server) -> Result<KeycloakAccountDeleter, IdentityDeletionError> {
    KeycloakAccountDeleter::new(KeycloakDeletionConfig {
        base_url: server.base.clone(),
        realm: "test".into(),
        client_id: "backend".into(),
        client_secret: Secret::new("test-secret".into()),
        allow_local_http: true,
    })
}

#[tokio::test]
async fn deletion_classifies_provider_responses_and_caches_the_token() -> Result<(), Box<dyn Error>>
{
    let cases = [
        (StatusCode::NO_CONTENT, Ok(())),
        (StatusCode::NOT_FOUND, Ok(())),
        (
            StatusCode::BAD_REQUEST,
            Err(IdentityDeletionError::Permanent),
        ),
        (StatusCode::FORBIDDEN, Err(IdentityDeletionError::Permanent)),
        (
            StatusCode::TOO_MANY_REQUESTS,
            Err(IdentityDeletionError::Retryable),
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Err(IdentityDeletionError::Retryable),
        ),
        (StatusCode::FOUND, Err(IdentityDeletionError::Permanent)),
    ];
    let provider = Arc::new(Provider::default());
    provider
        .delete_statuses
        .lock()
        .await
        .extend(cases.iter().map(|(status, _)| *status));
    let server = server(provider.clone()).await?;
    let adapter = adapter(&server)?;
    for (_, expected) in cases {
        assert_eq!(adapter.delete_account("subject").await, expected);
    }
    assert_eq!(provider.token_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.delete_calls.load(Ordering::SeqCst), cases.len());
    Ok(())
}

#[tokio::test]
async fn unauthorized_deletion_refreshes_the_cached_token_on_the_next_attempt()
-> Result<(), Box<dyn Error>> {
    let provider = Arc::new(Provider::default());
    provider
        .delete_statuses
        .lock()
        .await
        .extend([StatusCode::UNAUTHORIZED, StatusCode::NO_CONTENT]);
    let server = server(provider.clone()).await?;
    let adapter = adapter(&server)?;
    assert_eq!(
        adapter.delete_account("subject").await,
        Err(IdentityDeletionError::Retryable)
    );
    adapter.delete_account("subject").await?;
    assert_eq!(provider.token_calls.load(Ordering::SeqCst), 2);
    assert_eq!(provider.delete_calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn failed_token_grant_is_retryable_and_does_not_delete() -> Result<(), Box<dyn Error>> {
    let provider = Arc::new(Provider::default());
    provider
        .token_statuses
        .lock()
        .await
        .push_back(StatusCode::BAD_REQUEST);
    provider
        .delete_statuses
        .lock()
        .await
        .push_back(StatusCode::NO_CONTENT);
    let server = server(provider.clone()).await?;
    let adapter = adapter(&server)?;
    assert_eq!(
        adapter.delete_account("subject").await,
        Err(IdentityDeletionError::Retryable)
    );
    assert_eq!(provider.delete_calls.load(Ordering::SeqCst), 0);
    adapter.delete_account("subject").await?;
    assert_eq!(provider.token_calls.load(Ordering::SeqCst), 2);
    assert_eq!(provider.delete_calls.load(Ordering::SeqCst), 1);
    Ok(())
}

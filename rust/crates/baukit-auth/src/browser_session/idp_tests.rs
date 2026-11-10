use axum::{Json, Router, extract::State, routing::get};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_credential_vault::CredentialCipher;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::{BrowserSessionConfig, IdpDiscoveryError, idp::OidcTokenClient};

type TestError = Box<dyn std::error::Error + Send + Sync>;

const PUBLIC_ISSUER: &str = "http://localhost:18081/realms/test";

fn config(internal_origin: Option<&str>) -> Result<BrowserSessionConfig, TestError> {
    let cipher = CredentialCipher::parse(&format!("1:{}", STANDARD.encode([9_u8; 32])))?;
    let config = BrowserSessionConfig::new(
        PUBLIC_ISSUER,
        "web",
        "acme-session",
        "http://localhost:8080",
        cipher,
    )?
    .with_insecure_local_cookie(true)
    .with_redirect_uris(["http://localhost:5173/"]);
    match internal_origin {
        Some(origin) => Ok(config.with_internal_issuer(&format!("{origin}/realms/test"))?),
        None => Ok(config),
    }
}

async fn serve_discovery(issuer: Option<&str>) -> Result<String, TestError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let document = json!({
        "issuer": issuer.map_or_else(|| format!("{origin}/realms/test"), str::to_owned),
        "jwks_uri": format!("{origin}/realms/test/protocol/openid-connect/certs"),
        "token_endpoint": format!("{origin}/realms/test/protocol/openid-connect/token"),
    });
    let router = Router::new()
        .route(
            "/realms/test/.well-known/openid-configuration",
            get(|State(document): State<Value>| async move { Json(document) }),
        )
        .with_state(document);
    tokio::spawn(async move { axum::serve(listener, router).await });
    Ok(origin)
}

#[tokio::test]
async fn discovery_uses_the_internal_issuer_and_checks_the_public_name() -> Result<(), TestError> {
    let origin = serve_discovery(Some(PUBLIC_ISSUER)).await?;
    let client = OidcTokenClient::discover(&config(Some(&origin))?).await;
    assert!(client.is_ok());
    Ok(())
}

#[tokio::test]
async fn discovery_rejects_a_document_that_names_the_internal_issuer() -> Result<(), TestError> {
    let origin = serve_discovery(None).await?;
    let outcome = OidcTokenClient::discover(&config(Some(&origin))?).await;
    assert!(matches!(outcome, Err(IdpDiscoveryError::IssuerMismatch)));
    Ok(())
}

#[tokio::test]
async fn public_issuer_that_is_unreachable_fails_without_an_internal_issuer()
-> Result<(), TestError> {
    let outcome = OidcTokenClient::discover(&config(None)?).await;
    assert!(matches!(outcome, Err(IdpDiscoveryError::Request(_))));
    Ok(())
}

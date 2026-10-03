use std::{error::Error, time::Duration};

use baukit_auth::{OidcConfig, OidcVerifier, PrincipalClaimMapping};
use baukit_test::MockOidcServer;
use reqwest::{Client, Response, StatusCode};
use serde_json::Value;

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const REDIRECT: &str = "https://app.example/callback";

fn client() -> Result<Client, reqwest::Error> {
    Client::builder().timeout(Duration::from_secs(5)).build()
}

fn endpoint(server: &MockOidcServer) -> String {
    format!("{}/protocol/openid-connect/token", server.issuer())
}

async fn exchange(
    server: &MockOidcServer,
    code: &str,
    client_id: &str,
    redirect: &str,
    verifier: &str,
) -> Result<Response, Box<dyn Error>> {
    let body = serde_html_form::to_string([
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("redirect_uri", redirect),
        ("code_verifier", verifier),
    ])?;
    Ok(client()?
        .post(endpoint(server))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?)
}

async fn assert_error(
    response: Response,
    status: StatusCode,
    error: &str,
) -> Result<(), Box<dyn Error>> {
    assert_eq!(response.status(), status);
    let body: Value = response.json().await?;
    assert_eq!(body["error"], error);
    assert!(body.get("access_token").is_none());
    assert!(body.get("refresh_token").is_none());
    Ok(())
}

#[tokio::test]
async fn authorization_code_uses_s256_and_issues_a_refreshable_session()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let code = server.issue_authorization_code("user", "web", REDIRECT, CHALLENGE)?;
    let response = exchange(&server, &code, "web", REDIRECT, VERIFIER).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 300);
    let access_token = body["access_token"]
        .as_str()
        .ok_or("missing access token")?;
    let verifier = OidcVerifier::discover(OidcConfig::new(server.issuer(), "web")?).await?;
    assert_eq!(verifier.verify(access_token).await?.subject(), "user");
    let refresh = body["refresh_token"]
        .as_str()
        .ok_or("missing refresh token")?;
    let renewed = server.refresh_session(refresh).await?;
    assert_eq!(
        verifier.verify(renewed.access_token()).await?.subject(),
        "user"
    );
    assert_error(
        exchange(&server, &code, "web", REDIRECT, VERIFIER).await?,
        StatusCode::BAD_REQUEST,
        "invalid_grant",
    )
    .await?;
    Ok(())
}

#[tokio::test]
async fn code_binding_and_verifier_failures_are_rejected() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    for (client_id, redirect, verifier) in [
        ("attacker", REDIRECT, VERIFIER),
        ("web", "https://attacker.example/callback", VERIFIER),
        (
            "web",
            REDIRECT,
            "wrong-proof-with-the-same-length-as-the-verifier",
        ),
        ("web", REDIRECT, "short"),
        (
            "web",
            REDIRECT,
            "invalid-verifier-with-space-abcdefghijklmnopq ",
        ),
    ] {
        let code = server.issue_authorization_code("user", "web", REDIRECT, CHALLENGE)?;
        assert_error(
            exchange(&server, &code, client_id, redirect, verifier).await?,
            StatusCode::BAD_REQUEST,
            "invalid_grant",
        )
        .await?;
    }
    assert_error(
        exchange(&server, "unknown", "web", REDIRECT, VERIFIER).await?,
        StatusCode::BAD_REQUEST,
        "invalid_grant",
    )
    .await?;
    assert_error(
        client()?
            .post(endpoint(&server))
            .body("grant_type=authorization_code&client_id=web")
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await?;
    assert!(
        server
            .issue_authorization_code("user", "web", REDIRECT, "plain-challenge")
            .is_err()
    );
    assert!(
        server
            .issue_authorization_code("user", "web", REDIRECT, format!("{CHALLENGE}="))
            .is_err()
    );
    assert!(
        server
            .issue_authorization_code("user", "web", "invalid-url", CHALLENGE)
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_code_exchanges_succeed_only_once() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let code = server.issue_authorization_code("user", "web", REDIRECT, CHALLENGE)?;
    let (first, second) = tokio::join!(
        exchange(&server, &code, "web", REDIRECT, VERIFIER),
        exchange(&server, &code, "web", REDIRECT, VERIFIER)
    );
    let mut statuses = [first?.status().as_u16(), second?.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 400]);
    Ok(())
}

#[tokio::test]
async fn client_credentials_support_form_and_basic_authentication() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    server.register_client_credentials("worker", "fixture-secret", "api")?;
    let verifier = OidcVerifier::discover(
        OidcConfig::new(server.issuer(), "api")?
            .with_client_id("worker")?
            .with_principal_claims(PrincipalClaimMapping::new().client_id_claim("client_id")),
    )
    .await?;
    let form = client()?
        .post(endpoint(&server))
        .body("grant_type=client_credentials&client_id=worker&client_secret=fixture-secret")
        .send()
        .await?;
    let basic = client()?
        .post(endpoint(&server))
        .basic_auth("worker", Some("fixture-secret"))
        .body("grant_type=client_credentials")
        .send()
        .await?;
    for response in [form, basic] {
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await?;
        assert_eq!(body["expires_in"], 300);
        assert_eq!(body["token_type"], "Bearer");
        assert!(body.get("refresh_token").is_none());
        let principal = verifier
            .verify(
                body["access_token"]
                    .as_str()
                    .ok_or("missing access token")?,
            )
            .await?;
        assert_eq!(principal.subject(), "worker");
        assert_eq!(principal.client_id(), Some("worker"));
    }
    assert_eq!(server.refresh_request_count(), 0);
    Ok(())
}

#[tokio::test]
async fn client_credentials_reject_missing_unknown_and_incorrect_credentials()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    server.register_client_credentials("worker", "fixture-secret", "api")?;
    for body in [
        "grant_type=client_credentials",
        "grant_type=client_credentials&client_id=worker",
        "grant_type=client_credentials&client_id=unknown&client_secret=fixture-secret",
        "grant_type=client_credentials&client_id=worker&client_secret=fixture-secreu",
        "grant_type=client_credentials&client_id=worker&client_secret=",
    ] {
        assert_error(
            client()?.post(endpoint(&server)).body(body).send().await?,
            StatusCode::UNAUTHORIZED,
            "invalid_client",
        )
        .await?;
    }
    for authorization in ["Basic !!!", "Basic d29ya2Vy", "Bearer fixture-secret"] {
        let response = client()?
            .post(endpoint(&server))
            .header("authorization", authorization)
            .body("grant_type=client_credentials")
            .send()
            .await?;
        assert_eq!(
            response.headers()["www-authenticate"],
            "Basic realm=\"baukit-test\""
        );
        assert_error(response, StatusCode::UNAUTHORIZED, "invalid_client").await?;
    }
    let duplicate = client()?
        .post(endpoint(&server))
        .basic_auth("worker", Some("fixture-secret"))
        .body("grant_type=client_credentials&client_id=worker&client_secret=fixture-secret")
        .send()
        .await?;
    assert_error(duplicate, StatusCode::UNAUTHORIZED, "invalid_client").await?;
    assert_error(
        client()?
            .post(endpoint(&server))
            .body("grant_type=password")
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "unsupported_grant_type",
    )
    .await?;
    assert_error(
        client()?
            .post(endpoint(&server))
            .body("client_id=worker")
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "invalid_request",
    )
    .await?;
    assert!(
        server
            .register_client_credentials("worker", "", "api")
            .is_err()
    );
    Ok(())
}

//! Response headers kept on status errors, shown with a registry token flow.

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

use axum::http::{HeaderMap, HeaderValue, StatusCode, header::AUTHORIZATION};
use baukit_egress::{
    AddressPolicy, EgressError, EgressOptions, EgressOptionsError, EgressRequest, GuardedClient,
    MAX_ERROR_HEADER_VALUE_BYTES, StaticResolver,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const HOST: &str = "registry.test";
const TOKEN: &str = "registry-token";

fn registry_reply(request: &str, port: u16) -> String {
    let first_line = request.lines().next().unwrap_or_default();
    if first_line.starts_with("GET /token") {
        let body = format!(r#"{{"token":"{TOKEN}"}}"#);
        return format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
    }
    if request.contains(&format!("Bearer {TOKEN}")) {
        return "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned();
    }
    format!(
        "HTTP/1.1 401 Unauthorized\r\nwww-authenticate: Bearer realm=\"http://{HOST}:{port}/token\",service=\"registry.test\",scope=\"repository:app:pull\"\r\nx-secret: hidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    )
}

async fn serve_registry() -> TestResult<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buffer = [0_u8; 4096];
            let Ok(read) = stream.read(&mut buffer).await else {
                continue;
            };
            let request = String::from_utf8_lossy(buffer.get(..read).unwrap_or_default());
            let reply = registry_reply(&request, port);
            let _ = stream.write_all(reply.as_bytes()).await;
        }
    });
    Ok(port)
}

fn client(options: EgressOptions) -> TestResult<GuardedClient> {
    let resolver =
        Arc::new(StaticResolver::new().with_host(HOST, [IpAddr::V4(Ipv4Addr::LOCALHOST)]));
    Ok(GuardedClient::with_resolver(resolver, options)?)
}

fn options() -> EgressOptions {
    EgressOptions::default().with_policy(AddressPolicy::AllowLoopback)
}

fn challenge_param(challenge: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=\"");
    let start = challenge.find(&marker)? + marker.len();
    let rest = challenge.get(start..)?;
    Some(rest.get(..rest.find('"')?)?.to_owned())
}

async fn status_error(client: &GuardedClient, url: Url) -> TestResult<(StatusCode, HeaderMap)> {
    match client.execute(EgressRequest::get(url)).await {
        Err(EgressError::Status {
            status, headers, ..
        }) => Ok((status, *headers)),
        other => Err(format!("expected a status error, got {other:?}").into()),
    }
}

#[tokio::test]
async fn registry_challenge_drives_the_token_flow() -> TestResult {
    let port = serve_registry().await?;
    let client = client(options().with_error_headers(["www-authenticate"])?)?;
    let api = Url::parse(&format!("http://{HOST}:{port}/v2/"))?;

    let (status, headers) = status_error(&client, api.clone()).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.get("x-secret").is_none());
    let challenge = headers
        .get("www-authenticate")
        .ok_or("missing challenge")?
        .to_str()?;
    let realm = challenge_param(challenge, "realm").ok_or("no realm")?;
    let service = challenge_param(challenge, "service").ok_or("no service")?;
    let scope = challenge_param(challenge, "scope").ok_or("no scope")?;

    let token_url = Url::parse_with_params(&realm, [("service", service), ("scope", scope)])?;
    let granted = client.execute(EgressRequest::get(token_url)).await?;
    assert!(std::str::from_utf8(granted.body())?.contains(TOKEN));

    let mut auth = HeaderMap::new();
    auth.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {TOKEN}"))?,
    );
    let response = client
        .execute(EgressRequest::get(api).with_headers(auth))
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn headers_are_empty_unless_allowlisted() -> TestResult {
    let port = serve_registry().await?;
    let client = client(options())?;
    let (_, headers) =
        status_error(&client, Url::parse(&format!("http://{HOST}:{port}/v2/"))?).await?;
    assert!(headers.is_empty());
    Ok(())
}

#[tokio::test]
async fn oversized_header_values_are_dropped() -> TestResult {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let long = "a".repeat(MAX_ERROR_HEADER_VALUE_BYTES + 1);
            let reply = format!(
                "HTTP/1.1 401 Unauthorized\r\nwww-authenticate: {long}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            );
            let _ = stream.write_all(reply.as_bytes()).await;
        }
    });
    let client = client(options().with_error_headers(["www-authenticate"])?)?;
    let (_, headers) =
        status_error(&client, Url::parse(&format!("http://{HOST}:{port}/"))?).await?;
    assert!(headers.is_empty());
    Ok(())
}

#[test]
fn invalid_header_names_are_rejected() {
    assert_eq!(
        EgressOptions::default()
            .with_error_headers(["bad name"])
            .err(),
        Some(EgressOptionsError::InvalidErrorHeaderName)
    );
}

//! Vendor retry-after headers configured on [`EgressOptions`].

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use axum::http::StatusCode;
use baukit_egress::{
    AddressPolicy, EgressError, EgressOptions, EgressOptionsError, EgressRequest, GuardedClient,
    StaticResolver,
};
use baukit_http::RetryClass;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const HOST: &str = "vendor.test";
const FITBIT: &str = "fitbit-rate-limit-reset";
const WHOOP: &str = "x-ratelimit-reset";
const POLAR: &str = "ratelimit-reset";
const CAP: Duration = Duration::from_secs(60);

async fn serve_once(status: &'static str, header: (&'static str, &'static str)) -> TestResult<Url> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut buffer = [0_u8; 4096];
        if stream.read(&mut buffer).await.is_err() {
            return;
        }
        let reply = format!(
            "HTTP/1.1 {status}\r\n{}: {}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            header.0, header.1
        );
        let _ = stream.write_all(reply.as_bytes()).await;
    });
    Ok(Url::parse(&format!("http://{HOST}:{port}/limit"))?)
}

fn options() -> TestResult<EgressOptions> {
    Ok(EgressOptions::default()
        .with_policy(AddressPolicy::AllowLoopback)
        .with_max_retry_after(CAP)?)
}

async fn classify(
    options: EgressOptions,
    status: &'static str,
    header: (&'static str, &'static str),
) -> TestResult<(StatusCode, RetryClass)> {
    let url = serve_once(status, header).await?;
    let resolver =
        Arc::new(StaticResolver::new().with_host(HOST, [IpAddr::V4(Ipv4Addr::LOCALHOST)]));
    let client = GuardedClient::with_resolver(resolver, options)?;
    match client.execute(EgressRequest::get(url)).await {
        Err(EgressError::Status { status, class, .. }) => Ok((status, class)),
        other => Err(format!("expected a status error, got {other:?}").into()),
    }
}

fn after(seconds: u64) -> RetryClass {
    RetryClass::RetryAfter(Duration::from_secs(seconds))
}

#[tokio::test]
async fn each_vendor_header_sets_the_delay_and_is_clamped() -> TestResult {
    let vendors = options()?.with_retry_after_headers([FITBIT, WHOOP, POLAR])?;
    let first_field = vendors.clone().with_retry_after_first_field();
    let cases = [
        (vendors.clone(), (FITBIT, "45"), after(45)),
        (vendors.clone(), (WHOOP, "12"), after(12)),
        (vendors.clone(), (POLAR, "30"), after(30)),
        (
            vendors.clone(),
            (FITBIT, "86400"),
            RetryClass::RetryAfter(CAP),
        ),
        (
            vendors.clone(),
            (POLAR, "30, 3600"),
            RetryClass::RateLimited,
        ),
        (first_field.clone(), (POLAR, "30, 3600"), after(30)),
        (
            first_field,
            (POLAR, "7200, 3600"),
            RetryClass::RetryAfter(CAP),
        ),
    ];
    for (options, header, expected) in cases {
        let (status, class) = classify(options, "429 Too Many Requests", header).await?;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(class, expected, "{header:?}");
    }
    Ok(())
}

#[tokio::test]
async fn unconfigured_vendor_headers_are_ignored() -> TestResult {
    let only_fitbit = options()?.with_retry_after_headers([FITBIT])?;
    for options in [options()?, only_fitbit] {
        let (_, class) = classify(options, "429 Too Many Requests", (POLAR, "30")).await?;
        assert_eq!(class, RetryClass::RateLimited);
    }
    Ok(())
}

#[tokio::test]
async fn vendor_headers_on_forbidden_need_the_forbidden_rate_limit_opt_in() -> TestResult {
    let vendors = options()?.with_retry_after_headers([WHOOP])?;
    let cases = [
        (vendors.clone(), RetryClass::Revoked),
        (vendors.with_forbidden_rate_limit(), after(20)),
        (options()?.with_forbidden_rate_limit(), RetryClass::Revoked),
    ];
    for (options, expected) in cases {
        let (status, class) = classify(options, "403 Forbidden", (WHOOP, "20")).await?;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(class, expected);
    }
    Ok(())
}

#[test]
fn invalid_header_names_are_refused() -> TestResult {
    for name in ["", "bad header", "ümlaut"] {
        assert_eq!(
            options()?.with_retry_after_headers([name]),
            Err(EgressOptionsError::InvalidRetryHeaderName)
        );
    }
    let options = options()?
        .with_retry_after_headers([FITBIT])
        .and_then(|options| options.with_retry_after_headers([POLAR]))?;
    let names: Vec<&str> = options
        .retry_after_headers()
        .iter()
        .map(|name| name.as_str())
        .collect();
    assert_eq!(names, [FITBIT, POLAR]);
    assert!(!options.retry_after_first_field());
    Ok(())
}

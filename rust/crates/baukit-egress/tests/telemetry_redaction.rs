//! Checks that errors, request debug output, and telemetry never carry URL
//! secrets. Runs as its own test binary so its subscriber sees every callsite.

use std::{
    io,
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
};

use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
use baukit_egress::{AddressPolicy, EgressOptions, EgressRequest, GuardedClient, StaticResolver};
use tokio::{io::AsyncWriteExt, net::TcpListener};
use tracing_subscriber::fmt::format::FmtSpan;
use url::Url;

const SECRETS: [&str; 4] = ["url-secret", "path-secret", "query-secret", "header-secret"];
const SECRET_PATH: &str = "/hook/path-secret?token=query-secret";
const FAILURE: &[u8] =
    b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("log buffer should not be poisoned"))
            .into_owned()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("log buffer should not be poisoned")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn assert_no_secret(text: &str) {
    for secret in SECRETS {
        assert!(!text.contains(secret), "{secret} leaked into: {text}");
    }
}

async fn failing_server() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("local server should bind");
    let port = listener
        .local_addr()
        .expect("local server should report its address")
        .port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let _ = stream.write_all(FAILURE).await;
            let _ = stream.shutdown().await;
        }
    });
    port
}

async fn closed_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener should bind")
        .local_addr()
        .expect("listener should report its address")
        .port()
}

fn secret_url(authority: &str) -> Url {
    Url::parse(&format!("http://{authority}{SECRET_PATH}")).expect("test URL should parse")
}

#[tokio::test]
async fn errors_and_telemetry_do_not_leak_url_secrets() {
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::CLOSE)
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let resolver = StaticResolver::new().with_host("hooks.test", [IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    let options = EgressOptions::default().with_policy(AddressPolicy::AllowLoopback);
    let client = GuardedClient::with_resolver(Arc::new(resolver), options)
        .expect("guarded client should build");
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer header-secret"),
    );
    let urls = [
        secret_url(&format!("hooks.test:{}", failing_server().await)),
        secret_url(&format!("hooks.test:{}", closed_port().await)),
        secret_url("user:url-secret@hooks.test"),
    ];

    for url in urls {
        let request = EgressRequest::post(url).with_headers(headers.clone());
        assert_no_secret(&format!("{request:?}"));
        let error = client
            .execute(request)
            .await
            .expect_err("each request should fail");
        assert_no_secret(&error.to_string());
        assert_no_secret(&format!("{error:?}"));
    }

    let logs = logs.text();
    assert!(
        logs.contains("egress.request"),
        "spans were captured: {logs}"
    );
    assert!(
        logs.contains("egress request failed"),
        "failures were logged: {logs}"
    );
    assert!(logs.contains("hooks.test"), "the host is recorded: {logs}");
    assert_no_secret(&logs);
}

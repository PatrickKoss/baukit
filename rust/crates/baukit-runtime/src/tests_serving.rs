use std::{
    future::pending,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{Router, http::StatusCode, routing::get};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::Notify,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use super::{tests::http_get, *};

const SHORT_TIMEOUT: Duration = Duration::from_millis(500);
const LONG_TIMEOUT: Duration = Duration::from_secs(30);
const WAIT_LIMIT: Duration = Duration::from_secs(5);
const TIMER_SLACK: Duration = Duration::from_millis(50);

fn loopback() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
}

async fn spawn_server(router: Router, options: ServerOptions) -> (SocketAddr, CancellationToken) {
    let listener = TcpListener::bind(loopback()).await.expect("bind listener");
    let address = listener.local_addr().expect("listener address");
    let shutdown = CancellationToken::new();
    tokio::spawn(serve(listener, router, options, shutdown.clone()));
    (address, shutdown)
}

fn hello_router() -> Router {
    Router::new().route("/", get(|| async { "ok" }))
}

async fn read_until_closed(stream: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    tokio::time::timeout(WAIT_LIMIT, async {
        let mut buffer = [0_u8; 1024];
        loop {
            match stream.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => received.extend_from_slice(buffer.get(..read).unwrap_or_default()),
            }
        }
    })
    .await
    .expect("server should close the connection");
    received
}

#[tokio::test]
async fn slowloris_connection_closes_after_the_header_read_timeout() {
    let options = ServerOptions::default()
        .with_header_read_timeout(SHORT_TIMEOUT)
        .expect("valid timeout");
    let (address, _shutdown) = spawn_server(hello_router(), options).await;
    let mut stream = TcpStream::connect(address).await.expect("connect");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nX-Slow: ")
        .await
        .expect("write half a header");

    let started = Instant::now();
    let received = read_until_closed(&mut stream).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed + TIMER_SLACK >= SHORT_TIMEOUT,
        "closed after {elapsed:?}"
    );
    assert!(!String::from_utf8_lossy(&received).contains("200 OK"));
    assert!(http_get(address, "/").await.starts_with("HTTP/1.1 200 OK"));
}

#[tokio::test]
async fn idle_keep_alive_connection_closes_after_the_idle_timeout() {
    let options = ServerOptions::default()
        .with_header_read_timeout(LONG_TIMEOUT)
        .and_then(|options| options.with_idle_timeout(SHORT_TIMEOUT))
        .expect("valid timeouts");
    let (address, _shutdown) = spawn_server(hello_router(), options).await;
    let mut stream = TcpStream::connect(address).await.expect("connect");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .expect("write request");
    let mut response = [0_u8; 256];
    let read = stream.read(&mut response).await.expect("read response");
    assert!(
        response
            .get(..read)
            .is_some_and(|bytes| bytes.starts_with(b"HTTP/1.1 200 OK"))
    );

    let started = Instant::now();
    read_until_closed(&mut stream).await;
    let elapsed = started.elapsed();
    assert!(
        elapsed + TIMER_SLACK >= SHORT_TIMEOUT,
        "closed after {elapsed:?}"
    );
    assert!(elapsed < LONG_TIMEOUT);
}

#[tokio::test]
async fn connection_cap_holds_new_connections_until_one_closes() {
    let options = ServerOptions::default()
        .with_max_connections(1)
        .expect("valid cap");
    let (address, _shutdown) = spawn_server(hello_router(), options).await;
    let holder = TcpStream::connect(address).await.expect("first connection");
    tokio::time::sleep(TIMER_SLACK).await;

    let waiting = tokio::spawn(http_get(address, "/"));
    tokio::time::sleep(SHORT_TIMEOUT).await;
    assert!(
        !waiting.is_finished(),
        "second connection served past the cap"
    );

    drop(holder);
    let response = tokio::time::timeout(WAIT_LIMIT, waiting)
        .await
        .expect("second connection served after the first closed")
        .expect("request task joins");
    assert!(response.starts_with("HTTP/1.1 200 OK"));
}

#[test]
fn server_options_reject_zero_values() {
    let options = ServerOptions::default();
    assert_eq!(options.header_read_timeout(), Duration::from_secs(10));
    assert_eq!(
        options.with_header_read_timeout(Duration::ZERO),
        Err(ServerOptionsError::ZeroHeaderReadTimeout)
    );
    assert_eq!(
        options.with_idle_timeout(Duration::ZERO),
        Err(ServerOptionsError::ZeroIdleTimeout)
    );
    assert_eq!(
        options.with_max_connections(0),
        Err(ServerOptionsError::ZeroMaxConnections)
    );
}

#[test]
fn drain_budget_requires_room_for_one_request_and_the_flush() {
    let error = DrainBudget::new(Duration::from_secs(25), Duration::from_secs(30))
        .expect_err("a 30 s request cannot drain in 25 s");
    assert_eq!(error.request_timeout(), Duration::from_secs(30));
    assert!(error.to_string().contains("drain timeout"));
    assert!(DrainBudget::new(Duration::from_secs(33), Duration::from_secs(30)).is_err());

    let budget = DrainBudget::new(Duration::from_secs(35), Duration::from_secs(30))
        .expect("chart defaults fit");
    assert_eq!(budget.flush_reserve(), DEFAULT_FLUSH_RESERVE);
    assert_eq!(budget.drain_timeout(), Duration::from_secs(35));
}

struct DrainFixture {
    api_address: SocketAddr,
    operations_address: SocketAddr,
    shutdown: ShutdownToken,
    pair: tokio::task::JoinHandle<Result<(), ListenerPairError>>,
}

async fn start_pair(api_router: Router, budget: DrainBudget) -> DrainFixture {
    let api = TcpListener::bind(loopback()).await.expect("bind API");
    let operations = TcpListener::bind(loopback()).await.expect("bind ops");
    let api_address = api.local_addr().expect("API address");
    let operations_address = operations.local_addr().expect("ops address");
    let shutdown = ShutdownToken::with_budget(budget);
    let draining = Arc::new(AtomicBool::new(false));
    shutdown.on_drain({
        let draining = Arc::clone(&draining);
        move || draining.store(true, Ordering::SeqCst)
    });
    let operations_router = Router::new().route(
        "/readyz",
        get(move || {
            let draining = Arc::clone(&draining);
            async move {
                if draining.load(Ordering::SeqCst) {
                    (StatusCode::SERVICE_UNAVAILABLE, "draining")
                } else {
                    (StatusCode::OK, "ready")
                }
            }
        }),
    );
    let pair = tokio::spawn(serve_listener_pair_with_options(
        api,
        api_router,
        operations,
        operations_router,
        shutdown.clone(),
        ServeOptions {
            shutdown_order: ShutdownOrder::OpsOutlivesApi,
            ..ServeOptions::default()
        },
    ));
    DrainFixture {
        api_address,
        operations_address,
        shutdown,
        pair,
    }
}

fn slow_router(started: Arc<Notify>, duration: Duration) -> Router {
    Router::new().route(
        "/slow",
        get(move || {
            let started = Arc::clone(&started);
            async move {
                started.notify_one();
                tokio::time::sleep(duration).await;
                "finished"
            }
        }),
    )
}

#[tokio::test]
async fn in_flight_request_finishes_after_readiness_reports_draining_and_flush_runs() {
    const REQUEST: Duration = Duration::from_millis(1_500);
    let budget = DrainBudget::with_flush_reserve(
        Duration::from_secs(4),
        Duration::from_secs(2),
        Duration::from_secs(1),
    )
    .expect("budget fits");
    let started = Arc::new(Notify::new());
    let fixture = start_pair(slow_router(Arc::clone(&started), REQUEST), budget).await;
    let request = tokio::spawn(http_get(fixture.api_address, "/slow"));
    started.notified().await;

    fixture.shutdown.trigger();
    let readiness = http_get(fixture.operations_address, "/readyz").await;
    assert!(readiness.starts_with("HTTP/1.1 503"), "{readiness}");
    assert!(!request.is_finished());

    let response = request.await.expect("request task joins");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("finished"));
    fixture
        .pair
        .await
        .expect("listener task joins")
        .expect("listeners drain in time");

    let flushed = fixture
        .shutdown
        .run_flush(async { true })
        .await
        .expect("flush runs inside the deadline");
    assert!(flushed);
}

#[tokio::test]
async fn a_stuck_request_cannot_consume_the_flush_reserve() {
    const RESERVE: Duration = Duration::from_secs(1);
    const FLUSH: Duration = Duration::from_millis(500);
    let budget = DrainBudget::with_flush_reserve(
        Duration::from_secs(2),
        Duration::from_millis(500),
        RESERVE,
    )
    .expect("budget fits");
    let started = Arc::new(Notify::new());
    let stuck = Router::new().route(
        "/slow",
        get({
            let started = Arc::clone(&started);
            move || {
                let started = Arc::clone(&started);
                async move {
                    started.notify_one();
                    pending::<()>().await;
                }
            }
        }),
    );
    let fixture = start_pair(stuck, budget).await;
    let _request = tokio::spawn(http_get(fixture.api_address, "/slow"));
    started.notified().await;

    fixture.shutdown.trigger();
    let drained = fixture.pair.await.expect("listener task joins");
    assert!(matches!(drained, Err(ListenerPairError::Drain(_))));
    assert!(
        fixture
            .shutdown
            .run_during_drain(tokio::time::sleep(TIMER_SLACK))
            .await
            .is_err(),
        "drain work has no time left"
    );

    fixture
        .shutdown
        .run_flush(tokio::time::sleep(FLUSH))
        .await
        .expect("the flush still has its reserve");
}

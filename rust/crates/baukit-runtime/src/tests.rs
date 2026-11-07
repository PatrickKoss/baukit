use std::{
    future::pending,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::routing::get;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
    sync::Notify,
};

use super::*;

async fn http_get(address: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(address)
        .await
        .expect("connect to test listener");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write test request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read test response");
    String::from_utf8(response).expect("HTTP response is UTF-8")
}

#[tokio::test]
async fn token_cancels_children() {
    let shutdown = ShutdownToken::new(Duration::from_secs(5));
    let child = shutdown.child_token();
    assert!(!child.is_cancelled());

    assert!(shutdown.trigger());
    child.cancelled().await;

    assert!(child.is_cancelled());
    assert!(!shutdown.trigger());
}

#[tokio::test]
async fn drain_hooks_run_before_cancellation_and_late_hooks_run_immediately() {
    let shutdown = ShutdownToken::new(Duration::from_secs(5));
    let calls = Arc::new(AtomicUsize::new(0));
    shutdown.on_drain({
        let calls = Arc::clone(&calls);
        let shutdown = shutdown.clone();
        move || {
            assert!(!shutdown.is_cancelled());
            calls.fetch_add(1, Ordering::SeqCst);
        }
    });

    shutdown.trigger();
    assert!(shutdown.is_cancelled());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    shutdown.on_drain({
        let calls = Arc::clone(&calls);
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(!shutdown.trigger());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn drain_deadline_is_enforced() {
    let shutdown = ShutdownToken::new(Duration::from_secs(7));
    shutdown.trigger();
    let drain = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { shutdown.run_during_drain(pending::<()>()).await }
    });

    tokio::time::advance(Duration::from_secs(7)).await;
    let error = drain
        .await
        .expect("drain task should join")
        .expect_err("pending cleanup should exceed the deadline");
    assert_eq!(error.timeout(), Duration::from_secs(7));
}

#[tokio::test]
async fn supervisor_restarts_until_a_worker_stays_running() {
    let shutdown = ShutdownToken::new(Duration::from_secs(2));
    let worker_shutdown = shutdown.child_token();
    let attempts = Arc::new(AtomicUsize::new(0));
    let drained = Arc::new(AtomicUsize::new(0));
    let mut supervisor = TaskSupervisor::new(shutdown.clone());
    supervisor.spawn("restartable", RestartPolicy::Restart { max_restarts: 2 }, {
        let attempts = Arc::clone(&attempts);
        let drained = Arc::clone(&drained);
        move || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            let worker_shutdown = worker_shutdown.clone();
            let drained = Arc::clone(&drained);
            async move {
                if attempt < 2 {
                    return;
                }
                worker_shutdown.cancelled().await;
                drained.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        while attempts.load(Ordering::SeqCst) < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("worker should be restarted twice");

    shutdown.trigger();
    supervisor.join().await.expect("worker should drain");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(drained.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exhausted_restart_policy_triggers_process_shutdown() {
    let shutdown = ShutdownToken::new(Duration::from_secs(2));
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut supervisor = TaskSupervisor::new(shutdown.clone());
    supervisor.spawn("exhausted", RestartPolicy::Restart { max_restarts: 1 }, {
        let attempts = Arc::clone(&attempts);
        move || {
            attempts.fetch_add(1, Ordering::SeqCst);
            async {}
        }
    });

    tokio::time::timeout(Duration::from_secs(1), shutdown.cancelled())
        .await
        .expect("restart exhaustion should trigger shutdown");
    supervisor.join().await.expect("supervisor should join");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn supervisor_panic_triggers_process_shutdown() {
    let shutdown = ShutdownToken::new(Duration::from_secs(2));
    let mut supervisor = TaskSupervisor::new(shutdown.clone());
    supervisor.spawn("panicking", RestartPolicy::FailProcess, || async {
        panic!("intentional worker panic");
    });

    tokio::time::timeout(Duration::from_secs(1), shutdown.cancelled())
        .await
        .expect("panic should trigger shutdown");
    supervisor.join().await.expect("supervisor should join");
}

#[tokio::test]
async fn listener_pair_shuts_down_when_token_is_triggered() {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let api = TcpListener::bind(address).await.expect("bind API listener");
    let operations = TcpListener::bind(address)
        .await
        .expect("bind operations listener");
    let shutdown = ShutdownToken::new(Duration::from_secs(2));
    let pair = tokio::spawn(serve_listener_pair(
        api,
        Router::new(),
        operations,
        Router::new(),
        shutdown.clone(),
    ));

    tokio::task::yield_now().await;
    shutdown.trigger();

    tokio::time::timeout(Duration::from_secs(1), pair)
        .await
        .expect("listeners should finish promptly")
        .expect("listener task should join")
        .expect("listeners should drain successfully");
}

#[tokio::test]
async fn operations_listener_responds_while_api_is_draining() {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let api = TcpListener::bind(address).await.expect("bind API listener");
    let api_address = api.local_addr().expect("API address");
    let operations = TcpListener::bind(address)
        .await
        .expect("bind operations listener");
    let operations_address = operations.local_addr().expect("operations address");
    let request_started = Arc::new(Notify::new());
    let release_request = Arc::new(Notify::new());
    let api_router = Router::new().route(
        "/drain",
        get({
            let request_started = Arc::clone(&request_started);
            let release_request = Arc::clone(&release_request);
            move || {
                let request_started = Arc::clone(&request_started);
                let release_request = Arc::clone(&release_request);
                async move {
                    request_started.notify_one();
                    release_request.notified().await;
                    "drained"
                }
            }
        }),
    );
    let operations_router = Router::new().route("/readyz", get(|| async { "ready" }));
    let shutdown = ShutdownToken::new(Duration::from_secs(2));
    let pair = tokio::spawn(serve_listener_pair_with_shutdown_order(
        api,
        api_router,
        operations,
        operations_router,
        shutdown.clone(),
        ShutdownOrder::OpsOutlivesApi,
    ));
    let api_request = tokio::spawn(http_get(api_address, "/drain"));

    request_started.notified().await;
    shutdown.trigger();
    assert!(shutdown.is_cancelled());

    let operations_response = http_get(operations_address, "/readyz").await;
    assert!(operations_response.starts_with("HTTP/1.1 200 OK"));
    assert!(operations_response.ends_with("ready"));

    release_request.notify_one();
    let api_response = api_request.await.expect("API request task joins");
    assert!(api_response.starts_with("HTTP/1.1 200 OK"));
    tokio::time::timeout(Duration::from_secs(1), pair)
        .await
        .expect("listeners should finish after the API drains")
        .expect("listener task should join")
        .expect("listeners should drain successfully");
}

#[test]
fn service_info_reuses_telemetry_identity() {
    let build = BuildInfo::new("1.2.3", "abc123", "1.95.0");
    let service = ServiceInfo::new(
        "orders",
        ProcessKind::Worker,
        build,
        DeploymentEnvironment::Staging,
    );

    assert_eq!(service.name(), "orders-worker");
    assert_eq!(service.product(), "orders");
    assert_eq!(service.build().rust_version(), "1.95.0");
    assert_eq!(service.telemetry_identity().version(), "1.2.3");
}

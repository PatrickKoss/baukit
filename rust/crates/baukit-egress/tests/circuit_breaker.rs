#![cfg(test)]
//! Circuit breaker, bulkhead and deadline behaviour of [`GuardedClient`].

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use baukit_egress::{
    AddressPolicy, BreakerPolicy, EGRESS_CIRCUIT_STATE, EGRESS_CIRCUIT_TRANSITIONS_TOTAL,
    EGRESS_REQUEST_DURATION_SECONDS, EGRESS_REQUESTS_TOTAL, EgressError, EgressOptions,
    EgressRequest, GuardedClient, ResolveFuture, Resolver, StaticResolver,
};
use baukit_http::RetryClass;
use baukit_resilience::{Deadline, deadline};
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use url::Url;

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const OPEN_FOR: Duration = Duration::from_millis(300);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const MIN_CALLS: u32 = 10;
const CALLS: usize = 30;

#[derive(Clone)]
struct Behaviour {
    status: &'static str,
    headers: Vec<(&'static str, &'static str)>,
    delay: Duration,
}

impl Behaviour {
    fn status(status: &'static str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    fn with_header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.push((name, value));
        self
    }

    const fn after(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

struct ScriptedServer {
    port: u16,
    behaviour: Arc<Mutex<Behaviour>>,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Drop for ScriptedServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ScriptedServer {
    async fn start(behaviour: Behaviour) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("server binds");
        let port = listener.local_addr().expect("server address").port();
        let behaviour = Arc::new(Mutex::new(behaviour));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(accept_loop(
            listener,
            Arc::clone(&behaviour),
            Arc::clone(&requests),
        ));
        Self {
            port,
            behaviour,
            requests,
            task,
        }
    }

    fn set(&self, behaviour: Behaviour) {
        *self.behaviour.lock().expect("behaviour lock") = behaviour;
    }

    fn calls(&self) -> usize {
        self.requests.lock().expect("request log").len()
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("request log").clone()
    }

    fn request(&self, host: &str) -> EgressRequest {
        let url = Url::parse(&format!("http://{host}:{}/call?secret=1", self.port)).expect("url");
        EgressRequest::get(url)
    }
}

async fn accept_loop(
    listener: TcpListener,
    behaviour: Arc<Mutex<Behaviour>>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(answer(
            stream,
            Arc::clone(&behaviour),
            Arc::clone(&requests),
        ));
    }
}

async fn answer(
    mut stream: TcpStream,
    behaviour: Arc<Mutex<Behaviour>>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let request = read_head(&mut stream).await;
    requests.lock().expect("request log").push(request);
    let behaviour = behaviour.lock().expect("behaviour lock").clone();
    tokio::time::sleep(behaviour.delay).await;
    let mut text = format!(
        "HTTP/1.1 {}\r\ncontent-length: 0\r\nconnection: close\r\n",
        behaviour.status
    );
    for (name, value) in behaviour.headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("\r\n");
    let _ = stream.write_all(text.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 1024];
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..read]);
        if received.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&received).into_owned()
}

struct CountingResolver {
    inner: StaticResolver,
    lookups: AtomicUsize,
}

impl Resolver for CountingResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> ResolveFuture<'a> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve(host)
    }
}

fn counting_resolver(hosts: &[&str]) -> Arc<CountingResolver> {
    let inner = hosts.iter().fold(StaticResolver::new(), |resolver, host| {
        resolver.with_host(*host, [LOOPBACK])
    });
    Arc::new(CountingResolver {
        inner,
        lookups: AtomicUsize::new(0),
    })
}

fn fast_policy() -> BreakerPolicy {
    BreakerPolicy {
        open_base: OPEN_FOR,
        open_max: Duration::from_secs(10),
        jitter_percent: 0,
        probe_timeout: PROBE_TIMEOUT,
        ..BreakerPolicy::default()
    }
}

fn options() -> EgressOptions {
    EgressOptions::default()
        .with_policy(AddressPolicy::AllowLoopback)
        .with_circuit_breaker(fast_policy())
}

fn client(resolver: Arc<dyn Resolver>, options: EgressOptions) -> GuardedClient {
    GuardedClient::with_resolver("scripted", resolver, options).expect("client builds")
}

async fn call_many(client: &GuardedClient, server: &ScriptedServer, host: &str, calls: usize) {
    for _ in 0..calls {
        let _ = client.execute(server.request(host)).await;
    }
}

fn assert_circuit_open(error: &EgressError) {
    let EgressError::CircuitOpen { retry_after } = error else {
        panic!("expected an open circuit, got {error:?}");
    };
    assert!(*retry_after <= OPEN_FOR.max(PROBE_TIMEOUT).max(Duration::from_secs(2)));
    assert_eq!(error.retry_class(), RetryClass::RetryAfter(*retry_after));
    assert_eq!(error.code(), "circuit_open");
    let message = error.to_string();
    assert!(!message.contains("secret") && !message.contains("http"));
}

#[tokio::test]
async fn a_failing_destination_stops_receiving_calls_at_the_threshold() {
    let server = ScriptedServer::start(Behaviour::status("503 Service Unavailable")).await;
    let resolver = counting_resolver(&["down.test"]);
    let client = client(resolver.clone(), options());

    call_many(&client, &server, "down.test", CALLS).await;

    assert_eq!(server.calls(), usize::try_from(MIN_CALLS).expect("small"));
    let lookups = resolver.lookups.load(Ordering::SeqCst);
    let error = client
        .execute(server.request("down.test"))
        .await
        .expect_err("circuit open");
    assert_circuit_open(&error);
    assert_eq!(resolver.lookups.load(Ordering::SeqCst), lookups);
}

#[tokio::test]
async fn one_probe_closes_the_circuit_after_recovery() {
    let server = ScriptedServer::start(Behaviour::status("500 Internal Server Error")).await;
    let client = client(counting_resolver(&["flaky.test"]), options());
    call_many(&client, &server, "flaky.test", CALLS).await;
    assert_eq!(server.calls(), 10);

    server.set(Behaviour::status("204 No Content").after(Duration::from_millis(200)));
    tokio::time::sleep(OPEN_FOR + Duration::from_millis(50)).await;
    let probe = tokio::spawn({
        let client = client.clone();
        let request = server.request("flaky.test");
        async move { client.execute(request).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let during_probe = client
        .execute(server.request("flaky.test"))
        .await
        .expect_err("only one probe runs");
    assert_circuit_open(&during_probe);
    probe.await.expect("probe task").expect("probe succeeds");

    server.set(Behaviour::status("204 No Content"));
    call_many(&client, &server, "flaky.test", 5).await;
    assert_eq!(server.calls(), 16);
}

#[tokio::test]
async fn retry_after_holds_the_circuit_for_that_delay() {
    let server = ScriptedServer::start(
        Behaviour::status("429 Too Many Requests").with_header("retry-after", "2"),
    )
    .await;
    let client = client(counting_resolver(&["busy.test"]), options());

    let first = client
        .execute(server.request("busy.test"))
        .await
        .expect_err("rate limited");
    assert_eq!(
        first.retry_class(),
        RetryClass::RetryAfter(Duration::from_secs(2))
    );
    let held = client
        .execute(server.request("busy.test"))
        .await
        .expect_err("held");
    assert_circuit_open(&held);
    let EgressError::CircuitOpen { retry_after } = held else {
        return;
    };
    assert!(retry_after > Duration::from_millis(1500) && retry_after <= Duration::from_secs(2));
    assert_eq!(server.calls(), 1);

    server.set(Behaviour::status("200 OK"));
    tokio::time::sleep(retry_after + Duration::from_millis(50)).await;
    call_many(&client, &server, "busy.test", 3).await;
    assert_eq!(server.calls(), 4);
}

#[tokio::test]
async fn client_errors_never_open_the_circuit() {
    let server = ScriptedServer::start(Behaviour::status("400 Bad Request")).await;
    let client = client(counting_resolver(&["strict.test"]), options());
    call_many(&client, &server, "strict.test", CALLS).await;
    assert_eq!(server.calls(), CALLS);
}

#[tokio::test]
async fn destinations_on_one_client_have_independent_breakers() {
    let down = ScriptedServer::start(Behaviour::status("503 Service Unavailable")).await;
    let up = ScriptedServer::start(Behaviour::status("200 OK")).await;
    let client = client(counting_resolver(&["down.test", "up.test"]), options());

    call_many(&client, &down, "down.test", CALLS).await;
    call_many(&client, &up, "up.test", CALLS).await;

    assert_eq!(down.calls(), 10);
    assert_eq!(up.calls(), CALLS);
}

#[tokio::test]
async fn clones_share_breaker_state() {
    let server = ScriptedServer::start(Behaviour::status("502 Bad Gateway")).await;
    let original = client(counting_resolver(&["down.test"]), options());
    let clone = original.clone();
    call_many(&original, &server, "down.test", CALLS).await;
    let error = clone
        .execute(server.request("down.test"))
        .await
        .expect_err("shared open circuit");
    assert_circuit_open(&error);
    assert_eq!(server.calls(), 10);
}

#[tokio::test]
async fn without_a_breaker_every_call_is_sent() {
    let server = ScriptedServer::start(Behaviour::status("503 Service Unavailable")).await;
    let client = client(
        counting_resolver(&["down.test"]),
        options().without_circuit_breaker(),
    );
    call_many(&client, &server, "down.test", CALLS).await;
    assert_eq!(server.calls(), CALLS);
}

#[tokio::test]
async fn a_full_bulkhead_fails_fast_until_a_slot_frees() {
    let server =
        ScriptedServer::start(Behaviour::status("200 OK").after(Duration::from_millis(300))).await;
    let options = options().with_max_in_flight(1).expect("non-zero limit");
    let client = client(counting_resolver(&["slow.test", "other.test"]), options);

    let first = tokio::spawn({
        let client = client.clone();
        let request = server.request("slow.test");
        async move { client.execute(request).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let error = client
        .execute(server.request("slow.test"))
        .await
        .expect_err("saturated");
    assert!(matches!(error, EgressError::Saturated));
    assert_eq!(
        error.retry_class(),
        RetryClass::RetryAfter(Duration::from_secs(1))
    );
    assert!(!error.to_string().contains("slow.test"));
    client
        .execute(server.request("other.test"))
        .await
        .expect("another key has its own slots");

    first.await.expect("first task").expect("first call");
    client
        .execute(server.request("slow.test"))
        .await
        .expect("slot released");
    assert!(EgressOptions::default().with_max_in_flight(0).is_err());
}

#[tokio::test]
async fn a_request_deadline_caps_the_client_timeout_without_counting_a_failure() {
    let server =
        ScriptedServer::start(Behaviour::status("200 OK").after(Duration::from_secs(5))).await;
    let client = client(counting_resolver(&["slow.test"]), options());
    let started = Instant::now();
    let deadline = Deadline::after(started, Duration::from_secs(2)).expect("deadline");

    let result = deadline::scope(deadline, client.execute(server.request("slow.test"))).await;

    let elapsed = started.elapsed();
    assert!(matches!(result, Err(EgressError::Timeout)));
    assert!(elapsed >= Duration::from_millis(1900) && elapsed < Duration::from_millis(2500));
    let head = server.requests().concat().to_ascii_lowercase();
    assert!(!head.contains("deadline") && !head.contains("timeout"));
}

#[tokio::test]
async fn capped_timeouts_do_not_open_the_circuit() {
    let server =
        ScriptedServer::start(Behaviour::status("200 OK").after(Duration::from_secs(5))).await;
    let client = client(counting_resolver(&["slow.test"]), options());
    let consecutive =
        usize::try_from(BreakerPolicy::default().consecutive_timeouts).expect("small");
    for _ in 0..=consecutive {
        let deadline =
            Deadline::after(Instant::now(), Duration::from_millis(100)).expect("deadline");
        let result = deadline::scope(deadline, client.execute(server.request("slow.test"))).await;
        assert!(matches!(result, Err(EgressError::Timeout)));
    }
    assert_eq!(server.calls(), consecutive + 1);
}

#[tokio::test]
async fn an_almost_spent_deadline_fails_without_a_request() {
    let server = ScriptedServer::start(Behaviour::status("200 OK")).await;
    let client = client(counting_resolver(&["fast.test"]), options());
    let deadline = Deadline::after(Instant::now(), Duration::from_millis(20)).expect("deadline");

    let result = deadline::scope(deadline, client.execute(server.request("fast.test"))).await;

    assert!(matches!(result, Err(EgressError::Timeout)));
    assert_eq!(server.calls(), 0);
}

type Entries = Vec<(
    metrics_util::CompositeKey,
    Option<metrics::Unit>,
    Option<metrics::SharedString>,
    DebugValue,
)>;

fn snapshot_value<'a>(
    entries: &'a Entries,
    name: &str,
    labels: &[(&str, &str)],
) -> Option<&'a DebugValue> {
    entries
        .iter()
        .find(|(key, _, _, _)| {
            key.key().name() == name
                && labels.iter().all(|(label, value)| {
                    key.key()
                        .labels()
                        .any(|candidate| candidate.key() == *label && candidate.value() == *value)
                })
        })
        .map(|(_, _, _, value)| value)
}

#[tokio::test(flavor = "current_thread")]
async fn metrics_carry_the_destination_name_and_breaker_state() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let server = ScriptedServer::start(Behaviour::status("503 Service Unavailable")).await;
    let client = client(counting_resolver(&["down.test"]), options());

    call_many(&client, &server, "down.test", 12).await;

    let entries = snapshotter.snapshot().into_vec();
    let destination = ("destination", "scripted");
    assert_eq!(
        snapshot_value(
            &entries,
            EGRESS_REQUESTS_TOTAL,
            &[destination, ("status", "503"), ("outcome", "unavailable")]
        ),
        Some(&DebugValue::Counter(10))
    );
    assert_eq!(
        snapshot_value(
            &entries,
            EGRESS_REQUESTS_TOTAL,
            &[destination, ("status", "none"), ("outcome", "circuit_open")]
        ),
        Some(&DebugValue::Counter(2))
    );
    assert_eq!(
        snapshot_value(
            &entries,
            EGRESS_CIRCUIT_TRANSITIONS_TOTAL,
            &[destination, ("to", "open")]
        ),
        Some(&DebugValue::Counter(1))
    );
    assert!(matches!(
        snapshot_value(&entries, EGRESS_CIRCUIT_STATE, &[destination]),
        Some(DebugValue::Gauge(value)) if value.into_inner() == 2.0
    ));
    assert!(matches!(
        snapshot_value(&entries, EGRESS_REQUEST_DURATION_SECONDS, &[destination]),
        Some(DebugValue::Histogram(samples)) if samples.len() == 10
    ));
    let hosts_in_labels = entries.iter().any(|(key, ..)| {
        key.key()
            .labels()
            .any(|label| label.value().contains("down.test"))
    });
    assert!(!hosts_in_labels);
}

//! Integration tests for [`GuardedClient`] against local servers.

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::http::StatusCode;
use baukit_egress::{
    AddressPolicy, DestinationRejection, EgressError, EgressOptions, EgressOptionsError,
    EgressRequest, GuardedClient, PrivateNetworks, ResolveFuture, Resolver, ResponseBody,
    StaticResolver, validate_destination,
};
use baukit_http::RetryClass;
use time::{OffsetDateTime, format_description::well_known::Rfc2822};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};
use url::Url;

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const METADATA: IpAddr = IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254));
const PRIVATE: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
const PUBLIC: IpAddr = IpAddr::V4(Ipv4Addr::new(93, 184, 215, 14));
const HANG: Duration = Duration::from_secs(60);
const SHORT: Duration = Duration::from_millis(200);

#[derive(Clone)]
enum Reply {
    Respond(Vec<u8>),
    RespondThenWait(Vec<u8>),
    Hang,
}

struct LocalServer {
    port: u16,
    connections: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LocalServer {
    async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("local server should bind");
        let port = listener
            .local_addr()
            .expect("local server should report its address")
            .port();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(accept_loop(
            listener,
            reply,
            Arc::clone(&connections),
            Arc::clone(&requests),
        ));
        Self {
            port,
            connections,
            requests,
            task,
        }
    }

    fn url(&self, host: &str, path: &str) -> Url {
        Url::parse(&format!("http://{host}:{}{path}", self.port)).expect("test URL should parse")
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("request log should not be poisoned")
            .clone()
    }
}

async fn accept_loop(
    listener: TcpListener,
    reply: Reply,
    connections: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let mut tasks = JoinSet::new();
    while let Ok((stream, _)) = listener.accept().await {
        connections.fetch_add(1, Ordering::SeqCst);
        tasks.spawn(answer(stream, reply.clone(), Arc::clone(&requests)));
        while tasks.try_join_next().is_some() {}
    }
}

async fn answer(mut stream: TcpStream, reply: Reply, requests: Arc<Mutex<Vec<String>>>) {
    let request = read_request(&mut stream).await;
    requests
        .lock()
        .expect("request log should not be poisoned")
        .push(request);
    match reply {
        Reply::Respond(bytes) => {
            let _ = stream.write_all(&bytes).await;
            let _ = stream.shutdown().await;
        }
        Reply::RespondThenWait(bytes) => {
            stream
                .write_all(&bytes)
                .await
                .expect("headers should write");
            let mut buffer = [0_u8; 1];
            let _ = stream.read(&mut buffer).await;
        }
        Reply::Hang => tokio::time::sleep(HANG).await,
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 1024];
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..read]);
        if request_complete(&received) {
            break;
        }
    }
    String::from_utf8_lossy(&received).into_owned()
}

fn request_complete(received: &[u8]) -> bool {
    let text = String::from_utf8_lossy(received);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    body.len() >= length
}

fn response(status: &str, headers: &[(&str, &str)], body: &str) -> Reply {
    let mut text = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("\r\n");
    text.push_str(body);
    Reply::Respond(text.into_bytes())
}

fn chunked_response(chunks: &[&str]) -> Reply {
    chunked_status_response("200 OK", chunks)
}

fn chunked_status_response(status: &str, chunks: &[&str]) -> Reply {
    let mut text =
        format!("HTTP/1.1 {status}\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n");
    for chunk in chunks {
        text.push_str(&format!("{:x}\r\n{chunk}\r\n", chunk.len()));
    }
    text.push_str("0\r\n\r\n");
    Reply::Respond(text.into_bytes())
}

struct CountingResolver {
    answers: Mutex<Vec<Vec<IpAddr>>>,
    calls: AtomicUsize,
}

impl CountingResolver {
    fn new(answers: Vec<Vec<IpAddr>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers),
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Resolver for CountingResolver {
    fn resolve<'a>(&'a self, _host: &'a str) -> ResolveFuture<'a> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let answers = self
            .answers
            .lock()
            .expect("answers lock should not be poisoned");
        let answer = answers
            .get(call)
            .or_else(|| answers.last())
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(answer) })
    }
}

struct PendingResolver;

impl Resolver for PendingResolver {
    fn resolve<'a>(&'a self, _host: &'a str) -> ResolveFuture<'a> {
        Box::pin(std::future::pending())
    }
}

fn development() -> EgressOptions {
    EgressOptions::default().with_policy(AddressPolicy::AllowLoopback)
}

fn client(resolver: Arc<dyn Resolver>, options: EgressOptions) -> GuardedClient {
    GuardedClient::with_resolver(resolver, options).expect("guarded client should build")
}

fn loopback_resolver(hosts: &[&str]) -> Arc<StaticResolver> {
    Arc::new(hosts.iter().fold(StaticResolver::new(), |resolver, host| {
        resolver.with_host(*host, [LOOPBACK])
    }))
}

fn private_options() -> EgressOptions {
    let networks = PrivateNetworks::new(["10.255.255.0/24".parse().expect("valid network")])
        .expect("non-empty networks");
    EgressOptions::default()
        .with_policy(AddressPolicy::AllowPrivate(networks))
        .with_connect_timeout(SHORT)
        .expect("valid timeout")
        .with_request_timeout(Duration::from_secs(1))
        .expect("valid timeout")
}

#[tokio::test]
async fn listed_private_answers_reach_connect_and_unlisted_answers_are_blocked() {
    for scheme in ["http", "https"] {
        for address in ["10.255.255.1", "::ffff:10.255.255.1", "10.44.0.1"] {
            let resolver = StaticResolver::new().with_host(
                "prometheus.monitoring.svc",
                [address.parse().expect("valid address")],
            );
            let client = client(Arc::new(resolver), private_options());
            let url = Url::parse(&format!("{scheme}://prometheus.monitoring.svc:9/metrics"))
                .expect("valid URL");
            let error = client
                .execute(EgressRequest::get(url))
                .await
                .expect_err("destination is unreachable");
            if address == "10.44.0.1" {
                assert!(
                    matches!(error, EgressError::BlockedAddress),
                    "{scheme} {address}: {error:?}"
                );
            } else {
                assert!(
                    matches!(error, EgressError::Transport(_) | EgressError::Timeout),
                    "{scheme} {address}: {error:?}"
                );
            }
        }
    }
}

#[test]
fn private_address_literals_pass_the_same_destination_checks() {
    let options = private_options();
    for scheme in ["http", "https"] {
        for host in ["10.255.255.1", "[::ffff:10.255.255.1]"] {
            let url = Url::parse(&format!("{scheme}://{host}/metrics")).expect("valid URL");
            assert!(validate_destination(&url, options.policy()).is_ok());
        }
        let url = Url::parse(&format!("{scheme}://10.44.0.1/metrics")).expect("valid URL");
        assert!(matches!(
            validate_destination(&url, options.policy()),
            Err(EgressError::BlockedAddress)
        ));
    }
    let url = Url::parse("http://8.8.8.8/metrics").expect("valid URL");
    assert!(matches!(
        validate_destination(&url, options.policy()),
        Err(EgressError::Destination(DestinationRejection::Scheme))
    ));
}

#[tokio::test]
async fn private_plain_http_names_reject_public_and_mixed_answers() {
    let listed = "10.255.255.1".parse().expect("valid address");
    for (answers, expected_scheme) in [
        (vec![PUBLIC], true),
        (vec![listed, PUBLIC], true),
        (vec![PUBLIC, listed], true),
        (vec![listed, PRIVATE], false),
        (vec![PRIVATE, listed], false),
    ] {
        let resolver = StaticResolver::new().with_host("prometheus.monitoring.svc", answers);
        let client = client(Arc::new(resolver), private_options());
        let url = Url::parse("http://prometheus.monitoring.svc/metrics").expect("valid URL");
        let error = client
            .execute(EgressRequest::get(url))
            .await
            .expect_err("lookup must be refused");
        if expected_scheme {
            assert!(
                matches!(
                    error,
                    EgressError::Destination(DestinationRejection::Scheme)
                ),
                "{error:?}"
            );
        } else {
            assert!(matches!(error, EgressError::BlockedAddress), "{error:?}");
        }
    }
}

#[tokio::test]
async fn posts_to_the_address_the_resolver_returned() {
    let server = LocalServer::start(response("200 OK", &[], "ok")).await;
    let client = client(loopback_resolver(&["hooks.test"]), development());

    let delivered = client
        .execute(EgressRequest::post(server.url("hooks.test", "/deliver")).with_body("event"))
        .await
        .expect("delivery should succeed");

    assert_eq!(delivered.status(), StatusCode::OK);
    assert_eq!(delivered.body(), b"ok");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /deliver HTTP/1.1"));
    assert!(requests[0].ends_with("\r\n\r\nevent"));
}

#[tokio::test]
async fn the_default_policy_blocks_a_loopback_answer_before_connecting() {
    let server = LocalServer::start(response("200 OK", &[], "ok")).await;
    let client = client(loopback_resolver(&["hooks.test"]), EgressOptions::default());
    let url = Url::parse(&format!("https://hooks.test:{}/deliver", server.port))
        .expect("test URL should parse");

    let error = client
        .execute(EgressRequest::post(url))
        .await
        .expect_err("a loopback answer should be blocked");

    assert!(matches!(error, EgressError::BlockedAddress), "{error:?}");
    assert_eq!(error.retry_class(), RetryClass::Permanent);
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn one_disallowed_answer_blocks_the_whole_lookup() {
    let server = LocalServer::start(response("200 OK", &[], "ok")).await;
    for answers in [vec![LOOPBACK, PRIVATE], vec![PRIVATE, LOOPBACK]] {
        let resolver = CountingResolver::new(vec![answers]);
        let client = client(resolver, development());

        let error = client
            .execute(EgressRequest::post(server.url("hooks.test", "/deliver")))
            .await
            .expect_err("a mixed answer should be blocked");

        assert!(matches!(error, EgressError::BlockedAddress), "{error:?}");
    }
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn a_changed_dns_answer_never_reaches_a_connection() {
    let server = LocalServer::start(response("200 OK", &[], "ok")).await;
    let resolver = CountingResolver::new(vec![vec![LOOPBACK], vec![METADATA]]);
    let client = client(resolver.clone(), development());
    let url = server.url("hooks.test", "/deliver");

    client
        .execute(EgressRequest::post(url.clone()))
        .await
        .expect("the first answer is allowed");
    assert_eq!(resolver.calls(), 1, "one request resolves once");

    let error = client
        .execute(EgressRequest::post(url))
        .await
        .expect_err("the changed answer should be blocked");

    assert!(matches!(error, EgressError::BlockedAddress), "{error:?}");
    assert_eq!(resolver.calls(), 2);
    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn redirects_are_returned_and_never_followed() {
    let internal = LocalServer::start(response("200 OK", &[], "internal")).await;
    let targets = [
        internal.url("internal.test", "/steal").to_string(),
        "http://169.254.169.254/latest/meta-data/".to_owned(),
    ];
    for target in targets {
        let redirecting =
            LocalServer::start(response("302 Found", &[("location", &target)], "")).await;
        let client = client(
            loopback_resolver(&["hooks.test", "internal.test"]),
            development(),
        );

        let error = client
            .execute(EgressRequest::post(
                redirecting.url("hooks.test", "/deliver"),
            ))
            .await
            .expect_err("a redirect should not be followed");

        assert!(
            matches!(
                error,
                EgressError::Status {
                    status: StatusCode::FOUND,
                    class: RetryClass::Permanent,
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(redirecting.connections(), 1);
    }
    assert_eq!(internal.connections(), 0);
}

#[tokio::test]
async fn address_literals_are_checked_before_connecting() {
    let server = LocalServer::start(response("200 OK", &[], "ok")).await;
    let client = client(Arc::new(StaticResolver::new()), EgressOptions::default());
    let urls = [
        format!("https://127.0.0.1:{}/deliver", server.port),
        format!("https://[::ffff:127.0.0.1]:{}/deliver", server.port),
        format!("https://2130706433:{}/deliver", server.port),
    ];

    for url in urls {
        let url = Url::parse(&url).expect("test URL should parse");
        let error = client
            .execute(EgressRequest::get(url))
            .await
            .expect_err("a loopback literal should be blocked");
        assert!(matches!(error, EgressError::BlockedAddress), "{error:?}");
    }
    assert_eq!(server.connections(), 0);
}

#[tokio::test]
async fn refused_urls_name_the_rejection() {
    let client = client(Arc::new(StaticResolver::new()), EgressOptions::default());
    let cases = [
        ("http://hooks.test/deliver", DestinationRejection::Scheme),
        (
            "https://user:url-secret@hooks.test/deliver",
            DestinationRejection::Credentials,
        ),
        (
            "https://hooks.test/deliver#part",
            DestinationRejection::Fragment,
        ),
    ];

    for (url, rejection) in cases {
        let url = Url::parse(url).expect("test URL should parse");
        let error = client
            .execute(EgressRequest::post(url))
            .await
            .expect_err("the URL should be refused");
        assert!(
            matches!(error, EgressError::Destination(found) if found == rejection),
            "{error:?}"
        );
        assert!(!error.is_retryable());
    }
}

#[tokio::test]
async fn statuses_use_the_shared_classification() {
    let cases = [
        (
            response("429 Too Many Requests", &[("retry-after", "7")], ""),
            RetryClass::RetryAfter(Duration::from_secs(7)),
        ),
        (response("425 Too Early", &[], ""), RetryClass::Unavailable),
        (
            response("503 Service Unavailable", &[], ""),
            RetryClass::Unavailable,
        ),
        (
            response("504 Gateway Timeout", &[], ""),
            RetryClass::Timeout,
        ),
        (response("401 Unauthorized", &[], ""), RetryClass::Revoked),
        (response("404 Not Found", &[], ""), RetryClass::Permanent),
    ];

    for (reply, expected) in cases {
        let server = LocalServer::start(reply).await;
        let client = client(loopback_resolver(&["hooks.test"]), development());

        let error = client
            .execute(EgressRequest::post(server.url("hooks.test", "/deliver")))
            .await
            .expect_err("a non-2xx status is an error");

        assert_eq!(error.retry_class(), expected, "{error:?}");
        assert_eq!(error.code(), "upstream_status");
    }
}

#[tokio::test]
async fn forbidden_rate_limit_requires_opt_in_and_keeps_the_delay_cap() {
    let server = LocalServer::start(response("403 Forbidden", &[("retry-after", "7")], "")).await;
    let options = development();
    assert!(!options.forbidden_rate_limit());
    let capped = options
        .clone()
        .with_forbidden_rate_limit()
        .with_max_retry_after(Duration::from_secs(3))
        .expect("cap is valid");
    let cases = [
        (options.clone(), RetryClass::Revoked),
        (
            options.with_forbidden_rate_limit(),
            RetryClass::RetryAfter(Duration::from_secs(7)),
        ),
        (capped, RetryClass::RetryAfter(Duration::from_secs(3))),
    ];
    for (options, expected) in cases {
        let client = client(loopback_resolver(&["hooks.test"]), options);
        assert_eq!(
            client.options().forbidden_rate_limit(),
            expected.is_retryable()
        );
        let error = client
            .execute(EgressRequest::get(server.url("hooks.test", "/limit")))
            .await
            .expect_err("a 403 is an error");

        assert!(
            matches!(error, EgressError::Status { status: StatusCode::FORBIDDEN, class, .. } if class == expected),
            "{error:?}"
        );
        assert_eq!(error.is_retryable(), expected.is_retryable());
    }
}

#[tokio::test]
async fn long_retry_after_delays_are_capped() {
    let cases = [
        (development(), Duration::from_secs(300)),
        (
            development()
                .with_max_retry_after(Duration::from_secs(30))
                .expect("cap is valid"),
            Duration::from_secs(30),
        ),
    ];

    for (options, cap) in cases {
        let server = LocalServer::start(response(
            "429 Too Many Requests",
            &[("retry-after", "86400")],
            "",
        ))
        .await;
        let client = client(loopback_resolver(&["hooks.test"]), options);

        let error = client
            .execute(EgressRequest::post(server.url("hooks.test", "/deliver")))
            .await
            .expect_err("a 429 is an error");

        assert_eq!(error.retry_class(), RetryClass::RetryAfter(cap));
    }
}

#[test]
fn a_zero_retry_after_cap_is_refused() {
    assert_eq!(
        EgressOptions::default().with_max_retry_after(Duration::ZERO),
        Err(EgressOptionsError::ZeroRetryAfterCap)
    );
    assert_eq!(
        EgressOptions::default().max_retry_after(),
        Duration::from_secs(300)
    );
}

#[tokio::test]
async fn the_response_body_limit_is_enforced() {
    let options = development()
        .with_max_response_bytes(16)
        .expect("limit is valid");
    let cases = [
        (response("200 OK", &[], &"x".repeat(17)), false),
        (chunked_response(&["x".repeat(12).as_str(), "yyyyy"]), false),
        (response("200 OK", &[], &"x".repeat(16)), true),
        (
            chunked_response(&["x".repeat(8).as_str(), "y".repeat(8).as_str()]),
            true,
        ),
    ];

    for (reply, fits) in cases {
        let server = LocalServer::start(reply).await;
        let client = client(loopback_resolver(&["hooks.test"]), options.clone());

        let outcome = client
            .execute(EgressRequest::get(server.url("hooks.test", "/")))
            .await;

        match outcome {
            Ok(response) => {
                assert!(
                    fits,
                    "body of {} bytes should be refused",
                    response.body().len()
                );
            }
            Err(error) => {
                assert!(!fits, "{error:?}");
                assert!(
                    matches!(error, EgressError::ResponseTooLarge { limit: 16 }),
                    "{error:?}"
                );
                assert!(!error.is_retryable());
            }
        }
    }
}

#[tokio::test]
async fn a_discarded_success_body_is_never_too_large() {
    let options = development()
        .with_max_response_bytes(16)
        .expect("limit is valid");
    let replies = [
        response("200 OK", &[], &"x".repeat(1024)),
        chunked_response(&["x".repeat(12).as_str(), "y".repeat(12).as_str()]),
    ];

    for reply in replies {
        let server = LocalServer::start(reply).await;
        let client = client(loopback_resolver(&["hooks.test"]), options.clone());

        let delivered = client
            .execute(
                EgressRequest::post(server.url("hooks.test", "/deliver"))
                    .with_response_body(ResponseBody::Discard),
            )
            .await
            .expect("a discarded body should count as delivered");

        assert_eq!(delivered.status(), StatusCode::OK);
        assert!(delivered.body().is_empty());
        assert_eq!(server.requests().len(), 1);
    }
}

#[tokio::test]
async fn a_discarded_body_keeps_the_status_classification() {
    let server = LocalServer::start(response("503 Service Unavailable", &[], "busy")).await;
    let client = client(loopback_resolver(&["hooks.test"]), development());

    let error = client
        .execute(
            EgressRequest::post(server.url("hooks.test", "/deliver"))
                .with_response_body(ResponseBody::Discard),
        )
        .await
        .expect_err("a 503 is not delivered");

    assert!(
        matches!(
            error,
            EgressError::Status {
                status: StatusCode::SERVICE_UNAVAILABLE,
                class: RetryClass::Unavailable,
                ..
            }
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn service_unavailable_honors_retry_after_seconds_and_http_date() {
    let deadline = OffsetDateTime::now_utc() + time::Duration::minutes(2);
    let date = deadline.format(&Rfc2822).expect("HTTP date should format");
    for value in ["7", date.as_str()] {
        let server = LocalServer::start(response(
            "503 Service Unavailable",
            &[("retry-after", value)],
            "",
        ))
        .await;
        let client = client(loopback_resolver(&["hooks.test"]), development());
        let before = OffsetDateTime::now_utc();
        let error = client
            .execute(EgressRequest::get(server.url("hooks.test", "/")))
            .await
            .expect_err("a 503 is an error");
        let after = OffsetDateTime::now_utc();
        let delay = error
            .retry_class()
            .retry_after()
            .expect("delay is retained");
        if value == "7" {
            assert_eq!(delay, Duration::from_secs(7));
        } else {
            let parsed = OffsetDateTime::parse(&date, &Rfc2822).expect("HTTP date should parse");
            let lower: Duration = (parsed - after)
                .try_into()
                .expect("deadline is in the future");
            let upper: Duration = (parsed - before)
                .try_into()
                .expect("deadline is in the future");
            assert!(delay >= lower && delay <= upper, "delay {delay:?}");
        }
    }
}

#[test]
fn error_body_limits_are_opt_in_and_validated_in_both_builder_orders() {
    let options = EgressOptions::default();
    assert_eq!(options.error_body_limit(), None);
    assert_eq!(
        options.clone().with_error_body_limit(0),
        Err(EgressOptionsError::ZeroErrorBodyLimit)
    );
    assert_eq!(
        options
            .clone()
            .with_error_body_limit(options.max_response_bytes() + 1),
        Err(EgressOptionsError::ErrorBodyLimitTooLarge)
    );
    let options = options.with_max_response_bytes(16).expect("limit is valid");
    assert_eq!(
        options.clone().with_error_body_limit(17),
        Err(EgressOptionsError::ErrorBodyLimitTooLarge)
    );
    let options = options
        .with_error_body_limit(16)
        .expect("equal limit is valid");
    assert_eq!(options.error_body_limit(), Some(16));
    assert_eq!(
        options.clone().with_max_response_bytes(15),
        Err(EgressOptionsError::ErrorBodyLimitTooLarge)
    );
    assert_eq!(options.clone().with_max_response_bytes(16), Ok(options));
}

#[tokio::test]
async fn conflict_body_exposes_the_baukit_problem_code_and_redacts_personal_data() {
    let bytes = r#"{"error":{"code":"inventory_sequence_gap","message":"email-secret@example.com","requestId":"request-secret","details":{"email":"email-secret@example.com"}}}"#;
    let server = LocalServer::start(response(
        "409 Conflict",
        &[("content-type", "application/json; charset=utf-8")],
        bytes,
    ))
    .await;
    let options = development()
        .with_error_body_limit(1024)
        .expect("limit is valid");
    let client = client(loopback_resolver(&["hooks.test"]), options);
    let error = client
        .execute(
            EgressRequest::get(server.url("hooks.test", "/"))
                .with_response_body(ResponseBody::Discard),
        )
        .await
        .expect_err("a conflict is an error");

    assert!(matches!(
        error,
        EgressError::Status {
            status: StatusCode::CONFLICT,
            class: RetryClass::Permanent,
            ..
        }
    ));
    let body = error.error_body().expect("body capture is enabled");
    assert_eq!(body.bytes(), bytes.as_bytes());
    assert!(!body.truncated());
    assert_eq!(body.content_type(), Some("application/json; charset=utf-8"));
    assert_eq!(body.problem_code(), Some("inventory_sequence_gap"));
    assert_eq!(
        error.to_string(),
        "egress destination answered with status 409 Conflict"
    );
    assert_eq!(
        format!("{body:?}"),
        format!(
            "ErrorBody {{ len: {}, truncated: false, problem_code: Some(\"inventory_sequence_gap\") }}",
            bytes.len()
        )
    );
    for text in [error.to_string(), format!("{error:?}"), format!("{body:?}")] {
        for secret in [
            "email-secret",
            "request-secret",
            "application/json",
            "message",
            "details",
        ] {
            assert!(!text.contains(secret), "{secret} leaked into {text}");
        }
    }
}

#[tokio::test]
async fn disabled_error_body_capture_does_not_read_an_incomplete_body() {
    let server = LocalServer::start(Reply::Respond(
        b"HTTP/1.1 409 Conflict\r\ncontent-length: 1024\r\nconnection: close\r\n\r\n".to_vec(),
    ))
    .await;
    let client = client(loopback_resolver(&["hooks.test"]), development());
    let error = client
        .execute(EgressRequest::get(server.url("hooks.test", "/")))
        .await
        .expect_err("a conflict is an error");
    assert!(matches!(
        error,
        EgressError::Status {
            status: StatusCode::CONFLICT,
            body: None,
            ..
        }
    ));
    assert!(error.error_body().is_none());
}

#[tokio::test]
async fn error_bodies_stop_at_the_limit_with_and_without_content_length() {
    let options = development()
        .with_error_body_limit(16)
        .expect("limit is valid");
    let cases = [
        (response("500 Internal Server Error", &[], "abcdefghijklmnopq"), "abcdefghijklmnop", true),
        (chunked_status_response("500 Internal Server Error", &["abcdefgh", "ijklmnopq"]), "abcdefghijklmnop", true),
        (response("500 Internal Server Error", &[], "abcdefghijklmnop"), "abcdefghijklmnop", false),
        (chunked_status_response("500 Internal Server Error", &["abcdefghijklmnop"]), "abcdefghijklmnop", true),
        (chunked_status_response("500 Internal Server Error", &["abcdefghijklmno"]), "abcdefghijklmno", false),
        (Reply::RespondThenWait(b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 1024\r\n\r\nabcdefghijklmnop".to_vec()), "abcdefghijklmnop", true),
    ];
    for (reply, expected, truncated) in cases {
        let server = LocalServer::start(reply).await;
        let client = client(loopback_resolver(&["hooks.test"]), options.clone());
        let error = client
            .execute(EgressRequest::get(server.url("hooks.test", "/")))
            .await
            .expect_err("a 500 is an error");
        assert!(matches!(
            error,
            EgressError::Status {
                class: RetryClass::Unavailable,
                ..
            }
        ));
        let body = error.error_body().expect("body should be captured");
        assert_eq!(body.truncated(), truncated);
        assert_eq!(body.bytes(), expected.as_bytes());
        assert_eq!(body.problem_code(), None);
        assert_eq!(body.content_type(), None);
    }
}

#[tokio::test]
async fn non_envelope_and_truncated_json_bodies_have_no_problem_code() {
    let cases = [
        ("plain text", 1024),
        (r#"{"code":"inventory_sequence_gap"}"#, 1024),
        (r#"{"error":{"code":42}}"#, 1024),
        (
            r#"{"error":{"code":"inventory_sequence_gap","message":"gap","requestId":"test","details":{}}}"#,
            16,
        ),
        ("", 1024),
    ];
    for (bytes, limit) in cases {
        let server = LocalServer::start(response("401 Unauthorized", &[], bytes)).await;
        let options = development()
            .with_error_body_limit(limit)
            .expect("limit is valid");
        let client = client(loopback_resolver(&["hooks.test"]), options);
        let error = client
            .execute(EgressRequest::get(server.url("hooks.test", "/")))
            .await
            .expect_err("a 401 is an error");
        assert_eq!(error.retry_class(), RetryClass::Revoked);
        assert_eq!(
            error.error_body().expect("body is captured").problem_code(),
            None
        );
    }
}

#[tokio::test]
async fn a_failed_error_body_read_preserves_the_status_and_capped_retry_after() {
    let server = LocalServer::start(Reply::Respond(
        b"HTTP/1.1 503 Service Unavailable\r\nretry-after: 2\r\ncontent-length: 1024\r\nconnection: close\r\n\r\nbusy".to_vec(),
    ))
    .await;
    let options = development()
        .with_max_retry_after(Duration::from_secs(1))
        .expect("retry delay cap is valid");
    for options in [
        options.clone(),
        options.with_error_body_limit(16).expect("limit is valid"),
    ] {
        let client = client(loopback_resolver(&["hooks.test"]), options);
        let error = client
            .execute(EgressRequest::get(server.url("hooks.test", "/")))
            .await
            .expect_err("a 503 is an error");
        assert!(
            matches!(error, EgressError::Status {
                status: StatusCode::SERVICE_UNAVAILABLE,
                class: RetryClass::RetryAfter(delay),
                body: None,
            } if delay == Duration::from_secs(1)),
            "{error:?}"
        );
        assert_eq!(error.code(), "upstream_status");
        assert!(error.is_retryable());
        assert!(error.error_body().is_none());
    }
}

#[tokio::test]
async fn a_timed_out_error_body_read_preserves_the_status_and_capped_retry_after() {
    let server = LocalServer::start(Reply::RespondThenWait(
        b"HTTP/1.1 503 Service Unavailable\r\nretry-after: 2\r\ncontent-length: 1024\r\n\r\nbusy"
            .to_vec(),
    ))
    .await;
    let options = development()
        .with_max_retry_after(Duration::from_secs(1))
        .expect("retry delay cap is valid")
        .with_request_timeout(SHORT)
        .expect("timeout is valid");
    for options in [
        options.clone(),
        options.with_error_body_limit(16).expect("limit is valid"),
    ] {
        let client = client(loopback_resolver(&["hooks.test"]), options.clone());
        let start = Instant::now();
        let error = client
            .execute(EgressRequest::get(server.url("hooks.test", "/")))
            .await
            .expect_err("a 503 is an error");
        if options.error_body_limit().is_some() {
            assert!(
                start.elapsed() >= SHORT,
                "body reading should reach its timeout"
            );
        }
        assert!(
            matches!(error, EgressError::Status {
                status: StatusCode::SERVICE_UNAVAILABLE,
                class: RetryClass::RetryAfter(delay),
                body: None,
            } if delay == Duration::from_secs(1)),
            "{error:?}"
        );
        assert_eq!(error.code(), "upstream_status");
        assert!(error.is_retryable());
        assert!(error.error_body().is_none());
    }
}

#[tokio::test]
async fn plain_http_reaches_only_loopback_under_allow_loopback() {
    let server = LocalServer::start(response("200 OK", &[], "ok")).await;
    let resolver = Arc::new(
        StaticResolver::new()
            .with_host("public.test", [PUBLIC])
            .with_host("mixed.test", [LOOPBACK, PUBLIC])
            .with_host("local.test", [LOOPBACK]),
    );
    let client = client(resolver, development());

    for host in ["public.test", "mixed.test"] {
        let error = client
            .execute(EgressRequest::post(server.url(host, "/deliver")))
            .await
            .expect_err("plain http to a public answer should be refused");
        assert!(
            matches!(
                error,
                EgressError::Destination(DestinationRejection::Scheme)
            ),
            "{host}: {error:?}"
        );
        assert!(!error.is_retryable());
    }
    let literal = Url::parse("http://93.184.215.14/deliver").expect("test URL should parse");
    let error = client
        .execute(EgressRequest::post(literal))
        .await
        .expect_err("plain http to a public literal should be refused");
    assert!(
        matches!(
            error,
            EgressError::Destination(DestinationRejection::Scheme)
        ),
        "{error:?}"
    );
    assert_eq!(server.connections(), 0);

    client
        .execute(EgressRequest::post(server.url("local.test", "/deliver")))
        .await
        .expect("plain http to loopback is allowed");
    assert_eq!(server.connections(), 1);
}

#[tokio::test]
async fn a_silent_server_times_out() {
    let server = LocalServer::start(Reply::Hang).await;
    let options = development()
        .with_request_timeout(SHORT)
        .expect("timeout is valid");
    let client = client(loopback_resolver(&["hooks.test"]), options);

    let error = client
        .execute(EgressRequest::post(server.url("hooks.test", "/deliver")))
        .await
        .expect_err("the request should time out");

    assert!(matches!(error, EgressError::Timeout), "{error:?}");
    assert_eq!(error.retry_class(), RetryClass::Timeout);
}

#[tokio::test]
async fn a_slow_lookup_times_out() {
    let options = development()
        .with_resolve_timeout(SHORT)
        .expect("timeout is valid");
    let client = client(Arc::new(PendingResolver), options);
    let url = Url::parse("http://hooks.test/deliver").expect("test URL should parse");

    let error = client
        .execute(EgressRequest::post(url))
        .await
        .expect_err("the lookup should time out");

    assert!(matches!(error, EgressError::Timeout), "{error:?}");
}

#[tokio::test]
async fn lookup_and_connection_failures_are_retryable() {
    let closed_port = {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener should bind");
        listener
            .local_addr()
            .expect("listener should report its address")
            .port()
    };
    let client = client(loopback_resolver(&["hooks.test"]), development());

    let unknown = client
        .execute(EgressRequest::post(
            Url::parse("http://unknown.test/deliver").expect("test URL should parse"),
        ))
        .await
        .expect_err("an unknown name should fail");
    assert!(matches!(unknown, EgressError::Resolve), "{unknown:?}");
    assert_eq!(unknown.retry_class(), RetryClass::Unavailable);

    let refused = client
        .execute(EgressRequest::post(
            Url::parse(&format!("http://hooks.test:{closed_port}/deliver"))
                .expect("test URL should parse"),
        ))
        .await
        .expect_err("a closed port should fail");
    assert!(matches!(refused, EgressError::Transport(_)), "{refused:?}");
    assert_eq!(refused.retry_class(), RetryClass::Unavailable);
}

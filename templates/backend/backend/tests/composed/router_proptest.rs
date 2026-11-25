use std::{error::Error, net::SocketAddr};

use axum::http::{Method, Uri};
use proptest::{
    prelude::*,
    test_runner::{Config, TestRunner},
};

use crate::support::{self, Quota};

const CASES: u32 = 256;
const METHODS: [Method; 7] = [
    Method::GET,
    Method::POST,
    Method::PUT,
    Method::PATCH,
    Method::DELETE,
    Method::HEAD,
    Method::OPTIONS,
];

#[derive(Clone, Debug)]
struct Probe {
    method: Method,
    path: String,
    authorization: Option<String>,
}

fn probes(paths: Vec<String>, tokens: Vec<String>) -> impl Strategy<Value = Probe> {
    let method = proptest::sample::select(METHODS.to_vec());
    let path = prop_oneof![
        proptest::sample::select(paths),
        "/[a-zA-Z0-9._~%-]{0,12}(/[a-zA-Z0-9._~%{}-]{0,12}){0,3}",
    ];
    let authorization = prop_oneof![
        Just(None),
        "[ -~]{0,64}".prop_map(Some),
        "[ -~]{0,64}".prop_map(|token| Some(format!("Bearer {token}"))),
        proptest::sample::select(tokens).prop_map(|token| Some(format!("Bearer {token}"))),
    ];
    (method, path, authorization).prop_map(|(method, path, authorization)| Probe {
        method,
        path,
        authorization,
    })
}

/// G-07: random requests never succeed on secured paths, never 5xx, always carry security headers.
#[test]
fn random_requests_fail_closed_without_server_errors() -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
{% if context.auth_enabled %}    let composed = runtime.block_on(support::compose(&Quota::GENEROUS, None))?;
{% else %}    let composed = support::compose(&Quota::GENEROUS)?;
{% endif %}
    let routes = support::inventory(&composed.api)?;
    let paths: Vec<String> = routes.iter().map(|(_, path)| path.clone()).collect();
{% if context.auth_enabled %}    let secured: Vec<String> = support::secured_paths()?;
    let tokens = support::hostile_tokens(&composed.issuer)?;
{% else %}    let tokens = vec!["not-a-jwt".to_owned()];
{% endif %}    let client: SocketAddr = "10.200.0.1:40000".parse()?;
    let mut runner = TestRunner::new(Config::with_cases(CASES));
    runner
        .run(&probes(paths, tokens), |probe| {
            if probe.path.parse::<Uri>().is_err() {
                return Ok(());
            }
            let request = support::request(
                probe.method.clone(),
                &probe.path,
                client,
                probe.authorization.as_deref(),
                Vec::new(),
            );
            let Ok(request) = request else {
                return Ok(());
            };
            let reply = runtime
                .block_on(support::send(&composed.api.router, request))
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            let label = format!("{} {}", probe.method, probe.path);
            prop_assert!(
                !reply.status.is_server_error(),
                "{label}: status {}",
                reply.status
            );
{% if context.auth_enabled %}            // CORS answers preflights before auth; they carry no resource data.
            let is_preflight = probe.method == Method::OPTIONS;
            prop_assert!(
                is_preflight
                    || !(reply.status.is_success() && support::is_secured(&secured, &probe.path)),
                "{label}: secured path answered {} without a valid token",
                reply.status
            );
{% endif %}            support::assert_security_headers(&label, &reply.headers);
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

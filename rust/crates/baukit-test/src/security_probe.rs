use std::collections::BTreeMap;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderName, HeaderValue, Method, Request, header},
};
use serde_json::Value;
use tower::ServiceExt as _;

const BODY_LIMIT: usize = 1024 * 1024;
const HTTP_METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// Where security probes send their requests.
///
/// A router target calls the app in process through `tower::ServiceExt::oneshot`.
/// A base URL target sends real HTTP requests, so it also covers a deployed stack.
#[derive(Clone, Debug)]
pub enum SecurityTarget {
    /// In-process axum router, including all layers the product mounts.
    Router(Router),
    /// Origin such as `http://127.0.0.1:8080`, without a trailing slash.
    BaseUrl(String),
}

impl SecurityTarget {
    /// Probes an in-process router.
    #[must_use]
    pub fn router(router: Router) -> Self {
        Self::Router(router)
    }

    /// Probes a running server at `base_url`.
    #[must_use]
    pub fn base_url(base_url: impl Into<String>) -> Self {
        Self::BaseUrl(base_url.into().trim_end_matches('/').to_owned())
    }

    pub(crate) async fn send(&self, probe: ProbeRequest<'_>) -> Result<ProbeResponse, String> {
        match self {
            Self::Router(router) => send_to_router(router, probe).await,
            Self::BaseUrl(base_url) => send_to_url(base_url, probe).await,
        }
    }
}

/// Path values, request bodies and headers the probes use for each operation.
///
/// Path values map an OpenAPI path parameter name, such as `itemId`, to the
/// value substituted into every path that names it. Bodies are keyed by method
/// and path template, for example `("put", "/items/{itemId}")`, and are sent
/// as JSON. Headers go on every request, for example `if-match` or
/// `idempotency-key` that a product requires before its owner check runs.
#[derive(Clone, Debug, Default)]
pub struct OperationFixtures {
    path_values: BTreeMap<String, String>,
    bodies: BTreeMap<(String, String), Value>,
    headers: Vec<(String, String)>,
}

impl OperationFixtures {
    /// Creates empty fixtures.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Substitutes `value` for the path parameter `name`.
    #[must_use]
    pub fn path_value(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.path_values.insert(name.into(), value.into());
        self
    }

    /// Sends `body` as JSON to the operation `method path`.
    #[must_use]
    pub fn body(mut self, method: &str, path: impl Into<String>, body: Value) -> Self {
        self.bodies
            .insert((method.to_lowercase(), path.into()), body);
        self
    }

    /// Adds a header to every probe request.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub(crate) fn path_values(&self) -> &BTreeMap<String, String> {
        &self.path_values
    }

    pub(crate) fn request<'a>(
        &'a self,
        operation: &'a Operation,
        uri: &'a str,
        authorization: Option<&'a str>,
    ) -> ProbeRequest<'a> {
        ProbeRequest {
            method: &operation.method,
            uri,
            authorization,
            headers: &self.headers,
            body: self
                .bodies
                .get(&(operation.method.clone(), operation.path.clone())),
        }
    }
}

/// One documented operation.
#[derive(Clone, Debug)]
pub(crate) struct Operation {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) item: Value,
}

impl Operation {
    pub(crate) fn label(&self) -> String {
        format!("{} {}", self.method.to_uppercase(), self.path)
    }

    pub(crate) fn path_parameters(&self) -> Vec<&str> {
        path_parameters(&self.path)
    }
}

pub(crate) fn operations(document: &Value) -> Result<Vec<Operation>, String> {
    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| "the OpenAPI document has no paths object".to_owned())?;
    let mut operations = Vec::new();
    for (path, item) in paths {
        for method in HTTP_METHODS {
            if let Some(operation) = item.get(method) {
                operations.push(Operation {
                    method: method.to_owned(),
                    path: path.clone(),
                    item: operation.clone(),
                });
            }
        }
    }
    Ok(operations)
}

pub(crate) fn path_parameters(path: &str) -> Vec<&str> {
    path.split('{')
        .skip(1)
        .filter_map(|segment| segment.split_once('}').map(|(name, _)| name))
        .collect()
}

pub(crate) fn substitute(path: &str, values: &BTreeMap<String, String>) -> Result<String, String> {
    let mut uri = path.to_owned();
    for name in path_parameters(path) {
        let value = values
            .get(name)
            .ok_or_else(|| format!("no fixture value for path parameter `{name}`"))?;
        uri = uri.replace(&format!("{{{name}}}"), &percent_encode(value));
    }
    Ok(uri)
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

pub(crate) struct ProbeRequest<'a> {
    method: &'a str,
    uri: &'a str,
    authorization: Option<&'a str>,
    headers: &'a [(String, String)],
    body: Option<&'a Value>,
}

pub(crate) struct ProbeResponse {
    pub(crate) status: u16,
    pub(crate) body: String,
}

fn method(name: &str) -> Result<Method, String> {
    Method::from_bytes(name.to_uppercase().as_bytes()).map_err(|error| error.to_string())
}

async fn send_to_router(router: &Router, probe: ProbeRequest<'_>) -> Result<ProbeResponse, String> {
    let mut builder = Request::builder()
        .method(method(probe.method)?)
        .uri(probe.uri);
    for (name, value) in probe.headers {
        builder = builder.header(
            HeaderName::try_from(name.as_str()).map_err(|error| error.to_string())?,
            HeaderValue::try_from(value.as_str()).map_err(|error| error.to_string())?,
        );
    }
    if let Some(token) = probe.authorization {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match probe.body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let request = builder.body(body).map_err(|error| error.to_string())?;
    let response = router
        .clone()
        .oneshot(request)
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), BODY_LIMIT)
        .await
        .map_err(|error| error.to_string())?;
    Ok(ProbeResponse {
        status,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

async fn send_to_url(base_url: &str, probe: ProbeRequest<'_>) -> Result<ProbeResponse, String> {
    let method = reqwest::Method::from_bytes(probe.method.to_uppercase().as_bytes())
        .map_err(|error| error.to_string())?;
    let client = crate::test_http_client()
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.request(method, format!("{base_url}{}", probe.uri));
    for (name, value) in probe.headers {
        request = request.header(name, value);
    }
    if let Some(token) = probe.authorization {
        request = request.bearer_auth(token);
    }
    if let Some(body) = probe.body {
        request = request.json(body);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|error| error.to_string())?;
    Ok(ProbeResponse { status, body })
}

/// A route left out of a probe, with the reason printed in every report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteExclusion {
    /// Lowercase HTTP method.
    pub method: String,
    /// Path template as written in the document.
    pub path: String,
    /// Why the route needs no probe, printed with every run.
    pub reason: String,
}

impl RouteExclusion {
    pub(crate) fn new(method: &str, path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            method: method.to_lowercase(),
            path: path.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn matches(&self, operation: &Operation) -> bool {
        self.method == operation.method && self.path == operation.path
    }

    pub(crate) fn label(&self) -> String {
        format!("{} {}", self.method.to_uppercase(), self.path)
    }
}

pub(crate) fn validate_exclusions(
    exclusions: &[RouteExclusion],
    operations: &[Operation],
    violations: &mut Vec<String>,
) {
    for exclusion in exclusions {
        if exclusion.reason.trim().is_empty() {
            violations.push(format!("exclusion {} has no reason", exclusion.label()));
        }
        if !operations
            .iter()
            .any(|operation| exclusion.matches(operation))
        {
            violations.push(format!(
                "exclusion {} names no documented operation",
                exclusion.label()
            ));
        }
    }
}

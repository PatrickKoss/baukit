use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use axum::http::{
    HeaderMap, HeaderName, HeaderValue, Request, Response,
    header::{
        AUTHORIZATION, CACHE_CONTROL, CONTENT_SECURITY_POLICY, HOST, REFERRER_POLICY,
        STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
    },
};
use tower::{Layer, Service};

/// The request header the browser session client sends as a CSRF signal.
pub const X_BAUKIT_SESSION: HeaderName = HeaderName::from_static("x-baukit-session");

const SESSION_PATH_PREFIX: &str = "/auth/session/";
const NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");
const NO_REFERRER: HeaderValue = HeaderValue::from_static("no-referrer");
const DENY: HeaderValue = HeaderValue::from_static("DENY");
const API_CSP: HeaderValue = HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'");
const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");
const NO_STORE_DIRECTIVE: &str = "no-store";
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// Adds the baukit API security headers to every response.
///
/// `X-Content-Type-Options`, `Referrer-Policy`, `X-Frame-Options` and
/// `Content-Security-Policy` are set unless the handler already set them, so a
/// route serving HTML can ship its own CSP. Responses to `/auth/session/*` and
/// to requests carrying `Authorization` always end up with `no-store`: a
/// handler `Cache-Control` without it is replaced. HSTS is added only when
/// configured and never for a loopback `Host`.
#[derive(Clone, Debug, Default)]
pub struct SecurityHeadersLayer {
    hsts: Option<HeaderValue>,
}

impl SecurityHeadersLayer {
    /// Creates the layer, with HSTS when `hsts_max_age` is set.
    #[must_use]
    pub fn new(hsts_max_age: Option<Duration>) -> Self {
        Self {
            hsts: hsts_max_age.map(hsts_header_value),
        }
    }
}

impl<S> Layer<S> for SecurityHeadersLayer {
    type Service = SecurityHeaders<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SecurityHeaders {
            inner,
            hsts: self.hsts.clone(),
        }
    }
}

/// The service produced by [`SecurityHeadersLayer`].
#[derive(Clone, Debug)]
pub struct SecurityHeaders<S> {
    inner: S,
    hsts: Option<HeaderValue>,
}

type BoxedResponseFuture<B, E> = Pin<Box<dyn Future<Output = Result<Response<B>, E>> + Send>>;

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for SecurityHeaders<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = BoxedResponseFuture<ResBody, S::Error>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        let no_store = requires_no_store(&request);
        let hsts = self.hsts.clone().filter(|_| !is_loopback_host(&request));
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            let mut response = inner.call(request).await?;
            apply_headers(response.headers_mut(), no_store, hsts);
            Ok(response)
        })
    }
}

fn apply_headers(headers: &mut HeaderMap, no_store: bool, hsts: Option<HeaderValue>) {
    for (name, value) in [
        (X_CONTENT_TYPE_OPTIONS, NOSNIFF),
        (REFERRER_POLICY, NO_REFERRER),
        (X_FRAME_OPTIONS, DENY),
        (CONTENT_SECURITY_POLICY, API_CSP),
    ] {
        headers.entry(name).or_insert(value);
    }
    if no_store && !has_no_store(headers) {
        headers.insert(CACHE_CONTROL, NO_STORE);
    }
    if let Some(hsts) = hsts {
        headers.insert(STRICT_TRANSPORT_SECURITY, hsts);
    }
}

fn requires_no_store<B>(request: &Request<B>) -> bool {
    request.headers().contains_key(AUTHORIZATION)
        || request.uri().path().starts_with(SESSION_PATH_PREFIX)
}

fn has_no_store(headers: &HeaderMap) -> bool {
    headers
        .get_all(CACHE_CONTROL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|directive| directive.trim().eq_ignore_ascii_case(NO_STORE_DIRECTIVE))
}

fn is_loopback_host<B>(request: &Request<B>) -> bool {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| request.uri().host());
    host.is_some_and(|host| {
        let name = strip_port(host).to_ascii_lowercase();
        LOOPBACK_HOSTS.contains(&name.as_str()) || name.ends_with(".localhost")
    })
}

fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        return host
            .find(']')
            .and_then(|end| host.get(..=end))
            .unwrap_or(host);
    }
    host.split_once(':').map_or(host, |(name, _)| name)
}

#[expect(
    clippy::expect_used,
    reason = "`max-age=` followed by ASCII digits is always a valid header value"
)]
fn hsts_header_value(max_age: Duration) -> HeaderValue {
    HeaderValue::from_str(&format!("max-age={}", max_age.as_secs()))
        .expect("max-age with a decimal integer is a valid header value")
}

#[cfg(test)]
mod tests;

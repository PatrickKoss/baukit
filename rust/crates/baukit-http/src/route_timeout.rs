use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    extract::Request,
    response::{IntoResponse, Response},
};
use tower::{Layer, Service};

use crate::ApiError;

/// Bounds one route with its own timeout, inside the global request timeout.
///
/// Apply it to a single route's method router:
/// `get(handler).layer(baukit_http::route_timeout(Duration::from_secs(1)))`.
/// On expiry the handler future is dropped and the standard `504
/// request_timeout` envelope is returned. The global
/// [`HttpOptions::request_timeout`](crate::HttpOptions::request_timeout) still
/// applies, so an override longer than the global timeout has no effect.
#[must_use]
pub const fn route_timeout(timeout: Duration) -> RouteTimeoutLayer {
    RouteTimeoutLayer { timeout }
}

/// The layer returned by [`route_timeout`].
#[derive(Clone, Copy, Debug)]
pub struct RouteTimeoutLayer {
    timeout: Duration,
}

impl<S> Layer<S> for RouteTimeoutLayer {
    type Service = RouteTimeout<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RouteTimeout {
            inner,
            timeout: self.timeout,
        }
    }
}

/// The service produced by [`RouteTimeoutLayer`].
#[derive(Clone, Debug)]
pub struct RouteTimeout<S> {
    inner: S,
    timeout: Duration,
}

type BoxedResponseFuture<E> = Pin<Box<dyn Future<Output = Result<Response, E>> + Send>>;

impl<S> Service<Request> for RouteTimeout<S>
where
    S: Service<Request, Response = Response>,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxedResponseFuture<S::Error>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let response = self.inner.call(request);
        let timeout = self.timeout;
        Box::pin(async move {
            if let Ok(result) = tokio::time::timeout(timeout, response).await {
                result
            } else {
                tracing::warn!("request timed out at its route timeout");
                Ok(ApiError::timeout().into_response())
            }
        })
    }
}

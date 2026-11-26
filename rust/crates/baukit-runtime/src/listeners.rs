use std::{fmt, io};

use axum::Router;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    task::{JoinError, JoinSet},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use crate::{DrainTimeout, ServerOptions, ShutdownToken, server};

/// Identifies one member of an API/operations listener pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListenerKind {
    /// The product API listener.
    Api,
    /// The operations listener.
    Operations,
}

impl fmt::Display for ListenerKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Api => "API",
            Self::Operations => "operations",
        })
    }
}

/// An error returned while running the API and operations listeners.
#[derive(Debug, Error)]
pub enum ListenerPairError {
    /// A listener returned an I/O error.
    #[error("{listener} listener failed: {source}")]
    Listener {
        /// The listener which failed.
        listener: ListenerKind,
        /// The underlying server error.
        #[source]
        source: io::Error,
    },
    /// A listener task panicked or was externally cancelled.
    #[error("listener task failed: {0}")]
    Task(#[source] JoinError),
    /// A listener stopped successfully before shutdown was requested.
    #[error("{0} listener exited before shutdown")]
    UnexpectedExit(ListenerKind),
    /// One or both listeners did not drain before the process deadline.
    #[error(transparent)]
    Drain(#[from] DrainTimeout),
}

type ListenerResult = Result<ListenerKind, (ListenerKind, io::Error)>;

/// Controls when the operations listener stops relative to the public API.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShutdownOrder {
    /// Stops both listeners together when process shutdown begins.
    #[default]
    Together,
    /// Keeps operations serving until the API has drained or the deadline expires.
    OpsOutlivesApi,
}

/// How [`serve_listener_pair_with_options`] runs both listeners.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServeOptions {
    /// When the operations listener stops relative to the API.
    pub shutdown_order: ShutdownOrder,
    /// Connection limits for the API listener.
    pub api: ServerOptions,
    /// Connection limits for the operations listener.
    pub operations: ServerOptions,
}

/// Runs public API and operations routers concurrently with graceful shutdown.
///
/// Both TCP listeners are supplied already bound, keeping addresses and socket
/// policy in the composition root. If either listener fails, process shutdown is
/// triggered and the peer is drained. Once shutdown begins, both Axum servers
/// receive the token through `with_graceful_shutdown`; remaining work is bounded
/// by the token's shared deadline and forcefully aborted on expiry.
///
/// # Errors
///
/// Fails like [`serve_listener_pair_with_shutdown_order`].
pub async fn serve_listener_pair(
    api_listener: TcpListener,
    api_router: Router,
    operations_listener: TcpListener,
    operations_router: Router,
    shutdown: ShutdownToken,
) -> Result<(), ListenerPairError> {
    serve_listener_pair_with_shutdown_order(
        api_listener,
        api_router,
        operations_listener,
        operations_router,
        shutdown,
        ShutdownOrder::Together,
    )
    .await
}

/// Runs an API/operations listener pair with an explicit shutdown order.
///
/// Both listeners use [`ServerOptions::default`]; see
/// [`serve_listener_pair_with_options`].
///
/// [`ShutdownOrder::Together`] is identical to [`serve_listener_pair`]. With
/// [`ShutdownOrder::OpsOutlivesApi`], process shutdown first stops the API from
/// accepting new work and lets its in-flight requests drain. The operations
/// listener continues serving readiness and metrics until the API server exits;
/// it is then gracefully stopped within the same process-wide drain deadline.
/// If that deadline expires first, both listener tasks are aborted.
///
/// # Errors
///
/// Returns [`ListenerPairError::Listener`] when a server fails with an I/O error,
/// [`ListenerPairError::UnexpectedExit`] when one stops before shutdown,
/// [`ListenerPairError::Task`] when a listener task panics, and
/// [`ListenerPairError::Drain`] when the drain deadline expires.
pub async fn serve_listener_pair_with_shutdown_order(
    api_listener: TcpListener,
    api_router: Router,
    operations_listener: TcpListener,
    operations_router: Router,
    shutdown: ShutdownToken,
    shutdown_order: ShutdownOrder,
) -> Result<(), ListenerPairError> {
    serve_listener_pair_with_options(
        api_listener,
        api_router,
        operations_listener,
        operations_router,
        shutdown,
        ServeOptions {
            shutdown_order,
            ..ServeOptions::default()
        },
    )
    .await
}

/// Runs an API/operations listener pair with explicit connection limits.
///
/// Each listener serves through [`serve`](crate::serve) with its
/// [`ServerOptions`]. Shutdown follows [`ServeOptions::shutdown_order`] as
/// described on [`serve_listener_pair_with_shutdown_order`]. The drain ends
/// [`ShutdownToken::flush_reserve`] before the process deadline, so a token
/// built with [`ShutdownToken::with_budget`] keeps that time for
/// [`ShutdownToken::run_flush`].
///
/// # Errors
///
/// Fails like [`serve_listener_pair_with_shutdown_order`].
pub async fn serve_listener_pair_with_options(
    api_listener: TcpListener,
    api_router: Router,
    operations_listener: TcpListener,
    operations_router: Router,
    shutdown: ShutdownToken,
    options: ServeOptions,
) -> Result<(), ListenerPairError> {
    let shutdown_order = options.shutdown_order;
    let mut listeners = JoinSet::new();
    let operations_shutdown = match shutdown_order {
        ShutdownOrder::Together => shutdown.observed.clone(),
        ShutdownOrder::OpsOutlivesApi => CancellationToken::new(),
    };
    spawn_listener(
        &mut listeners,
        ListenerKind::Api,
        api_listener,
        api_router,
        options.api,
        shutdown.observed.clone(),
    );
    spawn_listener(
        &mut listeners,
        ListenerKind::Operations,
        operations_listener,
        operations_router,
        options.operations,
        operations_shutdown.clone(),
    );

    let mut first_error = None;
    while !listeners.is_empty() && !shutdown.is_cancelled() {
        tokio::select! {
            _ = shutdown.cancelled() => {}
            result = listeners.join_next() => {
                stop_operations_after_api(
                    result.as_ref(),
                    shutdown_order,
                    &operations_shutdown,
                );
                record_listener_result(result, &shutdown, &mut first_error);
            }
        }
    }

    if listeners.is_empty() {
        return first_error.map_or(Ok(()), Err);
    }

    let remaining = shutdown.remaining();
    let drained = timeout(remaining, async {
        while let Some(result) = listeners.join_next().await {
            stop_operations_after_api(Some(&result), shutdown_order, &operations_shutdown);
            record_listener_result(Some(result), &shutdown, &mut first_error);
        }
    })
    .await;

    if drained.is_err() {
        listeners.abort_all();
        while listeners.join_next().await.is_some() {}
        return Err(DrainTimeout::new(shutdown.drain_timeout).into());
    }

    first_error.map_or(Ok(()), Err)
}

fn spawn_listener(
    listeners: &mut JoinSet<ListenerResult>,
    kind: ListenerKind,
    listener: TcpListener,
    router: Router,
    options: ServerOptions,
    shutdown: CancellationToken,
) {
    listeners.spawn(async move {
        server::serve(listener, router, options, shutdown)
            .await
            .map(|()| kind)
            .map_err(|error| (kind, error))
    });
}

fn stop_operations_after_api(
    result: Option<&Result<ListenerResult, JoinError>>,
    shutdown_order: ShutdownOrder,
    operations_shutdown: &CancellationToken,
) {
    if shutdown_order != ShutdownOrder::OpsOutlivesApi {
        return;
    }

    let listener = match result {
        Some(Ok(Ok(kind) | Err((kind, _)))) => Some(*kind),
        Some(Err(_)) | None => None,
    };
    if listener == Some(ListenerKind::Api) {
        operations_shutdown.cancel();
    }
}

fn record_listener_result(
    result: Option<Result<ListenerResult, JoinError>>,
    shutdown: &ShutdownToken,
    first_error: &mut Option<ListenerPairError>,
) {
    let error = match result {
        Some(Ok(Ok(kind))) if !shutdown.is_cancelled() => {
            Some(ListenerPairError::UnexpectedExit(kind))
        }
        Some(Ok(Ok(_))) | None => None,
        Some(Ok(Err((listener, source)))) => Some(ListenerPairError::Listener { listener, source }),
        Some(Err(error)) => Some(ListenerPairError::Task(error)),
    };

    if let Some(error) = error {
        tracing::error!(%error, "listener pair initiated process shutdown");
        if first_error.is_none() {
            *first_error = Some(error);
        }
        shutdown.trigger();
    }
}

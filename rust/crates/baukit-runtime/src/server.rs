use std::{
    convert::Infallible,
    fmt,
    future::Future,
    io,
    net::SocketAddr,
    pin::{Pin, pin},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Request},
    http::Response,
};
use hyper::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use thiserror::Error;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{self, Instant},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tower::Service;

const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_MAX_CONNECTIONS: usize = 4_096;
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_secs(1);

/// Connection-level limits for one HTTP listener.
///
/// The defaults are a 10 second header read timeout, a 60 second idle timeout
/// and at most 4096 open connections.
///
/// For HTTP/1.1, hyper also arms the header read timeout while a keep-alive
/// connection waits for its next request, so an idle HTTP/1.1 connection closes
/// after the shorter of the two timeouts. A reverse proxy in front of the
/// listener should drop idle upstream connections before that, or it may reuse
/// a connection the server is closing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerOptions {
    header_read_timeout: Duration,
    idle_timeout: Duration,
    max_connections: usize,
}

impl ServerOptions {
    /// Sets how long a client may take to send a complete request head.
    ///
    /// # Errors
    ///
    /// Returns [`ServerOptionsError::ZeroHeaderReadTimeout`] for zero.
    pub const fn with_header_read_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, ServerOptionsError> {
        if timeout.is_zero() {
            return Err(ServerOptionsError::ZeroHeaderReadTimeout);
        }
        self.header_read_timeout = timeout;
        Ok(self)
    }

    /// Sets how long a connection may stay open with no request in progress.
    ///
    /// When it expires the connection is shut down gracefully, so a request
    /// that arrives at the same moment still completes.
    ///
    /// # Errors
    ///
    /// Returns [`ServerOptionsError::ZeroIdleTimeout`] for zero.
    pub const fn with_idle_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, ServerOptionsError> {
        if timeout.is_zero() {
            return Err(ServerOptionsError::ZeroIdleTimeout);
        }
        self.idle_timeout = timeout;
        Ok(self)
    }

    /// Sets the maximum number of open connections.
    ///
    /// Further connections wait in the kernel accept backlog until one closes.
    ///
    /// # Errors
    ///
    /// Returns [`ServerOptionsError::ZeroMaxConnections`] for zero.
    pub const fn with_max_connections(
        mut self,
        max_connections: usize,
    ) -> Result<Self, ServerOptionsError> {
        if max_connections == 0 {
            return Err(ServerOptionsError::ZeroMaxConnections);
        }
        self.max_connections = max_connections;
        Ok(self)
    }

    /// Returns the header read timeout.
    pub const fn header_read_timeout(&self) -> Duration {
        self.header_read_timeout
    }

    /// Returns the idle connection timeout.
    pub const fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    /// Returns the maximum number of open connections.
    pub const fn max_connections(&self) -> usize {
        self.max_connections
    }
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            header_read_timeout: DEFAULT_HEADER_READ_TIMEOUT,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }
}

/// An invalid [`ServerOptions`] value.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ServerOptionsError {
    /// The header read timeout must be non-zero.
    #[error("header read timeout must be non-zero")]
    ZeroHeaderReadTimeout,
    /// The idle timeout must be non-zero.
    #[error("idle timeout must be non-zero")]
    ZeroIdleTimeout,
    /// The connection cap must be non-zero.
    #[error("max connections must be non-zero")]
    ZeroMaxConnections,
}

/// Serves `router` on `listener` until `shutdown` is cancelled.
///
/// Once `shutdown` fires the listener stops accepting, every open connection
/// is shut down gracefully, and the future completes when the last one has
/// closed, exactly like axum's own graceful shutdown. Bounding
/// that wait is the caller's job.
///
/// # Errors
///
/// Never fails today; the `io::Result` matches `axum::serve`.
pub async fn serve(
    listener: TcpListener,
    router: Router,
    options: ServerOptions,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let connections = TaskTracker::new();
    let slots = Arc::new(Semaphore::new(
        options.max_connections.min(Semaphore::MAX_PERMITS),
    ));
    while let Some((stream, peer, slot)) = accept(&listener, &slots, &shutdown).await {
        let router = router.clone();
        let shutdown = shutdown.clone();
        connections.spawn(serve_connection(
            stream, peer, router, options, shutdown, slot,
        ));
    }
    drop(listener);
    connections.close();
    connections.wait().await;
    Ok(())
}

async fn accept(
    listener: &TcpListener,
    slots: &Arc<Semaphore>,
    shutdown: &CancellationToken,
) -> Option<(TcpStream, SocketAddr, OwnedSemaphorePermit)> {
    loop {
        let slot = tokio::select! {
            () = shutdown.cancelled() => return None,
            slot = Arc::clone(slots).acquire_owned() => slot.ok()?,
        };
        let accepted = tokio::select! {
            () = shutdown.cancelled() => return None,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, peer)) => return Some((stream, peer, slot)),
            Err(error) => handle_accept_error(&error).await,
        }
    }
}

async fn handle_accept_error(error: &io::Error) {
    if matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    ) {
        return;
    }
    tracing::error!(%error, "accept error");
    time::sleep(ACCEPT_ERROR_BACKOFF).await;
}

async fn serve_connection(
    stream: TcpStream,
    peer: SocketAddr,
    router: Router,
    options: ServerOptions,
    shutdown: CancellationToken,
    _slot: OwnedSemaphorePermit,
) {
    let activity = Arc::new(ConnectionActivity::new());
    let service = TowerToHyperService::new(TrackedRouter {
        router,
        peer,
        activity: Arc::clone(&activity),
    });
    let mut builder = Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(options.header_read_timeout);
    builder
        .http2()
        .timer(TokioTimer::new())
        .enable_connect_protocol();

    let mut connection =
        pin!(builder.serve_connection_with_upgrades(TokioIo::new(stream), service));
    let mut closing = false;
    loop {
        tokio::select! {
            result = connection.as_mut() => {
                if let Err(error) = result {
                    tracing::trace!(%error, "connection ended with an error");
                }
                return;
            }
            () = shutdown.cancelled(), if !closing => {
                connection.as_mut().graceful_shutdown();
                closing = true;
            }
            () = activity.idle_for(options.idle_timeout), if !closing => {
                connection.as_mut().graceful_shutdown();
                closing = true;
            }
        }
    }
}

struct ConnectionActivity {
    active: AtomicUsize,
    last_finished: Mutex<Instant>,
}

impl ConnectionActivity {
    fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            last_finished: Mutex::new(Instant::now()),
        }
    }

    fn begin(self: &Arc<Self>) -> RequestActivity {
        self.active.fetch_add(1, Ordering::AcqRel);
        RequestActivity(Arc::clone(self))
    }

    fn idle_deadline(&self, idle_timeout: Duration) -> Option<Instant> {
        if self.active.load(Ordering::Acquire) > 0 {
            return None;
        }
        let last_finished = *self
            .last_finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Some(last_finished + idle_timeout)
    }

    async fn idle_for(&self, idle_timeout: Duration) {
        loop {
            match self.idle_deadline(idle_timeout) {
                Some(deadline) if deadline <= Instant::now() => return,
                Some(deadline) => time::sleep_until(deadline).await,
                None => time::sleep(idle_timeout).await,
            }
        }
    }
}

struct RequestActivity(Arc<ConnectionActivity>);

impl Drop for RequestActivity {
    fn drop(&mut self) {
        *self
            .0
            .last_finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
struct TrackedRouter {
    router: Router,
    peer: SocketAddr,
    activity: Arc<ConnectionActivity>,
}

impl fmt::Debug for TrackedRouter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrackedRouter")
            .finish_non_exhaustive()
    }
}

type RouterFuture = Pin<Box<dyn Future<Output = Result<Response<Body>, Infallible>> + Send>>;

impl Service<Request<Incoming>> for TrackedRouter {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = RouterFuture;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Service::<Request>::poll_ready(&mut self.router, context)
    }

    fn call(&mut self, request: Request<Incoming>) -> Self::Future {
        let activity = self.activity.begin();
        let mut request = request.map(Body::new);
        request.extensions_mut().insert(ConnectInfo(self.peer));
        let response = self.router.call(request);
        Box::pin(async move {
            let response = response.await;
            drop(activity);
            response
        })
    }
}

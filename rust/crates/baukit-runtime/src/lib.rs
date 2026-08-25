//! Process-wide lifecycle and listener composition for Baukit services.
//!
//! This crate owns mechanics shared by an entire process: shutdown signaling,
//! bounded draining, background-task supervision, build identity, and the thin
//! composition of public and operations listeners. Products continue to own
//! configuration, routes, and application state.
//!
//! # Composition example
//!
//! A composition root can wire the crate to `baukit-config` and
//! `baukit-telemetry` without making runtime own either product configuration or
//! telemetry policy:
//!
//! ```no_run
//! use std::sync::Arc;
//! use axum::Router;
//! use baukit_runtime::{
//!     ProcessKind, RestartPolicy, ServiceInfo, ShutdownToken, TaskSupervisor,
//!     ShutdownOrder, build_info, serve_listener_pair_with_shutdown_order,
//! };
//! use baukit_telemetry::{DeploymentEnvironment, TelemetryBuilder};
//! use tokio::net::TcpListener;
//! # use std::time::Duration;
//! # async fn run_outbox_until(_: ShutdownToken) {}
//! # fn product_routes() -> Router { Router::new() }
//! # fn operations_routes() -> Router { Router::new() }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let service = ServiceInfo::new(
//!         "orders",
//!         ProcessKind::Api,
//!         build_info!(),
//!         DeploymentEnvironment::Local,
//!     );
//!     let telemetry = Arc::new(
//!         TelemetryBuilder::new(service.telemetry_identity().clone()).init()?,
//!     );
//!     let shutdown = ShutdownToken::new(Duration::from_secs(30));
//!     let signal_task = shutdown.spawn_signal_listener();
//!
//!     let mut workers = TaskSupervisor::new(shutdown.clone());
//!     let worker_shutdown = shutdown.child_token();
//!     workers.spawn("outbox", RestartPolicy::Restart { max_restarts: 3 }, move || {
//!         let worker_shutdown = worker_shutdown.clone();
//!         async move { run_outbox_until(worker_shutdown).await }
//!     });
//!
//!     let api = TcpListener::bind("127.0.0.1:0").await?;
//!     let ops = TcpListener::bind("127.0.0.1:0").await?;
//!     let listener_result = serve_listener_pair_with_shutdown_order(
//!         api,
//!         Router::new().merge(product_routes()),
//!         ops,
//!         Router::new().merge(operations_routes()),
//!         shutdown.clone(),
//!         ShutdownOrder::OpsOutlivesApi,
//!     ).await;
//!     shutdown.trigger();
//!     let _ = workers.join().await;
//!
//!     // Telemetry shutdown is synchronous, so keep it off the async executor.
//!     // `run_during_drain` still bounds it by the process-wide drain deadline.
//!     let telemetry_for_flush = Arc::clone(&telemetry);
//!     shutdown.run_during_drain(async move {
//!         tokio::task::spawn_blocking(move || telemetry_for_flush.shutdown()).await
//!     }).await???;
//!
//!     signal_task.abort();
//!     listener_result?;
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]

use std::{
    fmt,
    future::Future,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use axum::Router;
pub use baukit_core::{BuildInfo, DeploymentEnvironment, ProcessKind, ServiceIdentity};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    task::{JoinError, JoinHandle, JoinSet},
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

/// The Rust compiler version used to build this crate.
///
/// [`build_info!`] combines this with package metadata captured at its call site.
pub const RUST_VERSION: &str = env!("BAUKIT_RUST_VERSION");

/// Captures build metadata from the binary crate in which the macro is invoked.
///
/// `GIT_COMMIT` should be set by the build or release pipeline. It falls back to
/// `"unknown"` for local builds. The Rust compiler version is recorded by
/// `baukit-runtime`'s build script.
#[macro_export]
macro_rules! build_info {
    () => {
        $crate::BuildInfo::new(
            env!("CARGO_PKG_VERSION"),
            option_env!("GIT_COMMIT").unwrap_or("unknown"),
            $crate::RUST_VERSION,
        )
    };
}

/// Canonical service identity plus its complete process build metadata.
///
/// The service name follows the telemetry convention `<product>-<process>`.
/// Keeping the canonical [`ServiceIdentity`] inside this value ensures runtime,
/// logging, metrics, and traces cannot disagree about the process identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceInfo {
    name: String,
    identity: ServiceIdentity,
    build: BuildInfo,
}

impl ServiceInfo {
    /// Creates service information using Baukit's canonical service-name format.
    pub fn new(
        product: impl Into<String>,
        process: ProcessKind,
        build: BuildInfo,
        environment: DeploymentEnvironment,
    ) -> Self {
        let product = product.into();
        let identity = ServiceIdentity::new(
            product,
            process,
            build.version().to_owned(),
            build.commit().to_owned(),
            environment,
        );
        let name = identity.service_name();
        Self {
            name,
            identity,
            build,
        }
    }

    /// Returns the canonical `<product>-<process>` service name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the stable product identifier.
    pub fn product(&self) -> &str {
        self.identity.product()
    }

    /// Returns this process's kind.
    pub const fn process(&self) -> ProcessKind {
        self.identity.process()
    }

    /// Returns the service's build metadata.
    pub const fn build(&self) -> &BuildInfo {
        &self.build
    }

    /// Returns the identity accepted directly by `baukit-telemetry`.
    pub const fn telemetry_identity(&self) -> &ServiceIdentity {
        &self.identity
    }
}

struct ShutdownState {
    root: CancellationToken,
    started_at: OnceLock<Instant>,
    drain_hooks: Mutex<Vec<DrainHook>>,
}

type DrainHook = Box<dyn FnOnce() + Send + 'static>;

impl fmt::Debug for ShutdownState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hook_count = self
            .drain_hooks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        formatter
            .debug_struct("ShutdownState")
            .field("root", &self.root)
            .field("started_at", &self.started_at)
            .field("drain_hook_count", &hook_count)
            .finish()
    }
}

/// A cloneable, awaitable process-shutdown handle with one shared drain deadline.
///
/// Child tokens observe parent shutdown, while calling [`ShutdownToken::trigger`]
/// on any child still initiates process-wide shutdown. The deadline begins with
/// the first trigger and is shared by all clones and children.
#[derive(Clone, Debug)]
pub struct ShutdownToken {
    observed: CancellationToken,
    state: Arc<ShutdownState>,
    drain_timeout: Duration,
}

/// A concise alias for [`ShutdownToken`].
pub type Shutdown = ShutdownToken;

impl ShutdownToken {
    /// Creates a shutdown handle with the maximum duration allowed for draining.
    pub fn new(drain_timeout: Duration) -> Self {
        let root = CancellationToken::new();
        Self {
            observed: root.clone(),
            state: Arc::new(ShutdownState {
                root,
                started_at: OnceLock::new(),
                drain_hooks: Mutex::new(Vec::new()),
            }),
            drain_timeout,
        }
    }

    /// Returns a child which is cancelled by this token.
    ///
    /// Triggering the child deliberately triggers the shared process token too.
    pub fn child_token(&self) -> Self {
        Self {
            observed: self.observed.child_token(),
            state: Arc::clone(&self.state),
            drain_timeout: self.drain_timeout,
        }
    }

    /// Initiates process-wide shutdown and returns whether this was the first trigger.
    pub fn trigger(&self) -> bool {
        let first = self.state.started_at.set(Instant::now()).is_ok();
        if first {
            let hooks = std::mem::take(
                &mut *self
                    .state
                    .drain_hooks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for hook in hooks {
                if catch_unwind(AssertUnwindSafe(hook)).is_err() {
                    tracing::error!("a shutdown drain hook panicked");
                }
            }
            self.state.root.cancel();
        }
        first
    }

    /// Registers a fast synchronous action to run when drain begins.
    ///
    /// Hooks run exactly once on the first [`ShutdownToken::trigger`], before
    /// cancellation wakes asynchronous drain work. A hook registered after
    /// drain has started runs immediately. This is the dependency-neutral
    /// composition seam for closing `baukit_ops::TrafficGate` without a watcher
    /// task:
    ///
    /// ```no_run
    /// # use std::time::Duration;
    /// # use baukit_runtime::ShutdownToken;
    /// # #[derive(Clone)] struct TrafficGate;
    /// # impl TrafficGate { fn stop_accepting(&self) {} }
    /// # let traffic_gate = TrafficGate;
    /// let shutdown = ShutdownToken::new(Duration::from_secs(30));
    /// shutdown.on_drain({
    ///     let traffic_gate = traffic_gate.clone();
    ///     move || traffic_gate.stop_accepting()
    /// });
    /// ```
    ///
    /// Hooks should only flip in-memory state or perform similarly bounded work;
    /// asynchronous cleanup belongs in [`ShutdownToken::run_during_drain`].
    pub fn on_drain<F>(&self, hook: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let mut hook = Some(Box::new(hook) as DrainHook);
        {
            let mut hooks = self
                .state
                .drain_hooks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.state.started_at.get().is_none() {
                hooks.push(hook.take().expect("drain hook is available"));
            }
        }
        if let Some(hook) = hook
            && catch_unwind(AssertUnwindSafe(hook)).is_err()
        {
            tracing::error!("a late shutdown drain hook panicked");
        }
    }

    /// Returns whether this token has observed shutdown.
    pub fn is_cancelled(&self) -> bool {
        self.observed.is_cancelled()
    }

    /// Waits until process-wide shutdown reaches this token.
    pub async fn cancelled(&self) {
        self.observed.cancelled().await;
    }

    /// Returns the configured drain timeout.
    pub const fn drain_timeout(&self) -> Duration {
        self.drain_timeout
    }

    /// Returns the shared absolute drain deadline after shutdown has begun.
    pub fn deadline(&self) -> Option<Instant> {
        self.state
            .started_at
            .get()
            .map(|started_at| *started_at + self.drain_timeout)
    }

    /// Waits for shutdown, then runs `future` within the remaining drain budget.
    ///
    /// Every call uses the same absolute deadline, so sequential cleanup steps
    /// cannot accidentally receive a fresh timeout. On expiry, the future is
    /// dropped and [`DrainTimeout`] is returned.
    pub async fn run_during_drain<F>(&self, future: F) -> Result<F::Output, DrainTimeout>
    where
        F: Future,
    {
        self.cancelled().await;
        let remaining = self.remaining();
        timeout(remaining, future)
            .await
            .map_err(|_| DrainTimeout::new(self.drain_timeout))
    }

    /// Waits for SIGINT/SIGTERM (or the platform Ctrl-C event), then triggers shutdown.
    pub async fn shutdown_on_signal(&self) -> io::Result<ShutdownSignal> {
        let signal = wait_for_shutdown_signal().await?;
        self.trigger();
        Ok(signal)
    }

    /// Spawns a Tokio task which triggers shutdown on the first process signal.
    ///
    /// The returned task reports signal-handler setup failures when joined.
    #[must_use = "retain or join the signal listener so setup failures are observed"]
    pub fn spawn_signal_listener(&self) -> tokio::task::JoinHandle<io::Result<ShutdownSignal>> {
        let shutdown = self.clone();
        tokio::spawn(async move { shutdown.shutdown_on_signal().await })
    }

    fn remaining(&self) -> Duration {
        self.deadline()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(self.drain_timeout)
    }
}

/// The operating-system event which initiated shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownSignal {
    /// Ctrl-C or SIGINT was received.
    Interrupt,
    /// SIGTERM was received on Unix.
    Terminate,
}

/// Waits for the first supported process shutdown signal without triggering a token.
///
/// Factoring signal observation from [`ShutdownToken::trigger`] keeps lifecycle
/// behavior directly testable without delivering real process signals.
#[cfg(unix)]
pub async fn wait_for_shutdown_signal() -> io::Result<ShutdownSignal> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        ctrl_c = tokio::signal::ctrl_c() => {
            ctrl_c?;
            Ok(ShutdownSignal::Interrupt)
        }
        received = terminate.recv() => received.map_or_else(
            || Err(io::Error::other("SIGTERM signal stream ended")),
            |_| Ok(ShutdownSignal::Terminate),
        ),
    }
}

/// Waits for the platform Ctrl-C event without triggering a token.
#[cfg(not(unix))]
pub async fn wait_for_shutdown_signal() -> io::Result<ShutdownSignal> {
    tokio::signal::ctrl_c().await?;
    Ok(ShutdownSignal::Interrupt)
}

/// Indicates that process draining exceeded its shared deadline.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("shutdown drain deadline expired after {timeout:?}")]
pub struct DrainTimeout {
    timeout: Duration,
}

impl DrainTimeout {
    const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Returns the configured total drain timeout.
    pub const fn timeout(self) -> Duration {
        self.timeout
    }
}

/// The response to a supervised task's unexpected termination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartPolicy {
    /// Trigger process shutdown immediately.
    FailProcess,
    /// Restart up to `max_restarts` times, then trigger process shutdown.
    Restart {
        /// Maximum starts after the initial attempt.
        max_restarts: usize,
    },
}

/// Owns named background tasks tied to one [`ShutdownToken`].
pub struct TaskSupervisor {
    shutdown: ShutdownToken,
    tasks: JoinSet<()>,
}

impl TaskSupervisor {
    /// Creates an empty task supervisor.
    pub fn new(shutdown: ShutdownToken) -> Self {
        Self {
            shutdown,
            tasks: JoinSet::new(),
        }
    }

    /// Spawns a named task from a restartable future factory.
    ///
    /// Both a normal return and a panic are unexpected before shutdown. The
    /// selected `policy` either restarts the factory or triggers process shutdown.
    pub fn spawn<F, Fut>(&mut self, name: impl Into<String>, policy: RestartPolicy, factory: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let shutdown = self.shutdown.clone();
        let name = name.into();
        let factory = Arc::new(factory);
        self.tasks.spawn(async move {
            supervise_task(name, policy, factory, shutdown).await;
        });
    }

    /// Waits for shutdown and joins all supervised tasks within its drain deadline.
    ///
    /// Any tasks still alive when the deadline expires are aborted before this
    /// method returns [`DrainTimeout`].
    pub async fn join(mut self) -> Result<(), DrainTimeout> {
        self.shutdown.cancelled().await;
        let remaining = self.shutdown.remaining();
        let joined = timeout(remaining, async {
            while let Some(result) = self.tasks.join_next().await {
                if let Err(error) = result {
                    tracing::error!(%error, "task supervisor failed while joining a worker");
                }
            }
        })
        .await;

        if joined.is_err() {
            self.tasks.abort_all();
            while self.tasks.join_next().await.is_some() {}
            return Err(DrainTimeout::new(self.shutdown.drain_timeout));
        }

        Ok(())
    }

    /// Returns the number of supervised task loops currently owned.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Returns whether no supervised tasks are currently owned.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

async fn supervise_task<F, Fut>(
    name: String,
    policy: RestartPolicy,
    factory: Arc<F>,
    shutdown: ShutdownToken,
) where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut restarts = 0_usize;
    loop {
        let mut attempt = AbortOnDropTask(tokio::spawn(factory()));
        let outcome = tokio::select! {
            _ = shutdown.cancelled() => {
                if let Err(error) = (&mut attempt.0).await
                    && error.is_panic()
                {
                    tracing::error!(task = %name, %error, "supervised task panicked while draining");
                }
                return;
            }
            result = &mut attempt.0 => result,
        };

        if shutdown.is_cancelled() {
            return;
        }

        match &outcome {
            Ok(()) => tracing::error!(task = %name, "supervised task exited unexpectedly"),
            Err(error) if error.is_panic() => {
                tracing::error!(task = %name, %error, "supervised task panicked")
            }
            Err(error) => tracing::error!(task = %name, %error, "supervised task was cancelled"),
        }

        match policy {
            RestartPolicy::FailProcess => {
                shutdown.trigger();
                return;
            }
            RestartPolicy::Restart { max_restarts } if restarts < max_restarts => {
                restarts += 1;
                tracing::warn!(task = %name, restarts, max_restarts, "restarting supervised task");
            }
            RestartPolicy::Restart { max_restarts } => {
                tracing::error!(task = %name, restarts, max_restarts, "task restart limit exhausted");
                shutdown.trigger();
                return;
            }
        }
    }
}

struct AbortOnDropTask<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDropTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

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

/// Runs public API and operations routers concurrently with graceful shutdown.
///
/// Both TCP listeners are supplied already bound, keeping addresses and socket
/// policy in the composition root. If either listener fails, process shutdown is
/// triggered and the peer is drained. Once shutdown begins, both Axum servers
/// receive the token through `with_graceful_shutdown`; remaining work is bounded
/// by the token's shared deadline and forcefully aborted on expiry.
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
/// [`ShutdownOrder::Together`] is identical to [`serve_listener_pair`]. With
/// [`ShutdownOrder::OpsOutlivesApi`], process shutdown first stops the API from
/// accepting new work and lets its in-flight requests drain. The operations
/// listener continues serving readiness and metrics until the API server exits;
/// it is then gracefully stopped within the same process-wide drain deadline.
/// If that deadline expires first, both listener tasks are aborted.
pub async fn serve_listener_pair_with_shutdown_order(
    api_listener: TcpListener,
    api_router: Router,
    operations_listener: TcpListener,
    operations_router: Router,
    shutdown: ShutdownToken,
    shutdown_order: ShutdownOrder,
) -> Result<(), ListenerPairError> {
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
        shutdown.observed.clone(),
    );
    spawn_listener(
        &mut listeners,
        ListenerKind::Operations,
        operations_listener,
        operations_router,
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
    shutdown: CancellationToken,
) {
    listeners.spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
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

#[cfg(test)]
mod tests {
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
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
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
}

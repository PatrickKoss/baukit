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

mod budget;
mod listeners;
mod server;

pub use budget::{DEFAULT_FLUSH_RESERVE, DrainBudget, DrainBudgetError};
pub use listeners::{
    ListenerKind, ListenerPairError, ServeOptions, ShutdownOrder, serve_listener_pair,
    serve_listener_pair_with_options, serve_listener_pair_with_shutdown_order,
};
pub use server::{ServerOptions, ServerOptionsError, serve};

use std::{
    fmt,
    future::Future,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

pub use baukit_core::{BuildInfo, DeploymentEnvironment, ProcessKind, ServiceIdentity};
use thiserror::Error;
use tokio::{
    task::{JoinHandle, JoinSet},
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
    pub(crate) observed: CancellationToken,
    state: Arc<ShutdownState>,
    drain_timeout: Duration,
    flush_reserve: Duration,
}

impl ShutdownToken {
    /// Creates a shutdown handle with the maximum duration allowed for draining.
    ///
    /// It keeps no flush reserve. Services that flush telemetry at the end of
    /// draining should use [`ShutdownToken::with_budget`].
    pub fn new(drain_timeout: Duration) -> Self {
        Self::build(drain_timeout, Duration::ZERO)
    }

    /// Creates a shutdown handle from a validated [`DrainBudget`].
    ///
    /// Listener draining, [`TaskSupervisor::join`] and
    /// [`ShutdownToken::run_during_drain`] end `flush_reserve` before the drain
    /// deadline; [`ShutdownToken::run_flush`] may run until the deadline itself.
    pub fn with_budget(budget: DrainBudget) -> Self {
        Self::build(budget.drain_timeout(), budget.flush_reserve())
    }

    fn build(drain_timeout: Duration, flush_reserve: Duration) -> Self {
        let root = CancellationToken::new();
        Self {
            observed: root.clone(),
            state: Arc::new(ShutdownState {
                root,
                started_at: OnceLock::new(),
                drain_hooks: Mutex::new(Vec::new()),
            }),
            drain_timeout,
            flush_reserve,
        }
    }

    /// Returns a child which is cancelled by this token.
    ///
    /// Triggering the child deliberately triggers the shared process token too.
    #[must_use]
    pub fn child_token(&self) -> Self {
        Self {
            observed: self.observed.child_token(),
            state: Arc::clone(&self.state),
            drain_timeout: self.drain_timeout,
            flush_reserve: self.flush_reserve,
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
        let hook: DrainHook = Box::new(hook);
        let late_hook = {
            let mut hooks = self
                .state
                .drain_hooks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.state.started_at.get().is_none() {
                hooks.push(hook);
                None
            } else {
                Some(hook)
            }
        };
        if let Some(hook) = late_hook
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

    /// Returns the part of the drain timeout only [`ShutdownToken::run_flush`] may use.
    pub const fn flush_reserve(&self) -> Duration {
        self.flush_reserve
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
    /// cannot accidentally receive a fresh timeout. That deadline ends the
    /// [`ShutdownToken::flush_reserve`] before the drain deadline. On expiry,
    /// the future is dropped and [`DrainTimeout`] is returned.
    ///
    /// # Errors
    ///
    /// Returns [`DrainTimeout`] when `future` does not finish before the deadline.
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

    /// Waits for shutdown, then runs the final telemetry flush until the drain deadline.
    ///
    /// Unlike [`ShutdownToken::run_during_drain`] it may use the
    /// [`ShutdownToken::flush_reserve`], so it still gets that time after a
    /// listener drain that ran into its own deadline. Call it once, last.
    ///
    /// # Errors
    ///
    /// Returns [`DrainTimeout`] when `future` does not finish before the deadline.
    pub async fn run_flush<F>(&self, future: F) -> Result<F::Output, DrainTimeout>
    where
        F: Future,
    {
        self.cancelled().await;
        let remaining = self.deadline().map_or(self.drain_timeout, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        timeout(remaining, future)
            .await
            .map_err(|_| DrainTimeout::new(self.drain_timeout))
    }

    /// Waits for SIGINT/SIGTERM (or the platform Ctrl-C event), then triggers shutdown.
    ///
    /// # Errors
    ///
    /// Fails like [`wait_for_shutdown_signal`].
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

    pub(crate) fn remaining(&self) -> Duration {
        let work_budget = self.drain_timeout.saturating_sub(self.flush_reserve);
        self.state
            .started_at
            .get()
            .map_or(work_budget, |started_at| {
                (*started_at + work_budget).saturating_duration_since(Instant::now())
            })
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
///
/// # Errors
///
/// Returns an [`io::Error`] when the SIGTERM or Ctrl-C handler cannot be
/// installed, or when the SIGTERM stream ends without a signal.
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
///
/// # Errors
///
/// Returns an [`io::Error`] when the Ctrl-C handler cannot be installed.
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
    pub(crate) const fn new(timeout: Duration) -> Self {
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
    ///
    /// # Errors
    ///
    /// Returns [`DrainTimeout`] when tasks are still running at the drain deadline.
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

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_serving;

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;

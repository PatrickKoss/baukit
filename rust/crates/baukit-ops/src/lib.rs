//! Private operations endpoints for Baukit services.
//!
//! [`OpsRouter`] assembles liveness, concurrent dependency-aware readiness,
//! Prometheus exposition, and build-information endpoints. Bind the resulting
//! router to a separate operations listener; it has no authentication or CORS
//! policy and must never be mounted on public ingress.
//!
//! # Endpoints
//!
//! - `GET /healthz` returns process liveness.
//! - `GET /readyz` runs every registered gating and diagnostic check
//!   concurrently and also evaluates the manual [`TrafficGate`]. Diagnostic
//!   failures are reported but never make the process unready.
//! - `GET /metrics` renders the [`PrometheusHandle`] created by
//!   `baukit-telemetry`.
//! - `GET /buildinfo` returns service name, version, commit, and Rust version.
//!
//! # PostgreSQL acquisition instrumentation
//!
//! With the `sqlx-postgres` feature, use `baukit_ops::acquire` instead of
//! `PgPool::acquire`, and pass `&mut *connection` to queries that would otherwise
//! execute directly against `&PgPool`. Use `baukit_ops::begin` instead of
//! `PgPool::begin`. These patterns record pool wait duration and timeouts. SQLx
//! exposes no pool hook that can observe implicit `&PgPool` executor
//! acquisitions, so queries passed a raw pool reference bypass the acquisition
//! metrics; the periodic pool gauges remain unaffected.
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use baukit_ops::{OpsRouter, ReadinessError, ReadinessRegistry, TrafficGate};
//! use baukit_telemetry::{
//!     DeploymentEnvironment, ProcessKind, ServiceIdentity, TelemetryBuilder,
//! };
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let identity = ServiceIdentity::new(
//!     "orders",
//!     ProcessKind::Api,
//!     env!("CARGO_PKG_VERSION"),
//!     option_env!("GIT_COMMIT").unwrap_or("unknown"),
//!     DeploymentEnvironment::Local,
//! );
//! let telemetry = TelemetryBuilder::new(identity.clone()).init()?;
//! let readiness = ReadinessRegistry::new();
//! readiness.register_fn("database", Duration::from_secs(2), || async {
//!     // Ping the database here. Error text must be safe for an ops response.
//!     Ok::<_, ReadinessError>(())
//! })?;
//! readiness.register_diagnostic_fn_default("upstream_latency", || async {
//!     // Observability-only checks are reported without gating traffic.
//!     Ok::<_, ReadinessError>(())
//! })?;
//! let traffic = TrafficGate::new();
//! let app = OpsRouter::new(identity, telemetry.prometheus_handle().clone())
//!     .with_readiness(readiness)
//!     .with_traffic_gate(traffic.clone())
//!     .into_router();
//!
//! // The composition root binds `app` on its private operations listener.
//! # let _ = app;
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]

use std::{
    collections::HashMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
pub use baukit_core::ServiceIdentity;
pub use baukit_telemetry::PrometheusHandle;
use serde::Serialize;
use tokio::task::JoinSet;

#[cfg(feature = "sqlx-postgres")]
mod postgres;

#[cfg(feature = "sqlx-postgres")]
pub use postgres::{
    PoolMetricsSampler, PoolMetricsSamplerError, acquire, begin, spawn_pool_metrics_sampler,
};

/// Default timeout applied by [`ReadinessRegistry::register_default`].
pub const DEFAULT_READINESS_TIMEOUT: Duration = Duration::from_secs(2);

const MAX_ERROR_LENGTH: usize = 256;
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";
const RUST_VERSION: &str = env!("BAUKIT_RUST_VERSION");

/// The boxed future returned by a [`ReadinessCheck`].
pub type ReadinessFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ReadinessError>> + Send + 'a>>;

/// An asynchronous dependency check used by the readiness endpoint.
///
/// Implementations should make a lightweight call that establishes whether the
/// dependency is currently usable. Error messages are returned by `/readyz`, so
/// construct them from static, public-safe text rather than secrets or provider
/// payloads.
pub trait ReadinessCheck: Send + Sync + 'static {
    /// Checks whether the dependency is ready for use.
    fn check(&self) -> ReadinessFuture<'_>;
}

/// A public-safe readiness failure.
///
/// Construction removes control characters, normalizes whitespace, and bounds
/// output length. It cannot infer which application-specific values are secret;
/// callers must provide messages that are safe to expose on the private ops
/// listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadinessError(String);

impl ReadinessError {
    /// Creates a sanitized readiness error.
    pub fn new(message: impl AsRef<str>) -> Self {
        Self(sanitize_error(message.as_ref()))
    }

    /// Returns the sanitized public message.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ReadinessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ReadinessError {}

struct ClosureCheck<F>(F);

impl<F, Fut> ReadinessCheck for ClosureCheck<F>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), ReadinessError>> + Send + 'static,
{
    fn check(&self) -> ReadinessFuture<'_> {
        Box::pin((self.0)())
    }
}

#[derive(Clone)]
struct RegisteredCheck {
    name: String,
    timeout: Duration,
    check: Arc<dyn ReadinessCheck>,
    gating: bool,
}

/// A cloneable, thread-safe registry of named readiness checks.
///
/// Clones share registrations, so checks may be registered through a retained
/// handle after the Axum router has been built.
#[derive(Clone, Default)]
pub struct ReadinessRegistry {
    checks: Arc<RwLock<Vec<RegisteredCheck>>>,
}

impl ReadinessRegistry {
    /// Creates an empty readiness registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `check` with an explicit timeout.
    ///
    /// Names must be non-empty and unique. A zero timeout is rejected.
    pub fn register<C>(
        &self,
        name: impl Into<String>,
        timeout: Duration,
        check: C,
    ) -> Result<(), RegistrationError>
    where
        C: ReadinessCheck,
    {
        self.register_arc(name.into(), timeout, Arc::new(check), true)
    }

    /// Registers `check` with [`DEFAULT_READINESS_TIMEOUT`].
    pub fn register_default<C>(
        &self,
        name: impl Into<String>,
        check: C,
    ) -> Result<(), RegistrationError>
    where
        C: ReadinessCheck,
    {
        self.register(name, DEFAULT_READINESS_TIMEOUT, check)
    }

    /// Registers an async closure with an explicit timeout.
    pub fn register_fn<F, Fut>(
        &self,
        name: impl Into<String>,
        timeout: Duration,
        check: F,
    ) -> Result<(), RegistrationError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ReadinessError>> + Send + 'static,
    {
        self.register(name, timeout, ClosureCheck(check))
    }

    /// Registers an async closure with [`DEFAULT_READINESS_TIMEOUT`].
    pub fn register_fn_default<F, Fut>(
        &self,
        name: impl Into<String>,
        check: F,
    ) -> Result<(), RegistrationError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ReadinessError>> + Send + 'static,
    {
        self.register_fn(name, DEFAULT_READINESS_TIMEOUT, check)
    }

    /// Registers a non-gating diagnostic `check` with an explicit timeout.
    ///
    /// Diagnostic results appear in `/readyz`, but failures and timeouts never
    /// affect aggregate readiness or its HTTP status.
    pub fn register_diagnostic<C>(
        &self,
        name: impl Into<String>,
        timeout: Duration,
        check: C,
    ) -> Result<(), RegistrationError>
    where
        C: ReadinessCheck,
    {
        self.register_arc(name.into(), timeout, Arc::new(check), false)
    }

    /// Registers a non-gating diagnostic `check` with [`DEFAULT_READINESS_TIMEOUT`].
    pub fn register_diagnostic_default<C>(
        &self,
        name: impl Into<String>,
        check: C,
    ) -> Result<(), RegistrationError>
    where
        C: ReadinessCheck,
    {
        self.register_diagnostic(name, DEFAULT_READINESS_TIMEOUT, check)
    }

    /// Registers a non-gating diagnostic async closure with an explicit timeout.
    pub fn register_diagnostic_fn<F, Fut>(
        &self,
        name: impl Into<String>,
        timeout: Duration,
        check: F,
    ) -> Result<(), RegistrationError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ReadinessError>> + Send + 'static,
    {
        self.register_diagnostic(name, timeout, ClosureCheck(check))
    }

    /// Registers a non-gating diagnostic async closure with [`DEFAULT_READINESS_TIMEOUT`].
    pub fn register_diagnostic_fn_default<F, Fut>(
        &self,
        name: impl Into<String>,
        check: F,
    ) -> Result<(), RegistrationError>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ReadinessError>> + Send + 'static,
    {
        self.register_diagnostic_fn(name, DEFAULT_READINESS_TIMEOUT, check)
    }

    /// Returns the current number of registered dependency checks.
    pub fn len(&self) -> usize {
        self.checks
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Returns whether no dependency checks are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn register_arc(
        &self,
        name: String,
        timeout: Duration,
        check: Arc<dyn ReadinessCheck>,
        gating: bool,
    ) -> Result<(), RegistrationError> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(RegistrationError::EmptyName);
        }
        if name == "accepting_traffic" {
            return Err(RegistrationError::ReservedName(name));
        }
        if timeout.is_zero() {
            return Err(RegistrationError::ZeroTimeout(name));
        }

        let mut checks = self
            .checks
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if checks.iter().any(|registered| registered.name == name) {
            return Err(RegistrationError::DuplicateName(name));
        }
        checks.push(RegisteredCheck {
            name,
            timeout,
            check,
            gating,
        });
        Ok(())
    }

    fn snapshot(&self) -> Vec<RegisteredCheck> {
        self.checks
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// An invalid readiness-check registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistrationError {
    /// The supplied name was empty or only whitespace.
    EmptyName,
    /// The name is reserved for the manual traffic gate.
    ReservedName(String),
    /// A check with the name is already registered.
    DuplicateName(String),
    /// The check timeout was zero.
    ZeroTimeout(String),
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("readiness check name must not be empty"),
            Self::ReservedName(name) => {
                write!(formatter, "readiness check name `{name}` is reserved")
            }
            Self::DuplicateName(name) => {
                write!(formatter, "readiness check `{name}` is already registered")
            }
            Self::ZeroTimeout(name) => {
                write!(
                    formatter,
                    "readiness check `{name}` must have a non-zero timeout"
                )
            }
        }
    }
}

impl std::error::Error for RegistrationError {}

/// A cloneable manual gate controlling whether the process accepts new traffic.
///
/// The gate starts open. Call [`TrafficGate::stop_accepting`] before graceful
/// drain begins so load balancers observe readiness failure before shutdown.
#[derive(Clone, Debug)]
pub struct TrafficGate {
    accepting: Arc<AtomicBool>,
}

impl Default for TrafficGate {
    fn default() -> Self {
        Self::new()
    }
}

impl TrafficGate {
    /// Creates an open traffic gate.
    pub fn new() -> Self {
        Self {
            accepting: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Returns whether the process currently accepts new traffic.
    pub fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
    }

    /// Sets whether the process accepts new traffic.
    pub fn set_accepting(&self, accepting: bool) {
        self.accepting.store(accepting, Ordering::Release);
    }

    /// Closes the gate before graceful drain and shutdown.
    pub fn stop_accepting(&self) {
        self.set_accepting(false);
    }

    /// Reopens the gate.
    pub fn start_accepting(&self) {
        self.set_accepting(true);
    }
}

/// JSON returned by `/healthz`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct HealthResponse {
    /// Liveness status, always `"ok"` while the endpoint can respond.
    pub status: &'static str,
}

/// Overall readiness status returned by `/readyz`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    /// Every dependency check passed and the traffic gate is open.
    Ready,
    /// At least one check failed, timed out, or the traffic gate is closed.
    NotReady,
}

/// Status of one entry in a `/readyz` response.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The check passed.
    Pass,
    /// The check completed with a failure.
    Fail,
    /// The check exceeded its individual timeout.
    TimedOut,
}

/// The public result of one readiness check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CheckResult {
    /// Stable registration name.
    pub name: String,
    /// Whether the check passed, failed, or timed out.
    pub status: CheckStatus,
    /// Check wall-clock duration rounded down to milliseconds.
    pub duration_ms: u64,
    /// Sanitized failure detail, omitted for passing checks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// JSON returned by `/readyz`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReadinessResponse {
    /// Aggregate readiness derived from gating results only.
    pub status: ReadinessStatus,
    /// Whether the manual traffic gate is open.
    pub accepting_traffic: bool,
    /// Gate result followed by dependency results in registration order.
    pub checks: Vec<CheckResult>,
    /// Non-gating diagnostic results in registration order.
    pub diagnostics: Vec<CheckResult>,
}

/// JSON returned by `/buildinfo`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildInfoResponse {
    /// OpenTelemetry-compatible service name (`<product>-<process>`).
    pub service_name: String,
    /// Cargo package version supplied by the composition root.
    pub version: String,
    /// Source-control commit supplied by the composition root.
    pub commit: String,
    /// Rust compiler version used to build this crate.
    pub rust_version: &'static str,
}

impl From<&ServiceIdentity> for BuildInfoResponse {
    fn from(identity: &ServiceIdentity) -> Self {
        Self {
            service_name: identity.service_name(),
            version: identity.version().to_owned(),
            commit: identity.commit().to_owned(),
            rust_version: RUST_VERSION,
        }
    }
}

#[derive(Clone)]
struct OpsState {
    readiness: ReadinessRegistry,
    traffic_gate: TrafficGate,
    metrics: PrometheusHandle,
    build_info: BuildInfoResponse,
}

/// Builder for the private Axum operations router.
///
/// The builder intentionally adds no authentication and no CORS layer. The
/// composition root is responsible for binding its output only on a separate,
/// private operations listener.
pub struct OpsRouter {
    identity: ServiceIdentity,
    readiness: ReadinessRegistry,
    traffic_gate: TrafficGate,
    metrics: PrometheusHandle,
}

impl OpsRouter {
    /// Creates a builder using the render handle returned by `baukit-telemetry`.
    pub fn new(identity: ServiceIdentity, metrics: PrometheusHandle) -> Self {
        Self {
            identity,
            readiness: ReadinessRegistry::new(),
            traffic_gate: TrafficGate::new(),
            metrics,
        }
    }

    /// Uses a shared readiness registry.
    #[must_use]
    pub fn with_readiness(mut self, readiness: ReadinessRegistry) -> Self {
        self.readiness = readiness;
        self
    }

    /// Uses a shared manual traffic gate.
    #[must_use]
    pub fn with_traffic_gate(mut self, traffic_gate: TrafficGate) -> Self {
        self.traffic_gate = traffic_gate;
        self
    }

    /// Returns the readiness registry owned by this builder.
    pub const fn readiness(&self) -> &ReadinessRegistry {
        &self.readiness
    }

    /// Returns the traffic gate owned by this builder.
    pub const fn traffic_gate(&self) -> &TrafficGate {
        &self.traffic_gate
    }

    /// Builds the Axum router.
    pub fn into_router(self) -> Router {
        let state = OpsState {
            build_info: BuildInfoResponse::from(&self.identity),
            readiness: self.readiness,
            traffic_gate: self.traffic_gate,
            metrics: self.metrics,
        };

        Router::new()
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/metrics", get(metrics))
            .route("/buildinfo", get(buildinfo))
            .with_state(state)
    }
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn readyz(State(state): State<OpsState>) -> impl IntoResponse {
    let accepting = state.traffic_gate.is_accepting();
    let mut gating_results = vec![CheckResult {
        name: "accepting_traffic".to_owned(),
        status: if accepting {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        duration_ms: 0,
        error: (!accepting).then(|| "service is draining".to_owned()),
    }];

    let checks = state.readiness.snapshot();
    let mut tasks = JoinSet::new();
    let mut task_metadata = HashMap::new();
    for (index, registered) in checks.into_iter().enumerate() {
        let name = registered.name.clone();
        let gating = registered.gating;
        let task = tasks.spawn(run_check(index, registered));
        task_metadata.insert(task.id(), (index, name, gating));
    }

    let mut dependency_results = Vec::new();
    while let Some(result) = tasks.join_next_with_id().await {
        match result {
            Ok((id, result)) => {
                task_metadata.remove(&id);
                dependency_results.push(result);
            }
            Err(error) => {
                let (index, name, gating) = task_metadata.remove(&error.id()).unwrap_or((
                    usize::MAX,
                    "readiness_task".to_owned(),
                    true,
                ));
                dependency_results.push((
                    index,
                    gating,
                    CheckResult {
                        name,
                        status: CheckStatus::Fail,
                        duration_ms: 0,
                        error: Some("readiness check task failed".to_owned()),
                    },
                ));
            }
        }
    }
    dependency_results.sort_by_key(|(index, _, _)| *index);
    let mut diagnostic_results = Vec::new();
    for (_, gating, result) in dependency_results {
        if gating {
            gating_results.push(result);
        } else {
            diagnostic_results.push(result);
        }
    }

    let ready = gating_results
        .iter()
        .all(|result| result.status == CheckStatus::Pass);
    let response = ReadinessResponse {
        status: if ready {
            ReadinessStatus::Ready
        } else {
            ReadinessStatus::NotReady
        },
        accepting_traffic: accepting,
        checks: gating_results,
        diagnostics: diagnostic_results,
    };
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response))
}

async fn run_check(index: usize, registered: RegisteredCheck) -> (usize, bool, CheckResult) {
    let started = Instant::now();
    let outcome = tokio::time::timeout(registered.timeout, registered.check.check()).await;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (status, error) = match outcome {
        Ok(Ok(())) => (CheckStatus::Pass, None),
        Ok(Err(error)) => (CheckStatus::Fail, Some(error.to_string())),
        Err(_) => (
            CheckStatus::TimedOut,
            Some(format!(
                "timed out after {} ms",
                registered.timeout.as_millis()
            )),
        ),
    };

    (
        index,
        registered.gating,
        CheckResult {
            name: registered.name,
            status,
            duration_ms,
            error,
        },
    )
}

async fn metrics(State(state): State<OpsState>) -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(PROMETHEUS_CONTENT_TYPE),
        )],
        state.metrics.render(),
    )
}

async fn buildinfo(State(state): State<OpsState>) -> Json<BuildInfoResponse> {
    Json(state.build_info)
}

fn sanitize_error(message: &str) -> String {
    let without_controls = message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let normalized = without_controls
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = if normalized.is_empty() {
        "readiness check failed"
    } else {
        &normalized
    };

    if normalized.len() <= MAX_ERROR_LENGTH {
        return normalized.to_owned();
    }

    let mut boundary = MAX_ERROR_LENGTH;
    while !normalized.is_char_boundary(boundary) {
        boundary -= 1;
    }
    normalized[..boundary].to_owned()
}

#[cfg(test)]
mod tests;

//! Standard process-wide observability for Baukit services.
//!
//! [`TelemetryBuilder`] installs structured logging, W3C trace propagation, an
//! OpenTelemetry tracer provider, and the process-wide Prometheus recorder. The
//! returned [`Telemetry`] value supplies the Prometheus render handle and owns
//! tracer shutdown.
//!
//! # Process-global initialization contract
//!
//! Successful [`TelemetryBuilder::init`] is process-global and non-resettable.
//! [`Telemetry::shutdown`] flushes exporters but deliberately does not make a
//! second initialization possible. Contract tests that exercise real telemetry
//! must therefore initialize it once in one process-wide test and perform all
//! telemetry assertions within that test. Unit tests that only need log output
//! should install a lightweight test subscriber instead.
//!
//! Process identity, deployment environment, and log-format vocabulary are
//! defined by the dependency-light `baukit-core` crate and re-exported here.
//! Telemetry remains the specification-owning higher-level crate without forcing
//! its OpenTelemetry stack into configuration consumers.
//!
//! ```no_run
//! use baukit_telemetry::{
//!     DeploymentEnvironment, ProcessKind, ServiceIdentity, TelemetryBuilder,
//! };
//!
//! let identity = ServiceIdentity::new(
//!     "fitness-tracker",
//!     ProcessKind::Api,
//!     env!("CARGO_PKG_VERSION"),
//!     option_env!("GIT_COMMIT").unwrap_or("unknown"),
//!     DeploymentEnvironment::Local,
//! );
//! let telemetry = TelemetryBuilder::new(identity).init()?;
//!
//! tracing::info!(message = "service started");
//! metrics::counter!("domain_operations_total", "operation" => "sync").increment(1);
//! let prometheus_text = telemetry.prometheus_handle().render();
//! telemetry.shutdown()?;
//! # Ok::<(), baukit_telemetry::TelemetryError>(())
//! ```
//!
//! The [`tracing`], [`metrics`], and [`opentelemetry`] crates are re-exported so
//! products can add ordinary spans, metrics, and propagation without a Baukit
//! wrapper. Metric label values must remain bounded and known at build time;
//! never use paths, identities, tokens, arbitrary errors, trace IDs, request
//! IDs, or provider payload data as labels.
//!
//! # Disabling the OpenTelemetry SDK
//!
//! Setting `OTEL_SDK_DISABLED=true` (case-insensitive) prevents construction of
//! the tracer provider, OTLP exporter, span processor, and exporter background
//! tasks. Structured logging and the Prometheus recorder remain active because
//! Baukit owns those facilities independently of the OpenTelemetry SDK. Other
//! values, including an empty value, leave the SDK enabled. The programmatic
//! [`TelemetryBuilder::sdk_disabled`] setting takes precedence over the
//! environment variable.

#![deny(missing_docs)]

use std::{
    collections::BTreeMap,
    env, fmt,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{SecondsFormat, Utc};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use opentelemetry::{
    Context as OtelContext, KeyValue, global,
    trace::{TraceContextExt, TracerProvider as _},
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    propagation::TraceContextPropagator,
    trace::{Sampler, SdkTracerProvider},
};
use regex::Regex;
use serde_json::{Map, Value, json};
use tracing::{Event, Id, Subscriber, field::Visit};
use tracing_subscriber::{
    EnvFilter, Registry,
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer},
    layer::{Context, SubscriberExt as _},
    registry::LookupSpan,
    util::SubscriberInitExt as _,
};

pub use baukit_core::{DeploymentEnvironment, LogFormat, ProcessKind, ServiceIdentity};
pub use metrics;
pub use metrics_exporter_prometheus::PrometheusHandle;
pub use opentelemetry;
pub use tracing;
pub use tracing_opentelemetry::OpenTelemetrySpanExt;

const DEFAULT_FILTER: &str = "info";
const OTLP_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const OTEL_SDK_DISABLED_ENV: &str = "OTEL_SDK_DISABLED";

/// Histogram buckets in seconds applied to `http_request_duration_seconds`,
/// as required by telemetry-spec §2.1. `baukit-http` records the metric; this
/// crate owns the recorder and therefore the bucket configuration.
pub const HTTP_DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];
/// Histogram buckets in seconds applied to `worker_job_duration_seconds`, as
/// required by telemetry-spec §2.4.
pub const WORKER_DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0,
];
const RUST_VERSION: &str = env!("BAUKIT_RUST_VERSION");

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static EMAIL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b")
        .expect("the email scrubber regex is valid")
});
static BEARER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbearer\s+[A-Z0-9._~+/=-]+")
        .expect("the bearer-token scrubber regex is valid")
});
static SECRET_ASSIGNMENT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(authorization|cookie|token|password)\s*[:=]\s*(?:"[^"]*"|'[^']*'|[^\s,;]+)"#,
    )
    .expect("the secret-assignment scrubber regex is valid")
});

/// Errors returned while initializing or shutting down telemetry.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// Baukit telemetry was already initialized in this process.
    #[error("baukit telemetry has already been initialized")]
    AlreadyInitialized,
    /// A deployed process has no configured OTLP collector endpoint.
    #[error("{OTLP_ENDPOINT_ENV} must be set for the {environment} deployment environment")]
    MissingOtlpEndpoint {
        /// The deployed environment requiring an exporter.
        environment: DeploymentEnvironment,
    },
    /// The OTLP span exporter could not be constructed.
    #[error("failed to build OTLP span exporter: {0}")]
    OtlpExporter(#[source] opentelemetry_otlp::ExporterBuildError),
    /// A process-wide tracing subscriber was already installed elsewhere.
    #[error("a process-wide tracing subscriber is already installed")]
    TracingAlreadyInitialized,
    /// A process-wide metrics recorder was already installed elsewhere.
    #[error("a process-wide metrics recorder is already installed: {0}")]
    MetricsAlreadyInitialized(String),
    /// Flushing and shutting down the tracer provider failed.
    #[error("failed to shut down the OpenTelemetry tracer provider: {0}")]
    TraceShutdown(String),
}

/// Builder for process-wide logging, tracing, and Prometheus metrics.
#[derive(Clone, Debug)]
pub struct TelemetryBuilder {
    identity: ServiceIdentity,
    filter: Option<String>,
    log_format: LogFormat,
    otlp_endpoint: Option<String>,
    sampling_ratio: f64,
    sdk_disabled: Option<bool>,
}

impl TelemetryBuilder {
    /// Starts a telemetry builder for `identity`.
    pub fn new(identity: ServiceIdentity) -> Self {
        Self {
            identity,
            filter: None,
            log_format: LogFormat::Auto,
            otlp_endpoint: None,
            sampling_ratio: 1.0,
            sdk_disabled: None,
        }
    }

    /// Overrides `RUST_LOG` with an explicit [`EnvFilter`] directive string.
    pub fn filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Overrides automatic JSON-versus-pretty log selection.
    pub const fn log_format(mut self, log_format: LogFormat) -> Self {
        self.log_format = log_format;
        self
    }

    /// Overrides `OTEL_EXPORTER_OTLP_ENDPOINT` for OTLP trace export.
    pub fn otlp_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.otlp_endpoint = Some(endpoint.into());
        self
    }

    /// Sets parent-based trace sampling to a ratio clamped to `0.0..=1.0`.
    ///
    /// The default ratio is `1.0`, meaning all traces are sampled.
    pub fn sampling_ratio(mut self, ratio: f64) -> Self {
        self.sampling_ratio = if ratio.is_nan() {
            1.0
        } else {
            ratio.clamp(0.0, 1.0)
        };
        self
    }

    /// Explicitly enables or disables the OpenTelemetry SDK.
    ///
    /// This overrides `OTEL_SDK_DISABLED`. When disabled, Baukit does not build
    /// a tracer provider or exporter; structured logging and Prometheus metrics
    /// remain active.
    pub const fn sdk_disabled(mut self, disabled: bool) -> Self {
        self.sdk_disabled = Some(disabled);
        self
    }

    /// Installs process-wide telemetry and returns its metrics/shutdown owner.
    ///
    /// Initialization succeeds only once per process. A second call returns
    /// [`TelemetryError::AlreadyInitialized`], including after
    /// [`Telemetry::shutdown`]; successful initialization cannot be reset. Local
    /// processes may omit an OTLP endpoint; staging and production processes must
    /// configure one unless `OTEL_SDK_DISABLED=true` or
    /// [`TelemetryBuilder::sdk_disabled`] explicitly disables tracing. When an
    /// endpoint is configured, call this from within the process's Tokio runtime
    /// so the tonic gRPC exporter can use it.
    pub fn init(self) -> Result<Telemetry, TelemetryError> {
        if INITIALIZED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(TelemetryError::AlreadyInitialized);
        }

        match self.try_init() {
            Ok(telemetry) => Ok(telemetry),
            Err(error) => {
                INITIALIZED.store(false, Ordering::Release);
                Err(error)
            }
        }
    }

    fn try_init(self) -> Result<Telemetry, TelemetryError> {
        if tracing::dispatcher::has_been_set() {
            return Err(TelemetryError::TracingAlreadyInitialized);
        }

        let sdk_disabled = self.sdk_disabled.unwrap_or_else(otel_sdk_disabled_from_env);
        let mut provider = None;
        let otel_layer = if sdk_disabled {
            None
        } else {
            let endpoint = self.resolve_endpoint()?;
            let resource = resource_for(&self.identity);
            let root_sampler = if self.sampling_ratio == 1.0 {
                Sampler::AlwaysOn
            } else {
                Sampler::TraceIdRatioBased(self.sampling_ratio)
            };
            let sampler = Sampler::ParentBased(Box::new(root_sampler));
            let mut provider_builder = SdkTracerProvider::builder()
                .with_resource(resource)
                .with_sampler(sampler);

            if let Some(endpoint) = endpoint {
                let exporter = opentelemetry_otlp::SpanExporter::builder()
                    .with_tonic()
                    .with_endpoint(endpoint)
                    .build()
                    .map_err(TelemetryError::OtlpExporter)?;
                provider_builder = provider_builder.with_batch_exporter(exporter);
            }

            let tracer_provider = provider_builder.build();
            let tracer = tracer_provider.tracer(self.identity.service_name());
            provider = Some(tracer_provider);
            Some(tracing_opentelemetry::layer().with_tracer(tracer))
        };
        let recorder = PrometheusBuilder::new()
            .set_buckets_for_metric(
                Matcher::Full("http_request_duration_seconds".to_owned()),
                HTTP_DURATION_BUCKETS,
            )
            .expect("HTTP_DURATION_BUCKETS is non-empty")
            .set_buckets_for_metric(
                Matcher::Full("worker_job_duration_seconds".to_owned()),
                WORKER_DURATION_BUCKETS,
            )
            .expect("WORKER_DURATION_BUCKETS is non-empty")
            .build_recorder();
        let prometheus_handle = recorder.handle();
        if let Err(error) = metrics::set_global_recorder(recorder) {
            if let Some(provider) = provider.take() {
                let _ = provider.shutdown();
            }
            return Err(TelemetryError::MetricsAlreadyInitialized(error.to_string()));
        }

        let filter = self.filter.map(EnvFilter::new).unwrap_or_else(|| {
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
        });
        let format = resolve_log_format(self.log_format, self.identity.environment());
        let subscriber = Registry::default()
            .with(filter)
            .with(SpanFieldLayer)
            .with(otel_layer);

        let subscriber_result = match format {
            ResolvedLogFormat::Json => subscriber
                .with(
                    tracing_subscriber::fmt::layer()
                        .event_format(JsonEventFormatter)
                        .fmt_fields(tracing_subscriber::fmt::format::JsonFields::new()),
                )
                .try_init(),
            ResolvedLogFormat::Pretty => subscriber
                .with(
                    tracing_subscriber::fmt::layer()
                        .event_format(HumanEventFormatter)
                        .fmt_fields(tracing_subscriber::fmt::format::DefaultFields::new()),
                )
                .try_init(),
        };
        subscriber_result.map_err(|_| TelemetryError::TracingAlreadyInitialized)?;

        register_build_info(&self.identity);

        global::set_text_map_propagator(TraceContextPropagator::new());

        Ok(Telemetry {
            prometheus_handle,
            provider: Mutex::new(provider),
            sdk_disabled,
        })
    }

    fn resolve_endpoint(&self) -> Result<Option<String>, TelemetryError> {
        let endpoint = self
            .otlp_endpoint
            .clone()
            .or_else(|| env::var(OTLP_ENDPOINT_ENV).ok())
            .filter(|endpoint| !endpoint.trim().is_empty());

        if endpoint.is_none() && self.identity.environment() != DeploymentEnvironment::Local {
            return Err(TelemetryError::MissingOtlpEndpoint {
                environment: self.identity.environment(),
            });
        }

        Ok(endpoint)
    }
}

/// Owns the Prometheus render handle and OpenTelemetry shutdown lifecycle.
pub struct Telemetry {
    prometheus_handle: PrometheusHandle,
    provider: Mutex<Option<SdkTracerProvider>>,
    sdk_disabled: bool,
}

impl Telemetry {
    /// Returns the handle used by an operations endpoint to render `/metrics`.
    pub const fn prometheus_handle(&self) -> &PrometheusHandle {
        &self.prometheus_handle
    }

    /// Returns whether OpenTelemetry SDK initialization was disabled.
    ///
    /// This remains stable after [`Telemetry::shutdown`].
    pub const fn is_otel_sdk_disabled(&self) -> bool {
        self.sdk_disabled
    }

    /// Flushes pending spans and shuts down the tracer provider exactly once.
    ///
    /// Repeated calls, local no-exporter mode, and disabled-SDK mode are
    /// successful no-ops.
    pub fn shutdown(&self) -> Result<(), TelemetryError> {
        let provider = self
            .provider
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();

        if let Some(provider) = provider {
            provider
                .shutdown()
                .map_err(|error| TelemetryError::TraceShutdown(error.to_string()))?;
        }

        Ok(())
    }
}

fn otel_sdk_disabled_from_env() -> bool {
    env::var(OTEL_SDK_DISABLED_ENV)
        .ok()
        .is_some_and(|value| parse_otel_sdk_disabled(&value))
}

fn parse_otel_sdk_disabled(value: &str) -> bool {
    value.eq_ignore_ascii_case("true")
}

fn resource_for(identity: &ServiceIdentity) -> Resource {
    Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", identity.service_name()),
            KeyValue::new("service.version", identity.version().to_owned()),
            KeyValue::new("service.commit", identity.commit().to_owned()),
            KeyValue::new(
                "deployment.environment.name",
                identity.environment().to_string(),
            ),
            KeyValue::new("product", identity.product().to_owned()),
        ])
        .build()
}

fn register_build_info(identity: &ServiceIdentity) {
    metrics::describe_gauge!("build_info", "Static process build information");
    metrics::gauge!(
        "build_info",
        "version" => identity.version().to_owned(),
        "commit" => identity.commit().to_owned(),
        "rust_version" => RUST_VERSION,
    )
    .set(1.0);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolvedLogFormat {
    Json,
    Pretty,
}

const fn resolve_log_format(
    requested: LogFormat,
    environment: DeploymentEnvironment,
) -> ResolvedLogFormat {
    match requested {
        LogFormat::Json => ResolvedLogFormat::Json,
        LogFormat::Pretty => ResolvedLogFormat::Pretty,
        LogFormat::Auto => match environment {
            DeploymentEnvironment::Local => ResolvedLogFormat::Pretty,
            DeploymentEnvironment::Staging | DeploymentEnvironment::Production => {
                ResolvedLogFormat::Json
            }
        },
    }
}

#[derive(Default)]
struct JsonVisitor {
    fields: Map<String, Value>,
}

impl Visit for JsonVisitor {
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.record_value(field, json!(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_value(field, json!(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_value(field, json!(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_value(field, json!(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_value(field, Value::String(value.to_owned()));
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.record_value(field, Value::String(value.to_string()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.record_value(field, Value::String(format!("{value:?}")));
    }
}

impl JsonVisitor {
    fn record_value(&mut self, field: &tracing::field::Field, value: Value) {
        let value = if is_sensitive_field(field.name()) {
            Value::String("[REDACTED]".to_owned())
        } else if let Value::String(value) = value {
            Value::String(scrub_text(&value))
        } else {
            value
        };
        self.fields.insert(field.name().to_owned(), value);
    }
}

fn scrub_text(value: &str) -> String {
    let value = EMAIL_PATTERN.replace_all(value, "[REDACTED]");
    let value = BEARER_PATTERN.replace_all(&value, "Bearer [REDACTED]");
    SECRET_ASSIGNMENT_PATTERN
        .replace_all(&value, "$1=[REDACTED]")
        .into_owned()
}

fn is_sensitive_field(name: &str) -> bool {
    let normalized = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "authorization",
        "cookie",
        "token",
        "password",
        "email",
        "requestbody",
        "responsebody",
        "providerpayload",
    ]
    .iter()
    .any(|sensitive| normalized.contains(sensitive))
}

#[derive(Default)]
struct SpanFields(BTreeMap<String, Value>);

struct SpanFieldLayer;

impl<S> tracing_subscriber::Layer<S> for SpanFieldLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        id: &Id,
        context: Context<'_, S>,
    ) {
        let mut visitor = JsonVisitor::default();
        attributes.record(&mut visitor);
        if let Some(span) = context.span(id) {
            span.extensions_mut().insert(SpanFields(
                visitor.fields.into_iter().collect::<BTreeMap<_, _>>(),
            ));
        }
    }

    fn on_record(&self, id: &Id, values: &tracing::span::Record<'_>, context: Context<'_, S>) {
        let mut visitor = JsonVisitor::default();
        values.record(&mut visitor);
        if let Some(span) = context.span(id) {
            let mut extensions = span.extensions_mut();
            if let Some(fields) = extensions.get_mut::<SpanFields>() {
                fields.0.extend(visitor.fields);
            }
        }
    }
}

struct JsonEventFormatter;

impl<S, N> FormatEvent<S, N> for JsonEventFormatter
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        writeln!(writer, "{}", Value::Object(log_fields(context, event)))
    }
}

struct HumanEventFormatter;

impl<S, N> FormatEvent<S, N> for HumanEventFormatter
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let fields = log_fields(context, event);
        let text = |name| fields.get(name).and_then(Value::as_str).unwrap_or_default();
        write!(
            writer,
            "{} {} {}: {}",
            text("timestamp"),
            text("level"),
            text("target"),
            text("message")
        )?;

        for (name, value) in fields {
            if !matches!(name.as_str(), "timestamp" | "level" | "target" | "message") {
                write!(writer, " {name}={value}")?;
            }
        }
        writeln!(writer)
    }
}

fn log_fields<S, N>(context: &FmtContext<'_, S, N>, event: &Event<'_>) -> Map<String, Value>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    let metadata = event.metadata();
    let mut visitor = JsonVisitor::default();
    event.record(&mut visitor);

    let mut fields = Map::new();
    fields.insert(
        "timestamp".to_owned(),
        Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)),
    );
    fields.insert(
        "level".to_owned(),
        Value::String(metadata.level().to_string()),
    );
    fields.insert(
        "target".to_owned(),
        Value::String(metadata.target().to_owned()),
    );
    fields.extend(visitor.fields);
    fields
        .entry("message".to_owned())
        .or_insert_with(|| Value::String(String::new()));

    if let Some(scope) = context.event_scope() {
        for span in scope.from_root() {
            if let Some(span_fields) = span.extensions().get::<SpanFields>()
                && let Some(request_id) = span_fields.0.get("request_id")
            {
                fields.insert("request_id".to_owned(), request_id.clone());
            }
        }
    }

    let otel_context = OtelContext::current();
    let otel_span = otel_context.span();
    let span_context = otel_span.span_context();
    if span_context.is_valid() {
        fields.insert(
            "trace_id".to_owned(),
            Value::String(span_context.trace_id().to_string()),
        );
        fields.insert(
            "span_id".to_owned(),
            Value::String(span_context.span_id().to_string()),
        );
    }

    fields
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[derive(Clone)]
    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for BufferWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .write(bytes)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn identity(environment: DeploymentEnvironment) -> ServiceIdentity {
        ServiceIdentity::new(
            "fitness-tracker",
            ProcessKind::Api,
            "1.4.2",
            "a1b2c3d",
            environment,
        )
    }

    #[test]
    fn composes_service_names_for_every_process_kind() {
        for (kind, suffix) in [
            (ProcessKind::Api, "api"),
            (ProcessKind::Worker, "worker"),
            (ProcessKind::Migrate, "migrate"),
            (ProcessKind::Seed, "seed"),
        ] {
            let identity = ServiceIdentity::new(
                "fitness-tracker",
                kind,
                "1.4.2",
                "a1b2c3d",
                DeploymentEnvironment::Local,
            );
            assert_eq!(identity.service_name(), format!("fitness-tracker-{suffix}"));
        }
    }

    #[test]
    fn resource_contains_exact_contract_attributes() {
        let attributes = resource_for(&identity(DeploymentEnvironment::Production))
            .iter()
            .map(|(key, value)| (key.as_str().to_owned(), value.to_string()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(attributes.len(), 5);
        assert_eq!(attributes["service.name"], "fitness-tracker-api");
        assert_eq!(attributes["service.version"], "1.4.2");
        assert_eq!(attributes["service.commit"], "a1b2c3d");
        assert_eq!(attributes["deployment.environment.name"], "production");
        assert_eq!(attributes["product"], "fitness-tracker");
    }

    #[test]
    fn resolves_log_format_from_environment_and_override() {
        assert_eq!(
            resolve_log_format(LogFormat::Auto, DeploymentEnvironment::Local),
            ResolvedLogFormat::Pretty
        );
        assert_eq!(
            resolve_log_format(LogFormat::Auto, DeploymentEnvironment::Staging),
            ResolvedLogFormat::Json
        );
        assert_eq!(
            resolve_log_format(LogFormat::Auto, DeploymentEnvironment::Production),
            ResolvedLogFormat::Json
        );
        assert_eq!(
            resolve_log_format(LogFormat::Json, DeploymentEnvironment::Local),
            ResolvedLogFormat::Json
        );
        assert_eq!(
            resolve_log_format(LogFormat::Pretty, DeploymentEnvironment::Production),
            ResolvedLogFormat::Pretty
        );
    }

    #[test]
    fn parses_otel_sdk_disabled_using_standard_boolean_semantics() {
        for enabled in ["true", "TRUE", "True", "tRuE"] {
            assert!(parse_otel_sdk_disabled(enabled));
        }
        for disabled in ["", "false", "FALSE", "1", "yes", " true "] {
            assert!(!parse_otel_sdk_disabled(disabled));
        }
    }

    #[test]
    fn json_logs_include_correlation_and_redact_sensitive_fields() {
        let provider = SdkTracerProvider::builder().build();
        let tracer = provider.tracer("formatter-test");
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = BufferWriter(Arc::clone(&output));
        let subscriber = Registry::default()
            .with(SpanFieldLayer)
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(move || writer.clone())
                    .event_format(JsonEventFormatter)
                    .fmt_fields(tracing_subscriber::fmt::format::JsonFields::new()),
            );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("request", request_id = "req-123");
            let _entered = span.enter();
            tracing::info!(
                message = "request completed for private@example.com",
                user_email = "private@example.com"
            );
        });
        provider
            .shutdown()
            .expect("provider shutdown should succeed");

        let rendered = String::from_utf8(
            output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        )
        .expect("formatter should emit UTF-8");
        let event: Value = serde_json::from_str(rendered.trim()).expect("valid JSON log event");

        assert_eq!(event["message"], "request completed for [REDACTED]");
        assert_eq!(event["request_id"], "req-123");
        assert_eq!(event["user_email"], "[REDACTED]");
        assert!(event["timestamp"].as_str().is_some());
        assert_eq!(event["level"], "INFO");
        assert_eq!(event["target"], module_path!());
        assert_eq!(event["trace_id"].as_str().map(str::len), Some(32));
        assert_eq!(event["span_id"].as_str().map(str::len), Some(16));
    }
}

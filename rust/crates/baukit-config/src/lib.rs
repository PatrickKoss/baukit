//! Layered, validated configuration conventions for Baukit services.
//!
//! [`ConfigLoader`] applies configuration in this order: serde-backed defaults,
//! an optional local file, then environment variables. Environment variables use
//! an application prefix and a double-underscore separator, such as
//! `MYAPP__HTTP__PORT=8080`. A local `.env` file is loaded only when the loader's
//! deployment [`Environment`] is [`Environment::Local`].
//!
//! Durations in the standard sections are represented as [`Duration`] values and
//! deserialize from integer seconds.
//!
//! Shared environment and log-format vocabulary lives in the dependency-light
//! `baukit-core` crate and is re-exported here. This lets configuration and
//! telemetry use identical types without making configuration depend on the
//! OpenTelemetry exporter stack.
//!
//! # Example
//!
//! ```no_run
//! use baukit_config::{BaukitConfig, ConfigLoader, Environment, Validate, ValidationErrors};
//! use serde::Deserialize;
//!
//! #[derive(Debug, Default, Deserialize)]
//! #[serde(default)]
//! struct ProductConfig {
//!     feature_name: Option<String>,
//! }
//!
//! impl Validate for ProductConfig {
//!     fn validate(&self) -> Result<(), ValidationErrors> {
//!         Ok(())
//!     }
//! }
//!
//! let config: BaukitConfig<ProductConfig> =
//!     ConfigLoader::new("myapp", Environment::Local)?.load()?;
//! # Ok::<(), baukit_config::LoadError>(())
//! ```

#![deny(missing_docs)]

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    time::Duration,
};

pub use baukit_core::{
    DeploymentEnvironment, DeploymentEnvironment as Environment, LogFormat, ParseEnvironmentError,
};
use config::{Config, File, Value, ValueKind};
use serde::{Deserialize, Deserializer};
use thiserror::Error;
use zeroize::Zeroize;

const DEFAULT_LOCAL_FILE: &str = "config/local.toml";
const DEFAULT_DOTENV_FILE: &str = ".env";
const REDACTED: &str = "[redacted]";

/// A secret value whose formatted representations never reveal its contents.
///
/// The inner value is zeroized when the wrapper is dropped. Access is deliberately
/// named [`Secret::expose`] so reviews can find places where secrets are revealed.
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    /// Wraps a value as a secret.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Deliberately exposes a shared reference to the secret value.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> Clone for Secret<T>
where
    T: Clone + Zeroize,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

impl<T: Zeroize> fmt::Display for Secret<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<'de, T> Deserialize<'de> for Secret<T>
where
    T: Deserialize<'de> + Zeroize,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self)
    }
}

/// Configuration for the public HTTP listener.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    /// Address on which the public listener accepts connections.
    pub bind_address: IpAddr,
    /// TCP port for the public listener.
    pub port: u16,
    /// Maximum time allowed for an HTTP request, in integer seconds on input.
    #[serde(with = "duration_seconds")]
    pub request_timeout: Duration,
    /// Maximum accepted request-body size in bytes.
    pub body_size_limit: usize,
    /// Maximum number of requests processed concurrently.
    pub concurrency_limit: usize,
    /// Origins allowed by CORS. An empty list allows no cross-origin requests.
    pub cors_allowed_origins: Vec<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind_address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 8080,
            request_timeout: Duration::from_secs(30),
            body_size_limit: 2 * 1024 * 1024,
            concurrency_limit: 1_024,
            cors_allowed_origins: Vec::new(),
        }
    }
}

impl Validate for HttpConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        require_non_zero(self.port, "port", &mut errors);
        require_non_zero(
            self.request_timeout.as_secs(),
            "request_timeout",
            &mut errors,
        );
        require_non_zero(self.body_size_limit, "body_size_limit", &mut errors);
        require_non_zero(self.concurrency_limit, "concurrency_limit", &mut errors);
        for (index, origin) in self.cors_allowed_origins.iter().enumerate() {
            if origin.trim().is_empty() {
                errors.push(ValidationError::new(
                    format!("cors_allowed_origins[{index}]"),
                    "must not be empty",
                ));
            }
        }
        validation_result(errors)
    }
}

/// Configuration for the separate operations listener.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct OpsConfig {
    /// Address on which health, readiness, and metrics endpoints listen.
    pub bind_address: IpAddr,
    /// TCP port for the operations listener.
    pub port: u16,
}

impl Default for OpsConfig {
    fn default() -> Self {
        Self {
            bind_address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 9090,
        }
    }
}

impl Validate for OpsConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        require_non_zero(self.port, "port", &mut errors);
        validation_result(errors)
    }
}

/// Plain database connection and pool configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    /// Database connection URL. Formatting the wrapper always redacts it.
    pub url: Secret<String>,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of connections retained by the pool.
    pub min_connections: u32,
    /// Maximum time to wait for a connection, in integer seconds on input.
    #[serde(with = "duration_seconds")]
    pub acquire_timeout: Duration,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: Secret::new("postgres://localhost/app".to_owned()),
            max_connections: 10,
            min_connections: 1,
            acquire_timeout: Duration::from_secs(5),
        }
    }
}

impl Validate for DatabaseConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        if self.url.expose().trim().is_empty() {
            errors.push(ValidationError::new("url", "must not be empty"));
        }
        require_non_zero(self.max_connections, "max_connections", &mut errors);
        if self.min_connections > self.max_connections {
            errors.push(ValidationError::new(
                "min_connections",
                "must not exceed max_connections",
            ));
        }
        require_non_zero(
            self.acquire_timeout.as_secs(),
            "acquire_timeout",
            &mut errors,
        );
        validation_result(errors)
    }
}

/// Resource identity values consumed by telemetry initialization.
///
/// Deployment environment is carried by [`BaukitConfig::environment`]. Fields
/// remain optional because version and commit are commonly injected from build
/// metadata rather than a configuration file.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TelemetryResourceConfig {
    /// OpenTelemetry `service.name`, conventionally `<product>-<process>`.
    pub service_name: Option<String>,
    /// OpenTelemetry `service.version`, normally the Cargo package version.
    pub service_version: Option<String>,
    /// Baukit `service.commit`, normally a short build-time Git SHA.
    pub service_commit: Option<String>,
    /// Stable product identity shared by all of the product's processes.
    pub product: Option<String>,
}

impl Validate for TelemetryResourceConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        validate_optional_text(&self.service_name, "service_name", &mut errors);
        validate_optional_text(&self.service_version, "service_version", &mut errors);
        validate_optional_text(&self.service_commit, "service_commit", &mut errors);
        validate_optional_text(&self.product, "product", &mut errors);
        validation_result(errors)
    }
}

/// Configuration for traces and log rendering.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    /// OTLP endpoint, defaulting from `OTEL_EXPORTER_OTLP_ENDPOINT` when set.
    pub otlp_endpoint: Option<String>,
    /// Trace sampling ratio in the inclusive range `0.0..=1.0`.
    pub trace_sampling_ratio: f64,
    /// Optional log rendering override.
    pub log_format: LogFormat,
    /// Standard telemetry resource identity attributes.
    pub resource: TelemetryResourceConfig,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok(),
            trace_sampling_ratio: 1.0,
            log_format: LogFormat::Auto,
            resource: TelemetryResourceConfig::default(),
        }
    }
}

impl Validate for TelemetryConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        validate_optional_text(&self.otlp_endpoint, "otlp_endpoint", &mut errors);
        if !self.trace_sampling_ratio.is_finite()
            || !(0.0..=1.0).contains(&self.trace_sampling_ratio)
        {
            errors.push(ValidationError::new(
                "trace_sampling_ratio",
                "must be a finite value between 0.0 and 1.0",
            ));
        }
        extend_validation(&mut errors, self.resource.validate(), Some("resource"));
        validation_result(errors)
    }
}

/// Configuration for graceful shutdown.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ShutdownConfig {
    /// Maximum time allowed for draining work, in integer seconds on input.
    #[serde(with = "duration_seconds")]
    pub drain_timeout: Duration,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
        }
    }
}

impl Validate for ShutdownConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        require_non_zero(self.drain_timeout.as_secs(), "drain_timeout", &mut errors);
        validation_result(errors)
    }
}

/// Standard Baukit sections plus flattened, product-specific configuration.
///
/// Product fields live at the top level beside `http`, `ops`, optional `database`,
/// `telemetry`, and `shutdown`, while validation errors are prefixed with
/// `product.` to distinguish their ownership. Product types should use
/// `#[serde(default)]` when their default field values should apply to omitted
/// configuration keys.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct BaukitConfig<T: Default> {
    /// Deployment environment fixed by [`ConfigLoader`] at bootstrap.
    pub environment: Environment,
    /// Public HTTP listener settings.
    pub http: HttpConfig,
    /// Separate operations listener settings.
    pub ops: OpsConfig,
    /// Optional database connection and pool settings.
    ///
    /// Omit the section for database-free services. When present, omitted
    /// fields inside the section retain [`DatabaseConfig`] defaults.
    pub database: Option<DatabaseConfig>,
    /// Logging, tracing, and telemetry identity settings.
    pub telemetry: TelemetryConfig,
    /// Graceful shutdown settings.
    pub shutdown: ShutdownConfig,
    /// Product-owned extension fields, flattened into the top-level document.
    #[serde(flatten)]
    pub product: T,
}

impl<T: Default> Default for BaukitConfig<T> {
    fn default() -> Self {
        Self {
            environment: Environment::default(),
            http: HttpConfig::default(),
            ops: OpsConfig::default(),
            database: None,
            telemetry: TelemetryConfig::default(),
            shutdown: ShutdownConfig::default(),
            product: T::default(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct StandardConfig {
    environment: Environment,
    http: HttpConfig,
    ops: OpsConfig,
    database: Option<DatabaseConfig>,
    telemetry: TelemetryConfig,
    shutdown: ShutdownConfig,
}

impl<T> Validate for BaukitConfig<T>
where
    T: Default + Validate,
{
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        extend_validation(&mut errors, self.http.validate(), Some("http"));
        extend_validation(&mut errors, self.ops.validate(), Some("ops"));
        if let Some(database) = &self.database {
            extend_validation(&mut errors, database.validate(), Some("database"));
        }
        extend_validation(&mut errors, self.telemetry.validate(), Some("telemetry"));
        extend_validation(&mut errors, self.shutdown.validate(), Some("shutdown"));
        extend_validation(&mut errors, self.product.validate(), Some("product"));
        validation_result(errors)
    }
}

/// A type that checks its configuration invariants before startup.
pub trait Validate {
    /// Returns every validation problem found, rather than stopping at the first.
    fn validate(&self) -> Result<(), ValidationErrors>;
}

impl Validate for () {
    fn validate(&self) -> Result<(), ValidationErrors> {
        Ok(())
    }
}

/// One actionable configuration validation problem.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{path}: {message}")]
pub struct ValidationError {
    path: String,
    message: String,
}

impl ValidationError {
    /// Creates an error for a field path and an actionable message.
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Returns the field path associated with this error.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the actionable message associated with this error.
    pub fn message(&self) -> &str {
        &self.message
    }

    fn with_prefix(mut self, prefix: &str) -> Self {
        self.path = if self.path.is_empty() {
            prefix.to_owned()
        } else {
            format!("{prefix}.{}", self.path)
        };
        self
    }
}

/// An aggregate of all configuration validation problems.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{rendered}")]
pub struct ValidationErrors {
    errors: Vec<ValidationError>,
    rendered: String,
}

impl ValidationErrors {
    /// Creates an aggregate from validation problems.
    ///
    /// Callers should return `Ok(())` instead when `errors` is empty.
    pub fn new(errors: Vec<ValidationError>) -> Self {
        let mut rendered = format!(
            "configuration validation failed ({} error(s))",
            errors.len()
        );
        for error in &errors {
            rendered.push_str("\n- ");
            rendered.push_str(&error.to_string());
        }
        Self { errors, rendered }
    }

    /// Returns all collected validation problems.
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Consumes the aggregate and returns its individual problems.
    pub fn into_errors(self) -> Vec<ValidationError> {
        self.errors
    }
}

/// Loads a [`BaukitConfig`] using the Baukit precedence and naming conventions.
#[derive(Clone, Debug)]
pub struct ConfigLoader {
    prefix: String,
    environment: Environment,
    local_file: Option<PathBuf>,
    dotenv_file: Option<PathBuf>,
}

impl ConfigLoader {
    /// Creates a loader for an application and deployment environment.
    ///
    /// ASCII letters are uppercased and `-` becomes `_`, so `my-app` uses the
    /// `MY_APP__` prefix. Other characters are rejected.
    pub fn new(app_name: &str, environment: Environment) -> Result<Self, LoadError> {
        let prefix = normalize_prefix(app_name)?;
        Ok(Self {
            prefix,
            environment,
            local_file: Some(PathBuf::from(DEFAULT_LOCAL_FILE)),
            dotenv_file: Some(PathBuf::from(DEFAULT_DOTENV_FILE)),
        })
    }

    /// Replaces the optional local configuration-file path.
    #[must_use]
    pub fn local_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.local_file = Some(path.into());
        self
    }

    /// Disables the local configuration-file layer.
    #[must_use]
    pub fn without_local_file(mut self) -> Self {
        self.local_file = None;
        self
    }

    /// Replaces the local dotenv-file path.
    ///
    /// This path is ignored unless the loader environment is local.
    #[must_use]
    pub fn dotenv_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.dotenv_file = Some(path.into());
        self
    }

    /// Disables dotenv loading, including in the local environment.
    #[must_use]
    pub fn without_dotenv(mut self) -> Self {
        self.dotenv_file = None;
        self
    }

    /// Returns the normalized environment-variable prefix without its separator.
    pub fn environment_prefix(&self) -> &str {
        &self.prefix
    }

    /// Loads, deserializes, and validates a standard configuration plus product fields.
    pub fn load<T>(&self) -> Result<BaukitConfig<T>, LoadError>
    where
        T: Default + Validate + serde::de::DeserializeOwned,
    {
        self.load_dotenv()?;

        let mut builder = Config::builder();
        if let Some(path) = &self.local_file {
            builder = builder.add_source(File::from(path.clone()).required(false));
        }
        builder = builder.add_source(
            config::Environment::with_prefix(&self.prefix)
                .prefix_separator("__")
                .separator("__")
                // Keep the source representation intact so values destined for
                // `Secret<String>` cannot lose leading zeroes or exponent syntax.
                // `config` still converts strings when deserializing typed fields.
                .try_parsing(false),
        );
        // The bootstrap environment is authoritative because it controls whether
        // reading a dotenv file is safe.
        builder = builder.set_override("environment", self.environment.to_string())?;

        let merged = builder.build()?;
        let standard = merged.clone().try_deserialize::<StandardConfig>()?;
        let product = deserialize_product_config::<T>(merged.cache)?;
        let loaded = BaukitConfig {
            environment: standard.environment,
            http: standard.http,
            ops: standard.ops,
            database: standard.database,
            telemetry: standard.telemetry,
            shutdown: standard.shutdown,
            product,
        };
        loaded.validate()?;
        Ok(loaded)
    }

    fn load_dotenv(&self) -> Result<(), LoadError> {
        if self.environment != Environment::Local {
            return Ok(());
        }
        let Some(path) = &self.dotenv_file else {
            return Ok(());
        };
        match dotenvy::from_path(path) {
            Ok(_) => Ok(()),
            Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(())
            }
            Err(source) => Err(LoadError::Dotenv {
                path: path.clone(),
                source,
            }),
        }
    }
}

fn deserialize_product_config<T>(mut merged: Value) -> Result<T, config::ConfigError>
where
    T: Default + serde::de::DeserializeOwned,
{
    if let ValueKind::Table(table) = &mut merged.kind {
        for standard_key in [
            "environment",
            "http",
            "ops",
            "database",
            "telemetry",
            "shutdown",
        ] {
            table.remove(standard_key);
        }
        if table.is_empty() {
            return Ok(T::default());
        }
    }
    merged.try_deserialize()
}

/// An error that prevents configuration from being loaded for startup.
#[derive(Debug, Error)]
pub enum LoadError {
    /// The application name cannot be represented as an environment prefix.
    #[error("invalid application name `{app_name}` for an environment prefix: {reason}")]
    InvalidPrefix {
        /// Application name supplied by the caller.
        app_name: String,
        /// Explanation of the accepted prefix syntax.
        reason: &'static str,
    },
    /// A local dotenv file exists but cannot be loaded.
    #[error("failed to load dotenv file `{path}`: {source}")]
    Dotenv {
        /// Dotenv path being loaded.
        path: PathBuf,
        /// Underlying dotenv parser or I/O error.
        #[source]
        source: dotenvy::Error,
    },
    /// A configuration source could not be merged or deserialized.
    #[error("failed to load configuration: {0}")]
    Configuration(#[from] config::ConfigError),
    /// The merged configuration violated one or more invariants.
    #[error(transparent)]
    Validation(#[from] ValidationErrors),
}

fn normalize_prefix(app_name: &str) -> Result<String, LoadError> {
    if app_name.is_empty() {
        return Err(invalid_prefix(app_name));
    }
    let mut prefix = String::with_capacity(app_name.len());
    for character in app_name.chars() {
        match character {
            'a'..='z' => prefix.push(character.to_ascii_uppercase()),
            'A'..='Z' | '0'..='9' | '_' => prefix.push(character),
            '-' => prefix.push('_'),
            _ => return Err(invalid_prefix(app_name)),
        }
    }
    Ok(prefix)
}

fn invalid_prefix(app_name: &str) -> LoadError {
    LoadError::InvalidPrefix {
        app_name: app_name.to_owned(),
        reason: "use only ASCII letters, digits, `_`, or `-`",
    }
}

fn require_non_zero<T>(value: T, path: &str, errors: &mut Vec<ValidationError>)
where
    T: Default + PartialEq,
{
    if value == T::default() {
        errors.push(ValidationError::new(path, "must be non-zero"));
    }
}

fn validate_optional_text(value: &Option<String>, path: &str, errors: &mut Vec<ValidationError>) {
    if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
        errors.push(ValidationError::new(path, "must not be empty when set"));
    }
}

fn validation_result(errors: Vec<ValidationError>) -> Result<(), ValidationErrors> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors::new(errors))
    }
}

fn extend_validation(
    target: &mut Vec<ValidationError>,
    result: Result<(), ValidationErrors>,
    prefix: Option<&str>,
) {
    if let Err(errors) = result {
        target.extend(errors.into_errors().into_iter().map(|error| match prefix {
            Some(prefix) => error.with_prefix(prefix),
            None => error,
        }));
    }
}

mod duration_seconds {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer};

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        u64::deserialize(deserializer).map(Duration::from_secs)
    }
}

#[cfg(test)]
mod tests;

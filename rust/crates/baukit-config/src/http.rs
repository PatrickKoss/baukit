use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use serde::{Deserialize, Deserializer};

use crate::{
    Environment, Validate, ValidationError, ValidationErrors, duration_seconds, require_non_zero,
    validation_result,
};

/// HSTS `max-age` outside [`Environment::Local`] when the setting is omitted: one year.
pub const DEFAULT_HSTS_MAX_AGE: Duration = Duration::from_secs(365 * 24 * 60 * 60);

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
    /// HSTS `max-age`, in integer seconds on input (`<PREFIX>__HTTP__HSTS_MAX_AGE`).
    ///
    /// `None` sends no `Strict-Transport-Security` header. When the setting is
    /// omitted, [`ConfigLoader::load`](crate::ConfigLoader::load) leaves it off in
    /// [`Environment::Local`] and uses [`DEFAULT_HSTS_MAX_AGE`] elsewhere. `0`
    /// turns it off in every environment.
    #[serde(deserialize_with = "hsts_seconds")]
    pub hsts_max_age: Option<Duration>,
}

impl HttpConfig {
    pub(crate) fn with_hsts_default(mut self, environment: Environment, configured: bool) -> Self {
        if !configured && environment != Environment::Local {
            self.hsts_max_age = Some(DEFAULT_HSTS_MAX_AGE);
        }
        self
    }
}

pub(crate) fn hsts_configured(merged: &config::Config) -> Result<bool, config::ConfigError> {
    match merged.get::<u64>(HSTS_MAX_AGE_KEY) {
        Ok(_) => Ok(true),
        Err(config::ConfigError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

const HSTS_MAX_AGE_KEY: &str = "http.hsts_max_age";

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind_address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 8080,
            request_timeout: Duration::from_secs(30),
            body_size_limit: 2 * 1024 * 1024,
            concurrency_limit: 1_024,
            cors_allowed_origins: Vec::new(),
            hsts_max_age: None,
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

fn hsts_seconds<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    let seconds = u64::deserialize(deserializer)?;
    Ok((seconds > 0).then(|| Duration::from_secs(seconds)))
}

//! Dependency-light shared vocabulary for Baukit crates and services.
//!
//! This crate is the canonical home for process identity and configuration
//! vocabulary shared across crate boundaries. It deliberately contains no
//! telemetry exporters, async runtime, HTTP framework, or operational routing,
//! so configuration-only consumers do not inherit those dependency graphs.
//! Higher-level Baukit crates re-export these types from their established APIs.

#![deny(missing_docs)]

use std::{fmt, str::FromStr};

use serde::Deserialize;
use thiserror::Error;

/// The deployment environment attached to configuration and telemetry signals.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentEnvironment {
    /// A developer workstation or other local process.
    #[default]
    Local,
    /// A deployed pre-production process.
    Staging,
    /// A deployed production process.
    Production,
}

impl DeploymentEnvironment {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }
}

impl fmt::Display for DeploymentEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DeploymentEnvironment {
    type Err = ParseEnvironmentError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "local" => Ok(Self::Local),
            "staging" => Ok(Self::Staging),
            "production" => Ok(Self::Production),
            _ => Err(ParseEnvironmentError(value.to_owned())),
        }
    }
}

/// An error returned when a deployment environment name is not supported.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("unsupported environment `{0}`; expected local, staging, or production")]
pub struct ParseEnvironmentError(String);

/// Selection policy for the stdout log formatter.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Select JSON for staging and production, and pretty output locally.
    #[default]
    Auto,
    /// Always emit newline-delimited JSON.
    Json,
    /// Always emit human-readable pretty output.
    Pretty,
}

impl LogFormat {
    /// Resolves [`LogFormat::Auto`] for a deployment environment.
    #[must_use]
    pub const fn resolve(self, environment: DeploymentEnvironment) -> Self {
        match (self, environment) {
            (Self::Auto, DeploymentEnvironment::Local) => Self::Pretty,
            (Self::Auto, DeploymentEnvironment::Staging | DeploymentEnvironment::Production) => {
                Self::Json
            }
            (explicit, _) => explicit,
        }
    }
}

/// The kind of process being identified and instrumented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessKind {
    /// A public or internal HTTP API process.
    Api,
    /// A background worker process.
    Worker,
    /// A database migration process.
    Migrate,
    /// A data seeding process.
    Seed,
}

impl ProcessKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Worker => "worker",
            Self::Migrate => "migrate",
            Self::Seed => "seed",
        }
    }
}

impl fmt::Display for ProcessKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Stable identity and build metadata for one running process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceIdentity {
    product: String,
    process: ProcessKind,
    version: String,
    commit: String,
    environment: DeploymentEnvironment,
}

impl ServiceIdentity {
    /// Creates a service identity.
    pub fn new(
        product: impl Into<String>,
        process: ProcessKind,
        version: impl Into<String>,
        commit: impl Into<String>,
        environment: DeploymentEnvironment,
    ) -> Self {
        Self {
            product: product.into(),
            process,
            version: version.into(),
            commit: commit.into(),
            environment,
        }
    }

    /// Returns the stable product identifier.
    pub fn product(&self) -> &str {
        &self.product
    }

    /// Returns this process's kind.
    pub const fn process(&self) -> ProcessKind {
        self.process
    }

    /// Returns the Cargo package version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the short source-control commit identifier.
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// Returns the deployment environment.
    pub const fn environment(&self) -> DeploymentEnvironment {
        self.environment
    }

    /// Composes the OpenTelemetry `service.name` value.
    pub fn service_name(&self) -> String {
        format!("{}-{}", self.product, self.process)
    }
}

/// Immutable version-control and compiler metadata for a process build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildInfo {
    version: String,
    commit: String,
    rust_version: String,
}

impl BuildInfo {
    /// Creates build metadata from values supplied by a build pipeline.
    pub fn new(
        version: impl Into<String>,
        commit: impl Into<String>,
        rust_version: impl Into<String>,
    ) -> Self {
        Self {
            version: version.into(),
            commit: commit.into(),
            rust_version: rust_version.into(),
        }
    }

    /// Returns the Cargo package version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the source-control commit identifier.
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// Returns the Rust compiler version used for the build.
    pub fn rust_version(&self) -> &str {
        &self.rust_version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_vocabulary_follows_the_telemetry_contract() {
        let identity = ServiceIdentity::new(
            "orders",
            ProcessKind::Api,
            "1.2.3",
            "abc123",
            DeploymentEnvironment::Production,
        );

        assert_eq!(identity.service_name(), "orders-api");
        assert_eq!(identity.environment().to_string(), "production");
        assert_eq!(
            LogFormat::Auto.resolve(DeploymentEnvironment::Production),
            LogFormat::Json
        );
    }
}

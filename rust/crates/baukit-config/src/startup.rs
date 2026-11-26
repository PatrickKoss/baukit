use std::time::Duration;

use serde::Deserialize;

use crate::{Validate, ValidationErrors, duration_seconds, require_non_zero, validation_result};

const DEFAULT_STARTUP_BUDGET: Duration = Duration::from_secs(60);

/// How long a process waits for its dependencies before startup fails.
///
/// The budget bounds the connect retries for PostgreSQL, Redis and identity
/// provider discovery. Keep it inside the startup probe window.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct StartupConfig {
    /// Retry budget for each dependency connect, in integer seconds on input.
    #[serde(with = "duration_seconds")]
    pub budget: Duration,
}

impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            budget: DEFAULT_STARTUP_BUDGET,
        }
    }
}

impl Validate for StartupConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        require_non_zero(self.budget.as_secs(), "budget", &mut errors);
        validation_result(errors)
    }
}

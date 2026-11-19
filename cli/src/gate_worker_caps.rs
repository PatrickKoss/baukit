use std::collections::BTreeMap;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Tool variables the strict gate sets to each lane's core share.
pub const WORKER_VARIABLES: [&str; 8] = [
    "BAUKIT_GATE_WORKERS",
    "CARGO_BUILD_JOBS",
    "NEXTEST_TEST_THREADS",
    "RUST_TEST_THREADS",
    "VITEST_MAX_WORKERS",
    "E2E_WORKERS",
    "BAUKIT_QA_GRADLE_WORKERS",
    "BAUKIT_QA_NATIVE_JOBS",
];

/// `[gate]` in `baukit.toml`: settings the strict gate runner reads.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GateManifest {
    /// Upper bound per worker variable; the gate exports `min(share, cap)`.
    #[serde(
        default,
        rename = "worker-caps",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub worker_caps: BTreeMap<String, i64>,
}

impl GateManifest {
    pub fn is_empty(&self) -> bool {
        self.worker_caps.is_empty()
    }

    /// # Errors
    ///
    /// Fails when a cap names a variable the gate does not set or is below 1.
    pub fn validate(&self) -> Result<()> {
        for (name, cap) in &self.worker_caps {
            ensure!(
                WORKER_VARIABLES.contains(&name.as_str()),
                "gate.worker-caps names unknown variable `{name}`; known: {}",
                WORKER_VARIABLES.join(", ")
            );
            ensure!(
                *cap >= 1,
                "gate.worker-caps.{name} must be a positive integer, got {cap}"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(source: &str) -> GateManifest {
        toml::from_str(source).unwrap_or_else(|error| panic!("parse: {error}"))
    }

    #[test]
    fn accepts_known_positive_caps() {
        let gate = gate("[worker-caps]\nE2E_WORKERS = 4\nCARGO_BUILD_JOBS = 1\n");
        assert!(gate.validate().is_ok());
        assert_eq!(gate.worker_caps.get("E2E_WORKERS"), Some(&4));
    }

    #[test]
    fn rejects_unknown_variable() {
        let error = gate("[worker-caps]\nPLAYWRIGHT = 4\n")
            .validate()
            .map_err(|error| error.to_string());
        assert!(matches!(error, Err(message) if message.contains("unknown variable `PLAYWRIGHT`")));
    }

    #[test]
    fn rejects_non_positive_caps() {
        for value in ["0", "-2"] {
            let error = gate(&format!("[worker-caps]\nE2E_WORKERS = {value}\n"))
                .validate()
                .map_err(|error| error.to_string());
            assert!(
                matches!(&error, Err(message) if message.contains("must be a positive integer")),
                "{value}: {error:?}"
            );
        }
    }

    #[test]
    fn rejects_unknown_gate_keys() {
        assert!(toml::from_str::<GateManifest>("workers = 2\n").is_err());
    }
}

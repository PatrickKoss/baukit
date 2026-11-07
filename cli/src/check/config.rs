use std::{collections::BTreeMap, fs, io, path::Path};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

pub const CONFIG_FILE: &str = "baukit-check.toml";
pub const DEFAULT_RUST_MAX_LINES: usize = 1000;
pub const DEFAULT_TS_MAX_LINES: usize = 500;

/// Settings read from `baukit-check.toml` at the checked root.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckConfig {
    #[serde(default)]
    pub file_size: FileSizeConfig,
    #[serde(default)]
    pub suppressions: SuppressionsConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct FileSizeConfig {
    #[serde(default = "default_rust_max_lines")]
    pub rust_max_lines: usize,
    #[serde(default = "default_ts_max_lines")]
    pub ts_max_lines: usize,
    #[serde(default)]
    pub allowlist: BTreeMap<String, usize>,
}

impl Default for FileSizeConfig {
    fn default() -> Self {
        Self {
            rust_max_lines: DEFAULT_RUST_MAX_LINES,
            ts_max_lines: DEFAULT_TS_MAX_LINES,
            allowlist: BTreeMap::new(),
        }
    }
}

const fn default_rust_max_lines() -> usize {
    DEFAULT_RUST_MAX_LINES
}

const fn default_ts_max_lines() -> usize {
    DEFAULT_TS_MAX_LINES
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuppressionsConfig {
    #[serde(default)]
    pub max: BTreeMap<String, usize>,
}

impl CheckConfig {
    /// Reads `baukit-check.toml` under `root`; a missing file yields the defaults.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, does not parse, or raises a limit above
    /// its default.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(CONFIG_FILE);
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        let config: Self =
            toml::from_str(&source).with_context(|| format!("invalid {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.file_size.rust_max_lines <= DEFAULT_RUST_MAX_LINES,
            "{CONFIG_FILE}: file-size rust-max-lines may only be lowered below {DEFAULT_RUST_MAX_LINES}"
        );
        ensure!(
            self.file_size.ts_max_lines <= DEFAULT_TS_MAX_LINES,
            "{CONFIG_FILE}: file-size ts-max-lines may only be lowered below {DEFAULT_TS_MAX_LINES}"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = CheckConfig::load(root.path())?;
        assert_eq!(config.file_size.rust_max_lines, DEFAULT_RUST_MAX_LINES);
        assert!(config.suppressions.max.is_empty());
        Ok(())
    }

    #[test]
    fn raising_a_limit_is_rejected() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join(CONFIG_FILE),
            "[file_size]\nts-max-lines = 501\n",
        )?;
        let error = CheckConfig::load(root.path())
            .err()
            .context("expected an error")?;
        assert!(error.to_string().contains("only be lowered"));
        Ok(())
    }

    #[test]
    fn unknown_keys_are_rejected() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join(CONFIG_FILE), "[file_size]\nmax = 2\n")?;
        assert!(CheckConfig::load(root.path()).is_err());
        Ok(())
    }
}

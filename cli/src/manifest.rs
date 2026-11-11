use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{
    components::Components, dockerfile, dockerfile::BackendManifest, doctor_layout::DoctorPaths,
    gate_settings, ports::PortDeclaration,
};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    Oidc,
    Clerk,
    Workos,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum QualityProfile {
    #[default]
    Standard,
    Strict,
}

/// Whether the strict gate compares the OpenAPI schema with the base revision.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenApiCompatibility {
    #[default]
    Off,
    Report,
    Enforce,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub template_version: String,
    #[serde(default)]
    pub port_offset: u32,
    #[serde(default)]
    pub ports: BTreeMap<String, PortDeclaration>,
    pub app: AppManifest,
    #[serde(default)]
    pub quality: QualityManifest,
    #[serde(default)]
    pub auth: AuthManifest,
    pub capabilities: Capabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<PlatformManifest>,
    pub dependencies: Dependencies,
    pub openapi: OpenApiPaths,
    #[serde(default, skip_serializing_if = "DoctorPaths::is_empty")]
    pub doctor: DoctorPaths,
    #[serde(default, skip_serializing_if = "BackendManifest::is_empty")]
    pub backend: BackendManifest,
}

/// Generation metadata only. Hub settings belong in backend runtime config.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformManifest {
    pub migration: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppManifest {
    pub name: String,
}

/// Auth generation settings that are not part of the provider choice.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthManifest {
    /// Keycloak login theme name the realm and the theme browser suite use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_theme: Option<String>,
}

pub const DEFAULT_LOGIN_THEME: &str = "baukit-accessible";

impl AuthManifest {
    /// # Errors
    ///
    /// Fails when the theme name has characters outside letters, digits, `.`, `_` and `-`.
    pub fn resolved_login_theme(&self) -> Result<&str> {
        let theme = self.login_theme.as_deref().unwrap_or(DEFAULT_LOGIN_THEME);
        let valid = theme
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
            && theme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        anyhow::ensure!(
            valid,
            "auth.login_theme `{theme}` must start with a letter or digit and hold only letters, digits, `.`, `_` and `-`"
        );
        Ok(theme)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QualityManifest {
    #[serde(default)]
    pub profile: QualityProfile,
    #[serde(default = "default_backend_coverage_lines")]
    pub backend_coverage_lines: u8,
    #[serde(default)]
    pub critical_paths: Vec<String>,
    #[serde(default = "default_webkit_repeats")]
    pub webkit_repeats: u8,
    #[serde(default)]
    pub full_stack_e2e: bool,
    #[serde(default)]
    pub openapi_compatibility: OpenApiCompatibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_emulator_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_emulators: Option<u8>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub android_wifi: bool,
    #[serde(default)]
    pub gate_budget_minutes: gate_settings::GateBudgetMinutes,
    #[serde(default)]
    pub affected: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub shared: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub light: Vec<String>,
    #[serde(default)]
    pub ci: gate_settings::CiSettings,
}

impl Default for QualityManifest {
    fn default() -> Self {
        Self {
            profile: QualityProfile::Standard,
            backend_coverage_lines: default_backend_coverage_lines(),
            critical_paths: Vec::new(),
            webkit_repeats: default_webkit_repeats(),
            full_stack_e2e: false,
            openapi_compatibility: OpenApiCompatibility::Off,
            android_emulator_port: None,
            android_emulators: None,
            android_wifi: false,
            gate_budget_minutes: gate_settings::GateBudgetMinutes::default(),
            affected: BTreeMap::new(),
            shared: Vec::new(),
            light: Vec::new(),
            ci: gate_settings::CiSettings::default(),
        }
    }
}

pub(crate) const fn default_backend_coverage_lines() -> u8 {
    70
}

pub(crate) const fn default_webkit_repeats() -> u8 {
    3
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Capabilities {
    #[serde(default)]
    pub analytics: AnalyticsAdapter,
    #[serde(flatten)]
    pub components: Components,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthProvider>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalyticsAdapter {
    Posthog,
    #[default]
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Dependencies {
    pub baukit: BaukitDependency,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum BaukitDependency {
    Path { path: String },
    Git { git: String, tag: String },
    Registry { version: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenApiPaths {
    pub schema: String,
    #[serde(default)]
    pub consumers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typescript: Option<String>,
}

impl OpenApiPaths {
    pub fn consumers(&self) -> Vec<&str> {
        if self.consumers.is_empty() {
            self.typescript.iter().map(String::as_str).collect()
        } else {
            self.consumers.iter().map(String::as_str).collect()
        }
    }
}

/// Reads and validates `baukit.toml` at `root`.
///
/// # Errors
///
/// Fails when the file cannot be read, is not valid TOML for [`Manifest`], or its
/// backend image section does not validate.
pub fn read_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join("baukit.toml");
    let source = fs::read_to_string(&path).with_context(|| {
        format!(
            "could not read {}; run this command at a generated product root",
            path.display()
        )
    })?;
    let manifest: Manifest =
        toml::from_str(&source).with_context(|| format!("could not parse {}", path.display()))?;
    dockerfile::validate(&manifest)?;
    Ok(manifest)
}

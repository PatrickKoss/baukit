use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{
    components::Components, dockerfile, dockerfile::BackendManifest, doctor_layout::DoctorPaths,
    gate_settings, gate_worker_caps::GateManifest, ports::PortDeclaration,
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
    #[serde(default, skip_serializing_if = "GateManifest::is_empty")]
    pub gate: GateManifest,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gitops: Option<GitopsManifest>,
}

pub const DEFAULT_GITOPS_DOMAIN: &str = "lab.example.com";
pub const DEFAULT_GITOPS_REGISTRY: &str = "ghcr.io/example";

/// Where the `deploy/gitops` overlays point: the lab domain, the image registry and the realm.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GitopsManifest {
    /// Testing serves `<app>-testing.<domain>`, production `<app>.<domain>`.
    pub domain: String,
    /// Image prefix; images are `<registry>/<app>-<process>`.
    pub registry: String,
    /// Keycloak base URL; defaults to `https://id.<domain>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_url: Option<String>,
    /// Realm name; defaults to the app name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
}

impl Default for GitopsManifest {
    fn default() -> Self {
        Self {
            domain: DEFAULT_GITOPS_DOMAIN.to_owned(),
            registry: DEFAULT_GITOPS_REGISTRY.to_owned(),
            identity_url: None,
            realm: None,
        }
    }
}

impl GitopsManifest {
    /// # Errors
    ///
    /// Fails when the domain or registry holds characters that are unsafe in hostnames, YAML or
    /// shell, or the identity URL is not https.
    pub fn validate(&self) -> Result<()> {
        let hostname = |value: &str| {
            !value.is_empty()
                && value.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'-')
                })
        };
        anyhow::ensure!(
            hostname(&self.domain),
            "gitops.domain `{}` must be a lowercase DNS name",
            self.domain
        );
        anyhow::ensure!(
            self.registry.split('/').all(hostname),
            "gitops.registry `{}` must be a lowercase registry path such as ghcr.io/owner",
            self.registry
        );
        if let Some(url) = &self.identity_url {
            let valid = url
                .strip_prefix("https://")
                .and_then(|rest| rest.split('/').next())
                .is_some_and(hostname)
                && url.bytes().all(|byte| {
                    byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\'' | b'`' | b'$' | b'\\')
                });
            anyhow::ensure!(
                valid,
                "gitops.identity_url `{url}` must be an https URL with a lowercase host"
            );
        }
        if let Some(realm) = &self.realm {
            anyhow::ensure!(
                hostname(realm),
                "gitops.realm `{realm}` must hold lowercase letters, digits, `.` and `-`"
            );
        }
        Ok(())
    }

    pub fn resolved_identity_url(&self) -> String {
        self.identity_url.clone().map_or_else(
            || format!("https://id.{}", self.domain),
            |url| url.trim_end_matches('/').to_owned(),
        )
    }
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shared_resources: BTreeMap<String, Vec<String>>,
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
            resources: BTreeMap::new(),
            shared_resources: BTreeMap::new(),
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
    manifest.gate.validate()?;
    Ok(manifest)
}

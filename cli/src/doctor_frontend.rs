use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{EXPECTED_TYPESCRIPT_DEPENDENCIES, manifest::Capabilities};

#[derive(Deserialize)]
pub(crate) struct PackageDependencies {
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "devDependencies")]
    dev_dependencies: BTreeMap<String, String>,
}

pub(crate) const EXPO_WEB_SHAPE_DEPENDENCY: &str = "expo-router";
/// Present in a Vite web app only; a web app that renders the Expo routes has its own client.
pub(crate) const VITE_WEB_ONLY_FILES: &[&str] = &["web/src/api.ts"];

/// True when `web/` is the Expo web build: a Vite app that renders the mobile
/// Expo Router routes, so sign-in and API calls run through the mobile code.
pub(crate) fn web_renders_expo_routes(root: &Path) -> Result<bool> {
    let package_path = root.join("web/package.json");
    if !package_path.is_file() {
        return Ok(false);
    }
    let package = fs::read_to_string(&package_path)
        .with_context(|| format!("could not read {}", package_path.display()))?;
    let sections: PackageDependencies = serde_json::from_str(&package)
        .with_context(|| format!("could not parse {}", package_path.display()))?;
    Ok(sections
        .dependencies
        .contains_key(EXPO_WEB_SHAPE_DEPENDENCY))
}

#[derive(Deserialize)]
pub(crate) struct PnpmWorkspace {
    #[serde(default)]
    packages: Vec<String>,
}

pub(crate) fn validate_pnpm_workspace(
    root: &Path,
    capability: &str,
    failures: &mut Vec<String>,
) -> Result<()> {
    if root.join(capability).join("pnpm-workspace.yaml").is_file() {
        return Ok(());
    }
    let path = root.join("pnpm-workspace.yaml");
    if !path.is_file() {
        failures.push(format!(
            "missing pnpm workspace: expected `{capability}/pnpm-workspace.yaml` or `pnpm-workspace.yaml`"
        ));
        return Ok(());
    }
    let workspace: PnpmWorkspace = serde_yaml_ng::from_str(&fs::read_to_string(path)?)
        .context("could not parse root pnpm-workspace.yaml")?;
    let mut included = false;
    let mut excluded = false;
    for package in workspace.packages {
        let (negative, pattern) = package
            .strip_prefix('!')
            .map_or((false, package.as_str()), |pattern| (true, pattern));
        let glob =
            globset::GlobBuilder::new(pattern.trim_start_matches("./").trim_end_matches('/'))
                .literal_separator(true)
                .build()
                .with_context(|| format!("invalid pnpm workspace package pattern `{package}`"))?
                .compile_matcher();
        if glob.is_match(capability) {
            if negative {
                excluded = true;
            } else {
                included = true;
            }
        }
    }
    if !included || excluded {
        failures.push(format!(
            "root pnpm-workspace.yaml packages do not include `{capability}`"
        ));
    }
    Ok(())
}

pub(crate) fn validate_frontend_capability(
    root: &Path,
    capability: &str,
    target_dependencies: &[&str],
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    validate_pnpm_workspace(root, capability, failures)?;
    let package_path = root.join(capability).join("package.json");
    if package_path.is_file() {
        let package = fs::read_to_string(&package_path)
            .with_context(|| format!("could not read {}", package_path.display()))?;
        let sections: PackageDependencies = serde_json::from_str(&package)
            .with_context(|| format!("could not parse {}", package_path.display()))?;
        if capability == "web" && sections.dependencies.contains_key("@baukit/auth-node") {
            failures.push(
                "web/package.json must use `@baukit/auth-node` only as a devDependency for Keycloak e2e".to_owned(),
            );
        }
        for dependency in EXPECTED_TYPESCRIPT_DEPENDENCIES
            .iter()
            .chain(target_dependencies)
        {
            if !sections.dependencies.contains_key(*dependency)
                && !sections.dev_dependencies.contains_key(*dependency)
            {
                failures.push(format!(
                    "{capability}/package.json is missing dependency `{dependency}`"
                ));
            }
        }
        successes.push(format!(
            "{capability} template files and Baukit dependencies are present"
        ));
    }
    Ok(())
}

/// The app that serves the PWA: the web app, or the Expo web export when the product has no web app.
pub(crate) fn pwa_app_directory(capabilities: &Capabilities) -> Option<&'static str> {
    if capabilities.web {
        Some("web")
    } else if capabilities.mobile {
        Some("mobile")
    } else {
        None
    }
}

pub(crate) fn validate_pwa_worker_build(
    root: &Path,
    app_directory: &str,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let initial_failure_count = failures.len();
    let package_path = root.join(app_directory).join("package.json");
    if package_path.is_file() {
        let package = fs::read_to_string(&package_path)
            .with_context(|| format!("could not read {}", package_path.display()))?;
        for (snippet, requirement) in [
            ("\"build:sw\"", "`build:sw` script"),
            ("\"build:sw:check\"", "`build:sw:check` script"),
            ("\"@baukit/pwa-web\"", "`@baukit/pwa-web` dependency"),
        ] {
            if !package.contains(snippet) {
                failures.push(format!(
                    "the PWA capability requires the {app_directory} {requirement}"
                ));
            }
        }
    }

    let script = format!("{app_directory}/scripts/build-sw.mjs");
    let script_path = root.join(&script);
    if !script_path.is_file() {
        failures.push(format!("the PWA capability requires `{script}`"));
    } else if !fs::read_to_string(&script_path)?.contains("@baukit/pwa-web/worker") {
        failures.push(format!(
            "{script} must copy the supported `@baukit/pwa-web/worker` artifact"
        ));
    }

    if failures.len() == initial_failure_count {
        successes.push(format!(
            "{app_directory} PWA worker build uses the supported Baukit artifact"
        ));
    }
    Ok(())
}

pub(crate) fn validate_mobile_router_configuration(
    root: &Path,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let package_path = root.join("mobile/package.json");
    if package_path.is_file() {
        let package = fs::read_to_string(&package_path)
            .with_context(|| format!("could not read {}", package_path.display()))?;
        if !package.contains("\"main\": \"expo-router/entry\"") {
            failures.push("mobile/package.json must set `main` to `expo-router/entry`".to_owned());
        }
    }

    let config_path = root.join("mobile/app.config.ts");
    if config_path.is_file() {
        let config = fs::read_to_string(&config_path)
            .with_context(|| format!("could not read {}", config_path.display()))?;
        if !config.contains("scheme:") {
            failures.push("mobile/app.config.ts must declare a deep-link scheme".to_owned());
        }
    }

    if failures.iter().all(|failure| {
        !failure.contains("expo-router/entry") && !failure.contains("deep-link scheme")
    }) {
        successes
            .push("mobile Expo Router entry point and deep-link scheme are configured".to_owned());
    }
    Ok(())
}

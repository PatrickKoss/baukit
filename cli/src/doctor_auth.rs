use std::{fs, path::Path};

use anyhow::Result;

use crate::{
    doctor_frontend::web_renders_expo_routes,
    manifest::{AuthProvider, Manifest},
};

pub(crate) fn validate_auth_provider(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let Some(provider) = manifest.capabilities.auth else {
        return Ok(());
    };
    let name = match provider {
        AuthProvider::Oidc => "oidc",
        AuthProvider::Clerk => "clerk",
        AuthProvider::Workos => "workos",
    };
    let before = failures.len();
    let config_path = root.join("config/local.toml");
    if config_path.is_file() {
        let config: toml::Value = toml::from_str(&fs::read_to_string(config_path)?)?;
        if let Some(actual) = config
            .get("auth")
            .and_then(|auth| auth.get("provider"))
            .and_then(toml::Value::as_str)
            && actual != name
        {
            failures.push(format!(
                "config/local.toml auth.provider is `{actual}`, but baukit.toml selects `{name}`"
            ));
        }
        if provider != AuthProvider::Oidc
            && let Some(mcp) = config.get("mcp")
            && (mcp.get("introspection_client_id").is_some()
                || mcp.get("introspection_client_secret").is_some())
        {
            failures.push(format!(
                "`{name}` does not support the generated Keycloak MCP introspection policy"
            ));
        }
    }
    let native_dependency = match provider {
        AuthProvider::Clerk => "@clerk/expo",
        _ => "@baukit/auth-native",
    };
    let web_dependency = if web_renders_expo_routes(root)? {
        native_dependency
    } else {
        match provider {
            AuthProvider::Clerk => "@clerk/clerk-js",
            AuthProvider::Workos => "@workos-inc/authkit-js",
            AuthProvider::Oidc => "@baukit/auth-web",
        }
    };
    for (enabled, directory, dependency) in [
        (
            manifest.capabilities.components.web(),
            "web",
            web_dependency,
        ),
        (
            manifest.capabilities.components.mobile(),
            "mobile",
            native_dependency,
        ),
    ] {
        if !enabled {
            continue;
        }
        let path = root.join(directory).join("package.json");
        if !path.is_file() {
            continue;
        }
        let package: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        if package["dependencies"][dependency].is_null() {
            failures.push(format!(
                "{directory}/package.json is missing `{dependency}` for auth provider `{name}`"
            ));
        }
    }
    if provider == AuthProvider::Workos && manifest.capabilities.components.mobile() {
        validate_workos_mobile_configuration(root, failures)?;
    }
    if failures.len() == before {
        successes.push(format!(
            "auth provider `{name}` matches config and frontend SDK wiring"
        ));
    }
    Ok(())
}

pub(crate) const WORKOS_MOBILE_SETTINGS: [&str; 3] = [
    "EXPO_PUBLIC_WORKOS_CLIENT_ID",
    "EXPO_PUBLIC_WORKOS_API_HOSTNAME",
    "EXPO_PUBLIC_WORKOS_REDIRECT_URI",
];
pub(crate) const MOBILE_ENV_FILES: [&str; 2] = ["mobile/.env.example", "mobile/.env"];

pub(crate) const QA_BUILD_FLAG: &str = "BAUKIT_QA_BUILD";

pub(crate) fn validate_mobile_qa_build_flag(root: &Path, failures: &mut Vec<String>) -> Result<()> {
    for relative in MOBILE_ENV_FILES {
        let path = root.join(relative);
        if path.is_file() && declares_env_assignment(&fs::read_to_string(path)?, QA_BUILD_FLAG) {
            failures.push(format!(
                "{relative} sets `{QA_BUILD_FLAG}`; Expo loads it into every export, so a production bundle would sign in locally. Set it only in the QA scripts' shell"
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_workos_mobile_configuration(
    root: &Path,
    failures: &mut Vec<String>,
) -> Result<()> {
    let config_path = root.join("mobile/app.config.ts");
    if config_path.is_file() {
        let config = fs::read_to_string(config_path)?;
        for setting in WORKOS_MOBILE_SETTINGS {
            if !config.contains(&format!("'{setting}'")) {
                failures.push(format!(
                    "mobile/app.config.ts does not read `{setting}` for WorkOS sign-in"
                ));
            }
        }
    }
    for relative in MOBILE_ENV_FILES {
        let path = root.join(relative);
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(path)?;
        if relative.ends_with(".example") {
            for setting in WORKOS_MOBILE_SETTINGS {
                if !declares_env_assignment(&source, setting) {
                    failures.push(format!("{relative} does not declare `{setting}`"));
                }
            }
        }
        if source.lines().any(carries_secret_api_key) {
            failures.push(format!(
                "{relative} carries a secret API key (`sk_`); the WorkOS mobile client is public and needs none"
            ));
        }
    }
    Ok(())
}

pub(crate) fn declares_env_assignment(source: &str, key: &str) -> bool {
    source.lines().any(|line| {
        line.trim_start()
            .trim_start_matches("export ")
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == key)
    })
}

pub(crate) fn carries_secret_api_key(line: &str) -> bool {
    let Some((_, value)) = line.split_once('=') else {
        return false;
    };
    let value = value.trim().trim_matches(['"', '\'']);
    value.starts_with("sk_test_") || value.starts_with("sk_live_")
}

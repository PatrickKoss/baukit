use std::path::{Component, Path};

use anyhow::Result;
use serde_yaml_ng::Value as Yaml;

use super::{CheckContext, Finding, Rule, relative, walk};

const NAME: &str = "security";
const SERVE_CALL: &str = "axum::serve(";
const CONNECT_INFO: &str = "into_make_service_with_connect_info";
const MERGE_CALL: &str = ".merge(";
const LIMITED_WRAPPERS: [&str; 2] = ["rate_limited(", "protect_routes("];
const BIN_CRATE_SUFFIX: &str = "-bin";
const CLUSTER_IP: &str = "ClusterIP";
const LOCAL_MARKER: &str = "local";
const CORS_KEY_MARKERS: [&str; 3] = ["allowedorigin", "corsorigin", "apiorigin"];
const SCHEMA_DOCUMENT_KEYS: [&str; 3] = ["openapi", "swagger", "$schema"];
const NULL_ORIGIN: &str = "null";
const HTTPS_SCHEME: &str = "https://";
const HTTP_SCHEME: &str = "http://";
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];
const DEFAULT_SEPARATOR: &str = ":-";
const TEST_DIRECTORIES: [&str; 5] = ["tests", "benches", "examples", "e2e", "baukit-test"];
const TEST_CRATE_SUFFIX: &str = "-test-support";
const TEST_MODULE: &str = "#[cfg(test)]";

/// Production wiring the composed API depends on: client IPs reach the limiter, every merged
/// router is rate limited, the ops port stays inside the cluster and CORS origins are strict.
pub struct Security;

impl Rule for Security {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        let mut findings = Vec::new();
        for path in walk::files(context.root, is_candidate)? {
            let Some(text) = walk::read_text(&path)? else {
                continue;
            };
            let shown = relative(context.root, &path);
            findings.extend(check_file(&path, &shown, &text));
        }
        Ok(findings)
    }
}

fn is_candidate(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("rs" | "yaml" | "yml" | "toml")
    )
}

fn check_file(path: &Path, shown: &str, text: &str) -> Vec<Finding> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") if !is_test_code(path) => rust_findings(path, shown, text),
        Some("yaml" | "yml") => yaml_findings(shown, text),
        Some("toml") => toml_findings(shown, text),
        _ => Vec::new(),
    }
}

fn is_test_code(path: &Path) -> bool {
    let name = walk::file_name(path);
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || path.components().any(|component| {
            component.as_os_str().to_str().is_some_and(|name| {
                TEST_DIRECTORIES.contains(&name) || name.ends_with(TEST_CRATE_SUFFIX)
            })
        })
}

fn rust_findings(path: &Path, shown: &str, text: &str) -> Vec<Finding> {
    let text = text.find(TEST_MODULE).map_or(text, |end| &text[..end]);
    let mut findings = serve_findings(shown, text);
    if is_in_bin_crate(path) {
        findings.extend(merge_findings(shown, text));
    }
    findings
}

fn serve_findings(shown: &str, text: &str) -> Vec<Finding> {
    text.match_indices(SERVE_CALL)
        .filter(|(offset, _)| {
            let arguments = call_arguments(text, offset + SERVE_CALL.len());
            !arguments.contains(CONNECT_INFO)
        })
        .map(|(offset, _)| {
            Finding::error(
                NAME,
                shown,
                "`axum::serve` without `into_make_service_with_connect_info::<SocketAddr>()`; the IP rate limiter cannot see the client address",
            )
            .at_line(line_of(text, offset))
        })
        .collect()
}

fn merge_findings(shown: &str, text: &str) -> Vec<Finding> {
    text.match_indices(MERGE_CALL)
        .filter(|(offset, _)| {
            let argument: String = call_arguments(text, offset + MERGE_CALL.len())
                .split_whitespace()
                .collect();
            !LIMITED_WRAPPERS
                .iter()
                .any(|wrapper| argument.starts_with(wrapper))
        })
        .map(|(offset, _)| {
            Finding::error(
                NAME,
                shown,
                "`.merge(x)` in a bin crate must wrap x in `rate_limited(` or `protect_routes(`",
            )
            .at_line(line_of(text, offset))
        })
        .collect()
}

fn is_in_bin_crate(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name)
            if name.to_str().is_some_and(|name| name.ends_with(BIN_CRATE_SUFFIX)))
    })
}

/// The text of a call's arguments, from just after `(` to the matching `)`.
fn call_arguments(text: &str, start: usize) -> &str {
    let rest = text.get(start..).unwrap_or_default();
    let mut depth = 1_usize;
    for (offset, character) in rest.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return rest.get(..offset).unwrap_or(rest);
                }
            }
            _ => {}
        }
    }
    rest
}

fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(0, |before| before.matches('\n').count())
        + 1
}

fn yaml_findings(shown: &str, text: &str) -> Vec<Finding> {
    let Ok(document) = serde_yaml_ng::from_str::<Yaml>(text) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    if let Some(kind) = document
        .get("opsService")
        .and_then(|service| service.get("type"))
        .and_then(Yaml::as_str)
        && kind != CLUSTER_IP
    {
        findings.push(Finding::error(
            NAME,
            shown,
            format!("opsService.type is `{kind}`; the ops port must stay `ClusterIP`"),
        ));
    }
    if is_schema_document(&document) {
        return findings;
    }
    let mut origins = Vec::new();
    yaml_origins(&document, false, &mut origins);
    findings.extend(origin_findings(shown, &origins));
    findings
}

fn is_schema_document(document: &Yaml) -> bool {
    SCHEMA_DOCUMENT_KEYS
        .iter()
        .any(|key| document.get(key).is_some())
}

fn yaml_origins(value: &Yaml, under_origin_key: bool, origins: &mut Vec<String>) {
    match value {
        Yaml::String(text) if under_origin_key => origins.push(text.clone()),
        Yaml::Sequence(items) => {
            for item in items {
                yaml_origins(item, under_origin_key, origins);
            }
        }
        Yaml::Mapping(entries) => {
            for (key, entry) in entries {
                let is_origin = key.as_str().is_some_and(is_origin_key);
                yaml_origins(entry, is_origin, origins);
            }
        }
        _ => {}
    }
}

fn toml_findings(shown: &str, text: &str) -> Vec<Finding> {
    let Ok(document) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut origins = Vec::new();
    toml_origins(&toml::Value::Table(document), false, &mut origins);
    origin_findings(shown, &origins)
}

fn toml_origins(value: &toml::Value, under_origin_key: bool, origins: &mut Vec<String>) {
    match value {
        toml::Value::String(text) if under_origin_key => origins.push(text.clone()),
        toml::Value::Array(items) => {
            for item in items {
                toml_origins(item, under_origin_key, origins);
            }
        }
        toml::Value::Table(entries) => {
            for (key, entry) in entries {
                toml_origins(entry, is_origin_key(key), origins);
            }
        }
        _ => {}
    }
}

/// Matches CORS origin settings such as `allowedOrigins`, `cors_allowed_origins`,
/// `HTTP__CORS_ALLOWED_ORIGINS` and `publicApiOrigin`, but not a bare `origin` field.
fn is_origin_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();
    CORS_KEY_MARKERS
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn origin_findings(shown: &str, origins: &[String]) -> Vec<Finding> {
    let local = shown.to_ascii_lowercase().contains(LOCAL_MARKER);
    origins
        .iter()
        .flat_map(|value| origin_list(value))
        .filter_map(|origin| origin_problem(&origin, local))
        .map(|message| Finding::error(NAME, shown, message))
        .collect()
}

/// Splits a setting into origins: a JSON array string or a comma-separated list, each with
/// `${VAR:-default}` resolved to its default.
fn origin_list(value: &str) -> Vec<String> {
    let Some(resolved) = resolve_defaults(value.trim()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&resolved).unwrap_or_else(|_| {
        resolved
            .split(',')
            .map(|item| item.trim().trim_matches(['"', '\'']).to_owned())
            .collect()
    })
}

/// Replaces every `${VAR:-default}` with its default. A whole value that is a bare `${VAR}` or
/// `${VAR:?message}` is unset; inside a longer value such an interpolation resolves to nothing,
/// which leaves the scheme and host to judge.
fn resolve_defaults(value: &str) -> Option<String> {
    let mut resolved = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let (before, interpolation) = rest.split_at(start);
        let end = interpolation.find('}')?;
        let default = interpolation[2..end]
            .split_once(DEFAULT_SEPARATOR)
            .map(|(_, default)| default);
        let whole = before.is_empty() && end + 1 == interpolation.len();
        if default.is_none() && whole {
            return None;
        }
        resolved.push_str(before);
        resolved.push_str(default.unwrap_or_default());
        rest = &interpolation[end + 1..];
    }
    resolved.push_str(rest);
    Some(resolved)
}

fn is_loopback(origin: &str) -> bool {
    let Some(authority) = origin.strip_prefix(HTTP_SCHEME) else {
        return false;
    };
    let authority = authority.trim_end_matches('/');
    LOOPBACK_HOSTS.iter().any(|host| {
        authority
            .strip_prefix(host)
            .is_some_and(|port| port.is_empty() || port.starts_with(':'))
    })
}

fn origin_problem(origin: &str, local: bool) -> Option<String> {
    let trimmed = origin.trim();
    if trimmed.is_empty() || is_loopback(trimmed) {
        return None;
    }
    if trimmed.eq_ignore_ascii_case(NULL_ORIGIN) {
        return Some("origin `null` matches sandboxed and file:// pages; list real origins".into());
    }
    if !local && !trimmed.starts_with(HTTPS_SCHEME) {
        return Some(format!(
            "origin `{trimmed}` is not https; plain http origins belong only in local config"
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(path: &str, text: &str) -> Vec<String> {
        check_file(Path::new(path), path, text)
            .into_iter()
            .map(|finding| finding.to_string())
            .collect()
    }

    #[test]
    fn serve_without_connect_info_is_an_error() {
        let bad = "fn f() {\n    axum::serve(listener, app).await\n}";
        assert_eq!(
            messages("src/main.rs", bad),
            [
                "security: src/main.rs:2: `axum::serve` without `into_make_service_with_connect_info::<SocketAddr>()`; the IP rate limiter cannot see the client address"
            ]
        );
        let good = "axum::serve(\n    listener,\n    app.into_make_service_with_connect_info::<SocketAddr>(),\n)";
        assert!(messages("src/main.rs", good).is_empty());
        assert!(messages("tests/serve.rs", bad).is_empty());
        assert!(messages("src/tests.rs", bad).is_empty());
        assert!(messages("src/idp_tests.rs", bad).is_empty());
        assert!(
            messages(
                "src/lib.rs",
                &format!("#[cfg(test)]\nmod tests {{\n{bad}\n}}")
            )
            .is_empty()
        );
        assert!(messages("rust/crates/baukit-test/src/jwt.rs", bad).is_empty());
        assert!(messages("backend/crates/shop-test-support/src/lib.rs", bad).is_empty());
        assert!(messages("e2e/fake-product/src/main.rs", bad).is_empty());
    }

    #[test]
    fn bin_crate_merges_must_be_rate_limited() {
        let source = [
            "let api = api.merge(rate_limited(mcp, limits));",
            "let api = api.merge(\n    protect_routes(x, late, auth, limits)?,\n);",
            "let api = api.merge(platform.router);",
        ]
        .join("\n");
        assert_eq!(
            messages("crates/shop-bin/src/compose.rs", &source),
            [
                "security: crates/shop-bin/src/compose.rs:5: `.merge(x)` in a bin crate must wrap x in `rate_limited(` or `protect_routes(`"
            ]
        );
        assert!(messages("crates/shop-api/src/lib.rs", &source).is_empty());
    }

    #[test]
    fn ops_service_must_be_cluster_ip() {
        assert_eq!(
            messages("deploy/values.yaml", "opsService:\n  type: NodePort\n"),
            [
                "security: deploy/values.yaml: opsService.type is `NodePort`; the ops port must stay `ClusterIP`"
            ]
        );
        assert!(messages("deploy/values.yaml", "opsService:\n  type: ClusterIP\n").is_empty());
    }

    #[test]
    fn origins_are_https_outside_local_config_and_never_null() {
        let values = "browserSession:\n  allowedOrigins: [http://app.example.com, https://ok.example.com, \"null\"]\n  publicApiOrigin: ~\n";
        assert_eq!(
            messages("deploy/values.yaml", values),
            [
                "security: deploy/values.yaml: origin `http://app.example.com` is not https; plain http origins belong only in local config",
                "security: deploy/values.yaml: origin `null` matches sandboxed and file:// pages; list real origins",
            ]
        );
        let local = "[http]\ncors_allowed_origins = [\"http://localhost:5173\"]\n";
        assert!(messages("config/local.toml", local).is_empty());
        let remote = "[http]\ncors_allowed_origins = [\"http://app.example.com\"]\n";
        assert!(messages("config/local.toml", remote).is_empty());
        assert_eq!(
            messages("config/production.toml", remote),
            [
                "security: config/production.toml: origin `http://app.example.com` is not https; plain http origins belong only in local config"
            ]
        );
        assert_eq!(
            messages(
                "config/local.toml",
                "[http]\ncors_allowed_origins = [\"null\"]\n"
            ),
            [
                "security: config/local.toml: origin `null` matches sandboxed and file:// pages; list real origins"
            ]
        );
    }

    #[test]
    fn loopback_http_origins_pass_everywhere() {
        for origin in [
            "http://localhost:5173",
            "http://127.0.0.1",
            "http://[::1]:8080",
        ] {
            let config = format!("[http]\ncors_allowed_origins = [\"{origin}\"]\n");
            assert!(messages("config/production.toml", &config).is_empty());
        }
        assert_eq!(
            messages(
                "config/production.toml",
                "[http]\ncors_allowed_origins = [\"http://localhost.evil.com\"]\n"
            ),
            [
                "security: config/production.toml: origin `http://localhost.evil.com` is not https; plain http origins belong only in local config"
            ]
        );
    }

    #[test]
    fn compose_env_origins_resolve_their_default() {
        let compose = "services:\n  api:\n    environment:\n      APP__HTTP__CORS_ALLOWED_ORIGINS: '[\"${APP_URL:-http://localhost:5173}\"]'\n";
        assert!(messages("compose.yaml", compose).is_empty());
        let remote = compose.replace("http://localhost:5173", "http://app.example.com");
        assert_eq!(
            messages("compose.yaml", &remote),
            [
                "security: compose.yaml: origin `http://app.example.com` is not https; plain http origins belong only in local config"
            ]
        );
        assert!(
            messages(
                "compose.yaml",
                &compose.replace(":-http://localhost:5173", "")
            )
            .is_empty()
        );
    }

    #[test]
    fn json_array_and_comma_separated_origins_are_judged_per_element() {
        let json = "env:\n  HTTP__CORS_ALLOWED_ORIGINS: '[\"https://app.lab.example.de\"]'\n";
        assert!(messages("deploy/gitops/testing/environment.yaml", json).is_empty());
        let listed =
            "env:\n  CORS__ALLOWED_ORIGINS: https://a.example.com, http://b.example.com,null\n";
        assert_eq!(
            messages("deploy/environment.yaml", listed),
            [
                "security: deploy/environment.yaml: origin `http://b.example.com` is not https; plain http origins belong only in local config",
                "security: deploy/environment.yaml: origin `null` matches sandboxed and file:// pages; list real origins",
            ]
        );
    }

    #[test]
    fn schema_fields_named_origin_are_not_cors_config() {
        let openapi = "openapi: 3.0.1\ncomponents:\n  schemas:\n    Flow:\n      properties:\n        origin:\n          type: string\n      example:\n        origin: origin\n        allowedOrigins: origin\n";
        assert!(messages("packages/sdk-go/api/openapi.yaml", openapi).is_empty());
        assert!(messages("deploy/data.yaml", "flow:\n  origin: origin\n").is_empty());
    }
}

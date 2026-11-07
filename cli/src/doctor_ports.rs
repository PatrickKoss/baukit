use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{
    doctor_dev_ports, doctor_layout,
    manifest::{AuthProvider, Manifest},
    ports,
    ports::{PortConfiguration, PortDeclaration},
};

#[derive(PartialEq)]
pub(crate) enum MakefileAssignment {
    Default,
    Immediate,
    Recursive,
}

pub(crate) fn makefile_assignment(line: &str) -> Option<(&str, MakefileAssignment, &str)> {
    if line.starts_with('\t') {
        return None;
    }
    let line = line.split('#').next()?.trim();
    let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
    let (binding, value) = line.split_once('=')?;
    let binding = binding.trim_end();
    let (name, assignment) = if let Some(name) = binding.strip_suffix('?') {
        (name, MakefileAssignment::Default)
    } else if let Some(name) = binding.strip_suffix(':') {
        (name, MakefileAssignment::Immediate)
    } else {
        (binding, MakefileAssignment::Recursive)
    };
    let name = name.trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
    {
        return None;
    }
    Some((name, assignment, value.trim()))
}

pub(crate) fn expand_makefile_variables(
    source: &str,
    variables: &BTreeMap<String, String>,
    expanding: &mut BTreeSet<String>,
) -> String {
    let mut resolved = String::new();
    let mut remaining = source;
    while let Some((prefix, suffix)) = remaining.split_once('$') {
        resolved.push_str(prefix);
        if let Some(suffix) = suffix.strip_prefix('$') {
            resolved.push_str("$$");
            remaining = suffix;
            continue;
        }
        let delimiter = match suffix.chars().next() {
            Some('(') => ')',
            Some('{') => '}',
            _ => {
                resolved.push('$');
                remaining = suffix;
                continue;
            }
        };
        let Some((name, suffix)) = suffix[1..].split_once(delimiter) else {
            resolved.push('$');
            remaining = suffix;
            break;
        };
        let reference_length = remaining.len() - suffix.len();
        if let Some(value) = variables
            .get(name)
            .filter(|_| expanding.insert(name.to_owned()))
        {
            resolved.push_str(&expand_makefile_variables(value, variables, expanding));
            expanding.remove(name);
        } else {
            resolved.push_str(&remaining[prefix.len()..reference_length]);
        }
        remaining = suffix;
    }
    resolved.push_str(remaining);
    resolved
}

pub(crate) fn resolve_makefile_variables(source: &str) -> String {
    let mut variables = BTreeMap::new();
    for (name, assignment, value) in source.lines().filter_map(makefile_assignment) {
        if assignment == MakefileAssignment::Default && variables.contains_key(name) {
            continue;
        }
        let value = if assignment == MakefileAssignment::Immediate {
            expand_makefile_variables(value, &variables, &mut BTreeSet::new())
        } else {
            value.to_owned()
        };
        variables.insert(name.to_owned(), value);
    }
    expand_makefile_variables(source, &variables, &mut BTreeSet::new())
}

pub(crate) fn validate_port_configuration(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let ports = match PortConfiguration::from_manifest(manifest) {
        Ok(ports) => ports,
        Err(error) => {
            failures.push(format!("invalid port configuration: {error}"));
            return Ok(());
        }
    };
    doctor_dev_ports::validate(root, manifest, &ports, successes, failures)?;
    if manifest.port_offset == 0
        && manifest
            .ports
            .keys()
            .all(|name| ports::is_dev_port(name) || name == "keycloak_extra")
    {
        successes.push("no port offset is configured".to_owned());
        return Ok(());
    }
    let initial_failure_count = failures.len();
    let mut expected = Vec::new();
    if manifest.capabilities.components.backend() {
        let redis_port = validate_compose_ports(root, manifest, &ports, failures)?;
        expected.extend([
            ("Makefile", format!("HTTP__PORT={}", ports.api)),
            ("Makefile", format!("OPS__PORT={}", ports.ops)),
            (
                "deploy/values.yaml",
                format!(
                    "http: {}",
                    manifest
                        .ports
                        .get("api")
                        .and_then(PortDeclaration::as_mapping)
                        .and_then(|port| port.container)
                        .unwrap_or(ports.api)
                ),
            ),
            (
                "deploy/values.yaml",
                format!(
                    "ops: {}",
                    manifest
                        .ports
                        .get("ops")
                        .and_then(PortDeclaration::as_mapping)
                        .and_then(|port| port.container)
                        .unwrap_or(ports.ops)
                ),
            ),
        ]);
        if doctor_layout::uses_redis(root, manifest)? {
            expected.push((
                "Makefile",
                format!("REDIS_URL=redis://127.0.0.1:{redis_port}/"),
            ));
        }
        if uses_bundled_keycloak(root, manifest)? {
            expected.extend([
                ("Makefile", format!("localhost:{}/realms/", ports.keycloak)),
                ("scripts/pkce-login.py", format!("localhost:{}", ports.api)),
            ]);
        }
    }
    if manifest.capabilities.components.mobile() {
        expected.extend([
            ("mobile/.env.example", format!("localhost:{}", ports.api)),
            ("mobile/src/api.ts", format!("localhost:{}", ports.api)),
        ]);
        if oidc_clients_use_bundled_keycloak(root, manifest)? {
            expected.extend([
                (
                    "mobile/.env.example",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
                (
                    "mobile/app.config.ts",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
                (
                    "mobile/src/auth.ts",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
                (
                    "mobile/src/delete-profile.ts",
                    format!("localhost:{}", ports.api),
                ),
            ]);
        }
    }
    if manifest.capabilities.components.web() {
        expected.extend([
            ("web/.env.example", format!("localhost:{}", ports.api)),
            ("web/src/api.ts", format!("localhost:{}", ports.api)),
        ]);
        if oidc_clients_use_bundled_keycloak(root, manifest)? {
            expected.extend([
                (
                    "web/.env.example",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
                (
                    "web/src/auth.ts",
                    format!("localhost:{}/realms/", ports.keycloak),
                ),
                (
                    "web/src/delete-profile.ts",
                    format!("localhost:{}", ports.api),
                ),
                (
                    "web/e2e/stack/keycloak.ts",
                    format!("localhost:{}", ports.keycloak),
                ),
            ]);
        }
    }
    if provider_clients_use_bundled_keycloak(root, manifest)? {
        let realm = format!("localhost:{}/realms/", ports.keycloak);
        if manifest.capabilities.components.web() {
            expected.push(("web/.env.example", realm.clone()));
        }
        if manifest.capabilities.components.mobile() {
            expected.push(("mobile/.env.example", realm));
            let qa_port = format!("{QA_KEYCLOAK_PORT_VARIABLE}{QA_KEYCLOAK_PORT}");
            for script in QA_KEYCLOAK_SCRIPTS {
                expected.push((script, qa_port.clone()));
            }
        }
    }
    let mut expected = expected
        .into_iter()
        .map(|(path, snippet)| (path.to_owned(), snippet))
        .collect::<Vec<_>>();
    if manifest.capabilities.components.backend()
        && oidc_clients_use_bundled_keycloak(root, manifest)?
    {
        for krate in doctor_layout::rust_crates(root, manifest)? {
            for path in krate.sources {
                let source = fs::read_to_string(&path)?;
                let relative = path.strip_prefix(root)?.to_string_lossy().into_owned();
                if source.contains("identity_admin_realm:") {
                    expected.push((
                        relative.clone(),
                        format!("localhost:{}/realms/", ports.keycloak),
                    ));
                }
                if source.contains("identity_admin_base_url:") {
                    expected.push((relative, format!("localhost:{}", ports.keycloak)));
                }
            }
        }
    }
    for (relative, snippet) in expected {
        let relative = relative.replace("PLACEHOLDER", &manifest.app.name);
        let path = root.join(&relative);
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(&path)?;
        let source = if relative == "Makefile" {
            resolve_makefile_variables(&source)
        } else {
            source
        };
        let mismatched = if snippet.starts_with(QA_KEYCLOAK_PORT_VARIABLE) {
            has_mismatched_qa_keycloak_port(&source, &ports)
        } else if relative == "Makefile" && snippet.starts_with("REDIS_URL=") {
            let expected_port = loopback_ports(&snippet, false)
                .first()
                .copied()
                .context("expected Redis port")?;
            let redis_sources = source
                .split("redis://")
                .skip(1)
                .map(|tail| {
                    tail.split(|character: char| {
                        character.is_whitespace() || matches!(character, '}' | '\'' | '"')
                    })
                    .next()
                    .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("\n");
            let actual_ports = loopback_ports(&redis_sources, false);
            actual_ports.is_empty() || actual_ports.iter().any(|port| *port != expected_port)
        } else if relative.ends_with(".rs")
            || is_environment_capable_source(&relative)
            || (relative == "Makefile" && snippet.contains("/realms/"))
        {
            let expected_port = localhost_port(&snippet).expect("expected snippet has a port");
            let source = source.replace("${KEYCLOAK_PORT}", &ports.keycloak.to_string());
            let source = if relative.ends_with(".rs") && !snippet.contains("/realms/") {
                source
                    .split("identity_admin_base_url:")
                    .skip(1)
                    .filter_map(|field| field.split(',').next())
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                source
            };
            if matches!(relative.as_str(), "mobile/src/api.ts" | "web/src/api.ts") {
                let issuer = snippet.contains("/realms/");
                loopback_ports(&source, issuer).iter().any(|port| {
                    *port != expected_port && !(issuer && ports.keycloak_extra.contains(port))
                })
            } else {
                has_mismatched_loopback_port(
                    &source,
                    expected_port,
                    &ports.keycloak_extra,
                    snippet.contains("/realms/"),
                )
            }
        } else {
            !source.contains(&snippet)
        };
        if mismatched {
            failures.push(if manifest.ports.is_empty() {
                format!("generated file `{relative}` does not use port offset {}", manifest.port_offset)
            } else {
                format!("generated file `{relative}` does not match declared ports (expected `{snippet}`)")
            });
        }
    }
    if failures.len() == initial_failure_count {
        successes.push(if manifest.ports.is_empty() {
            format!("generated files use port offset {}", manifest.port_offset)
        } else {
            "generated files use declared host and container ports".to_owned()
        });
    }
    Ok(())
}

pub(crate) fn is_environment_capable_source(relative: &str) -> bool {
    matches!(
        relative,
        "mobile/.env.example"
            | "web/.env.example"
            | "mobile/src/api.ts"
            | "mobile/src/auth.ts"
            | "mobile/src/delete-profile.ts"
            | "mobile/app.config.ts"
            | "web/src/api.ts"
            | "web/src/auth.ts"
            | "web/src/delete-profile.ts"
            | "web/e2e/stack/keycloak.ts"
            | "scripts/pkce-login.py"
    ) || relative.ends_with("-bin/src/lib.rs")
}

#[derive(Deserialize)]
pub(crate) struct ComposeFile {
    #[serde(default)]
    services: BTreeMap<String, ComposeService>,
}

#[derive(Deserialize)]
pub(crate) struct ComposeService {
    #[serde(default)]
    ports: Vec<ComposePort>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum ComposePort {
    Short(String),
    ContainerOnly(u16),
    Long {
        target: PublishedPort,
        published: Option<PublishedPort>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum PublishedPort {
    Number(u16),
    Text(String),
}

impl PublishedPort {
    fn literal(&self) -> Option<u16> {
        match self {
            Self::Number(port) => Some(*port),
            Self::Text(port) => port.parse().ok(),
        }
    }
}

pub(crate) fn compose_parameter_default(port: &str) -> Option<u16> {
    let parameter = port.rsplit_once("${")?.1.strip_suffix('}')?;
    let (_, value) = parameter
        .split_once(":-")
        .or_else(|| parameter.split_once('-'))?;
    value.parse().ok()
}

impl ComposePort {
    fn literal_mapping(&self) -> Option<(u16, u16)> {
        match self {
            Self::Short(mapping) => {
                let (published, target) = mapping.rsplit_once(':')?;
                let target = target.split('/').next()?.parse().ok()?;
                let published = published.rsplit(':').next()?.parse().ok()?;
                Some((target, published))
            }
            Self::Long { target, published } => {
                Some((target.literal()?, published.as_ref()?.literal()?))
            }
            Self::ContainerOnly(_port) => None,
        }
    }

    fn parameterized_default_mapping(&self) -> Option<(u16, u16)> {
        match self {
            Self::Short(mapping) => {
                let (published, target) = mapping.rsplit_once(':')?;
                let target = target.split('/').next()?.parse().ok()?;
                Some((target, compose_parameter_default(published)?))
            }
            Self::Long {
                target,
                published: Some(PublishedPort::Text(published)),
            } => Some((target.literal()?, compose_parameter_default(published)?)),
            Self::Long { .. } | Self::ContainerOnly(_) => None,
        }
    }
}

pub(crate) fn validate_compose_ports(
    root: &Path,
    manifest: &Manifest,
    ports: &PortConfiguration,
    failures: &mut Vec<String>,
) -> Result<u16> {
    let path = root.join("compose.yaml");
    if !path.is_file() {
        return Ok(ports.redis);
    }
    let compose: ComposeFile = serde_yaml_ng::from_str(&fs::read_to_string(&path)?)
        .context("could not parse compose.yaml")?;
    let mut mappings = vec![("postgres", 5432, ports.postgres)];
    if uses_bundled_keycloak(root, manifest)? {
        mappings.push(("keycloak", 8080, ports.keycloak));
    }
    if doctor_layout::uses_redis(root, manifest)? {
        mappings.push(("redis", 6379, ports.redis));
    }
    for (name, declaration) in &manifest.ports {
        let Some(mapping) = declaration.as_mapping() else {
            continue;
        };
        let Some(container) = mapping.container else {
            continue;
        };
        let default_service = match name.as_str() {
            "api" | "ops" => {
                if compose.services.contains_key("backend") {
                    "backend"
                } else {
                    "api"
                }
            }
            "fake_provider" => "fake-provider",
            name => name,
        };
        let service_name = mapping.service.as_deref().unwrap_or(default_service);
        let Some(service) = compose.services.get(service_name) else {
            continue;
        };
        let actual = service
            .ports
            .iter()
            .filter_map(|port| {
                port.literal_mapping()
                    .or_else(|| port.parameterized_default_mapping())
            })
            .collect::<Vec<_>>();
        let expected = (container, mapping.host);
        if !actual.contains(&expected)
            || actual
                .iter()
                .any(|port| (port.0 == container || port.1 == mapping.host) && *port != expected)
        {
            failures.push(format!("compose.yaml service `{service_name}` must publish declared ports.{} host {} to container {}", name, mapping.host, container));
        }
    }
    for (service_name, target, expected) in mappings {
        if manifest.ports.contains_key(service_name) {
            continue;
        }
        let Some(service) = compose.services.get(service_name) else {
            continue;
        };
        if service
            .ports
            .iter()
            .filter_map(ComposePort::literal_mapping)
            .any(|(container, published)| {
                (container == target && published != expected)
                    || (published == expected && container != target)
            })
        {
            failures.push(format!(
                "generated file `compose.yaml` does not use port offset {} for container port {target}",
                manifest.port_offset
            ));
        }
    }
    let redis_port = compose
        .services
        .get(
            manifest
                .ports
                .get("redis")
                .and_then(PortDeclaration::as_mapping)
                .and_then(|mapping| mapping.service.as_deref())
                .unwrap_or("redis"),
        )
        .and_then(|service| {
            service
                .ports
                .iter()
                .filter_map(ComposePort::parameterized_default_mapping)
                .find_map(|(target, published)| (target == 6379).then_some(published))
        })
        .unwrap_or(ports.redis);
    Ok(redis_port)
}

pub(crate) fn localhost_port(snippet: &str) -> Option<u16> {
    snippet
        .split_once("localhost:")?
        .1
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

pub(crate) fn loopback_ports(source: &str, issuer: bool) -> Vec<u16> {
    ["localhost:", "127.0.0.1:", "[::1]:"]
        .into_iter()
        .flat_map(|marker| source.match_indices(marker))
        .filter_map(|(index, marker)| {
            let suffix = &source[index + marker.len()..];
            let length = suffix.bytes().take_while(u8::is_ascii_digit).count();
            if suffix[length..].starts_with("/realms/") != issuer {
                return None;
            }
            suffix[..length].parse::<u16>().ok()
        })
        .collect()
}

pub(crate) fn has_mismatched_loopback_port(
    source: &str,
    expected_port: u16,
    extra_issuer_ports: &[u16],
    issuer: bool,
) -> bool {
    let ports = loopback_ports(source, issuer);
    if issuer {
        ports
            .iter()
            .any(|port| *port != expected_port && !extra_issuer_ports.contains(port))
    } else {
        !ports.is_empty() && !ports.contains(&expected_port)
    }
}

pub(crate) const QA_KEYCLOAK_PORT_VARIABLE: &str = "BAUKIT_QA_KEYCLOAK_PORT:-";
pub(crate) const QA_KEYCLOAK_PORT: u16 = 18081;
pub(crate) const QA_KEYCLOAK_SCRIPTS: [&str; 5] = [
    "mobile/scripts/qa/docker-compose.qa.yml",
    "mobile/scripts/qa/services.sh",
    "mobile/scripts/qa/build-android.sh",
    "mobile/scripts/qa/build-ios.sh",
    "mobile/scripts/qa/android-env.sh",
];

pub(crate) const KEYCLOAK_PORT_VARIABLE_SUFFIX: &str = "_KEYCLOAK_PORT";

/// The defaults of `${NAME_KEYCLOAK_PORT:-<port>}` expansions. A default that
/// nests another expansion is skipped, since the inner one carries the port.
pub(crate) fn qa_keycloak_port_defaults(source: &str) -> Vec<Option<u16>> {
    source
        .match_indices("${")
        .filter_map(|(start, _)| {
            let (name, default) = source[start + 2..].split_once(":-")?;
            let keycloak_port_variable = name.ends_with(KEYCLOAK_PORT_VARIABLE_SUFFIX)
                && name.chars().all(|character| {
                    character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
                });
            if !keycloak_port_variable || default.starts_with("${") {
                return None;
            }
            let digits = default
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            Some(digits.parse::<u16>().ok())
        })
        .collect()
}

pub(crate) fn has_mismatched_qa_keycloak_port(source: &str, declared: &PortConfiguration) -> bool {
    let accepted = |port: u16| {
        port == QA_KEYCLOAK_PORT
            || port == declared.keycloak
            || declared.keycloak_extra.contains(&port)
    };
    let ports = qa_keycloak_port_defaults(source);
    ports.is_empty() || ports.iter().any(|port| !port.is_some_and(accepted))
}

pub(crate) fn provider_clients_use_bundled_keycloak(
    root: &Path,
    manifest: &Manifest,
) -> Result<bool> {
    Ok(matches!(
        manifest.capabilities.auth,
        Some(AuthProvider::Clerk | AuthProvider::Workos)
    ) && uses_bundled_keycloak(root, manifest)?)
}

pub(crate) fn oidc_clients_use_bundled_keycloak(root: &Path, manifest: &Manifest) -> Result<bool> {
    Ok(manifest.capabilities.auth == Some(AuthProvider::Oidc)
        && uses_bundled_keycloak(root, manifest)?)
}

pub(crate) fn uses_bundled_keycloak(root: &Path, manifest: &Manifest) -> Result<bool> {
    if manifest.capabilities.auth.is_none() {
        return Ok(false);
    }
    let path = root.join("config/local.toml");
    if !path.is_file() {
        return Ok(true);
    }
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(path)?).context("could not parse config/local.toml")?;
    let issuer = config
        .get("auth")
        .and_then(|auth| auth.get("issuer"))
        .and_then(toml::Value::as_str);
    let ports = PortConfiguration::from_manifest(manifest)?;
    Ok(issuer.is_none_or(|issuer| {
        ["localhost", "127.0.0.1"].into_iter().any(|host| {
            issuer.trim_end_matches('/')
                == format!(
                    "http://{host}:{}/realms/{}",
                    ports.keycloak, manifest.app.name
                )
        })
    }))
}

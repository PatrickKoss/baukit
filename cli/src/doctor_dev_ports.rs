use std::{fs, path::Path};

use anyhow::Result;

use crate::{Manifest, ports::PortConfiguration};

pub(crate) fn validate(
    root: &Path,
    manifest: &Manifest,
    ports: &PortConfiguration,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    if let Some(port) = ports.web {
        let before = failures.len();
        check_vite(root, port, failures)?;
        check_e2e_default(
            root,
            "web/e2e/playwright.stack.config.ts",
            "web",
            port,
            failures,
        )?;
        if crate::uses_bundled_keycloak(root, manifest)? {
            check_keycloak_origins(root, manifest, port, failures)?;
        }
        check_authorized_parties(root, manifest, port, failures)?;
        check_redirect_default(root, port, failures)?;
        record_success("web", port, before, successes, failures);
    }
    if let Some(port) = ports.web_preview {
        let before = failures.len();
        check_e2e_default(
            root,
            "web/e2e/playwright.config.ts",
            "web_preview",
            port,
            failures,
        )?;
        record_success("web_preview", port, before, successes, failures);
    }
    if let Some(port) = ports.metro {
        let before = failures.len();
        check_expo_scripts(root, port, failures)?;
        for directory in ["scripts", "mobile/scripts"] {
            check_metro_directory(root, &root.join(directory), port, failures)?;
        }
        check_metro_file(root, &root.join("Makefile"), port, failures)?;
        record_success("metro", port, before, successes, failures);
    }
    Ok(())
}

fn record_success(
    name: &str,
    port: u16,
    before: usize,
    successes: &mut Vec<String>,
    failures: &[String],
) {
    if failures.len() == before {
        successes.push(format!(
            "generated files use declared ports.{name} host {port}"
        ));
    }
}

fn read_optional(root: &Path, relative: &str) -> Result<Option<String>> {
    let path = root.join(relative);
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(path)?))
}

fn check_vite(root: &Path, port: u16, failures: &mut Vec<String>) -> Result<()> {
    let relative = "web/vite.config.ts";
    let Some(source) = read_optional(root, relative)? else {
        return Ok(());
    };
    let found = delimited_field(&source, "server", '{', '}')
        .and_then(|server| value_after_field(server, "port", ':'))
        .and_then(literal_port);
    check_port(relative, "web", port, found, failures);
    Ok(())
}

fn check_e2e_default(
    root: &Path,
    relative: &str,
    name: &str,
    port: u16,
    failures: &mut Vec<String>,
) -> Result<()> {
    let Some(source) = read_optional(root, relative)? else {
        return Ok(());
    };
    let found = source
        .split_once("E2E_WEB_PORT")
        .and_then(|(_, tail)| tail.split_once("??"))
        .and_then(|(_, tail)| literal_port(tail));
    check_port(relative, name, port, found, failures);
    Ok(())
}

fn check_port(
    relative: &str,
    name: &str,
    expected: u16,
    found: Option<u16>,
    failures: &mut Vec<String>,
) {
    if found == Some(expected) {
        return;
    }
    let found = found.map_or_else(
        || "no literal port".to_owned(),
        |port| format!("port {port}"),
    );
    failures.push(format!("generated file `{relative}` does not match declared ports.{name} (expected port {expected}, found {found})"));
}

fn check_keycloak_origins(
    root: &Path,
    manifest: &Manifest,
    port: u16,
    failures: &mut Vec<String>,
) -> Result<()> {
    let mcp_client = format!("{}-mcp", manifest.app.name);
    for (relative, fields) in [
        ("keycloak/realm.json", ["webOrigins", "redirectUris"]),
        (
            "keycloak/reconcile.json",
            ["activeOrigins", "activeRedirectUris"],
        ),
    ] {
        let Some(source) = read_optional(root, relative)? else {
            continue;
        };
        let json: serde_json::Value = serde_json::from_str(&source)?;
        let Some(clients) = json["clients"].as_array() else {
            continue;
        };
        for client in clients {
            if manifest.capabilities.mcp && client["clientId"].as_str() == Some(&mcp_client) {
                continue;
            }
            for field in fields {
                if let Some(origins) = client[field].as_array() {
                    for origin in origins.iter().filter_map(serde_json::Value::as_str) {
                        check_origins(relative, origin, port, failures);
                    }
                }
            }
        }
    }
    Ok(())
}

fn check_authorized_parties(
    root: &Path,
    manifest: &Manifest,
    port: u16,
    failures: &mut Vec<String>,
) -> Result<()> {
    for krate in crate::doctor_layout::rust_crates(root, manifest)? {
        for path in krate.sources {
            let source = fs::read_to_string(&path)?;
            let relative = path.strip_prefix(root)?.to_string_lossy();
            for (index, marker) in source.match_indices("authorized_parties:") {
                let section = &source[index + marker.len()..];
                if section
                    .split('[')
                    .next()
                    .is_some_and(|prefix| prefix.contains(','))
                {
                    continue;
                }
                let field = delimited_field(section, "", '[', ']');
                if let Some(field) = field {
                    check_origins(&relative, field, port, failures);
                }
            }
        }
    }
    Ok(())
}

fn check_redirect_default(root: &Path, port: u16, failures: &mut Vec<String>) -> Result<()> {
    let relative = "scripts/pkce-login.py";
    let Some(source) = read_optional(root, relative)? else {
        return Ok(());
    };
    if let Some((_, tail)) = source.split_once("--redirect-uri") {
        let argument = tail.split(')').next().unwrap_or_default();
        check_origins(relative, argument, port, failures);
    }
    Ok(())
}

fn check_origins(relative: &str, source: &str, port: u16, failures: &mut Vec<String>) {
    for found in crate::loopback_ports(source, false) {
        if found != port {
            check_port(relative, "web", port, Some(found), failures);
        }
    }
}

fn check_expo_scripts(root: &Path, port: u16, failures: &mut Vec<String>) -> Result<()> {
    let relative = "mobile/package.json";
    let Some(source) = read_optional(root, relative)? else {
        return Ok(());
    };
    let package: serde_json::Value = serde_json::from_str(&source)?;
    let Some(scripts) = package["scripts"].as_object() else {
        return Ok(());
    };
    for (name, command) in scripts
        .iter()
        .filter_map(|(name, value)| Some((name, value.as_str()?)))
    {
        for command in command.split(['&', ';', '|', '\n']) {
            let words = command.split_whitespace().collect::<Vec<_>>();
            for (index, pair) in words.windows(2).enumerate() {
                if pair[0] != "expo" || !(pair[1] == "start" || pair[1].starts_with("run:")) {
                    continue;
                }
                let flags = &words[index + 2..];
                let found = expo_port_flags(flags);
                let relative = format!("{relative} scripts.{name}");
                if found.is_empty() {
                    check_port(&relative, "metro", port, None, failures);
                }
                for found in found {
                    check_port(&relative, "metro", port, found, failures);
                }
            }
        }
    }
    Ok(())
}

fn expo_port_flags(flags: &[&str]) -> Vec<Option<u16>> {
    flags
        .iter()
        .enumerate()
        .filter_map(|(index, flag)| {
            if *flag == "--port" {
                Some(
                    flags
                        .get(index + 1)
                        .and_then(|value| value.trim_matches(['\'', '"']).parse().ok()),
                )
            } else {
                flag.strip_prefix("--port=")
                    .map(|value| value.trim_matches(['\'', '"']).parse().ok())
            }
        })
        .collect()
}

fn check_metro_directory(
    root: &Path,
    directory: &Path,
    port: u16,
    failures: &mut Vec<String>,
) -> Result<()> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let skipped_directory = matches!(
            entry.file_name().to_str(),
            Some("node_modules" | ".git" | "target" | "__pycache__")
        );
        if kind.is_dir() && !skipped_directory {
            check_metro_directory(root, &entry.path(), port, failures)?;
        } else if kind.is_file() {
            check_metro_file(root, &entry.path(), port, failures)?;
        }
    }
    Ok(())
}

fn check_metro_file(root: &Path, path: &Path, port: u16, failures: &mut Vec<String>) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let bytes = fs::read(path)?;
    let Ok(source) = std::str::from_utf8(&bytes) else {
        return Ok(());
    };
    let resolved_source = crate::resolve_makefile_variables(source);
    let relative = path.strip_prefix(root)?.to_string_lossy();
    for (original_line, line) in source.lines().zip(resolved_source.lines()) {
        for (_, tail) in line
            .split("RCT_METRO_PORT")
            .skip(1)
            .filter_map(|tail| tail.split_once('='))
        {
            if let Some(found) = literal_port(tail) {
                check_port(&relative, "metro", port, Some(found), failures);
            }
        }
        if line.contains("adb") && original_line.to_ascii_lowercase().contains("metro") {
            for found in line
                .split("reverse")
                .skip(1)
                .flat_map(|tail| tail.split("tcp:").skip(1).take(2))
                .filter_map(literal_port)
            {
                check_port(&relative, "metro", port, Some(found), failures);
            }
        }
    }
    Ok(())
}

fn literal_port(source: &str) -> Option<u16> {
    let source = source.trim_start().trim_start_matches(['\'', '"']);
    let length = source.bytes().take_while(u8::is_ascii_digit).count();
    source[..length].parse().ok()
}

fn value_after_field<'a>(source: &'a str, field: &str, separator: char) -> Option<&'a str> {
    source.match_indices(field).find_map(|(index, _)| {
        if index > 0 && source[..index].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            return None;
        }
        source[index + field.len()..]
            .trim_start()
            .strip_prefix(separator)
    })
}

fn delimited_field<'a>(source: &'a str, field: &str, open: char, close: char) -> Option<&'a str> {
    let tail = if field.is_empty() {
        source
    } else {
        value_after_field(source, field, ':')?
    };
    let start = tail.find(open)? + open.len_utf8();
    let mut depth = 1;
    for (index, character) in tail[start..].char_indices() {
        if character == open {
            depth += 1;
        }
        if character == close {
            depth -= 1;
        }
        if depth == 0 {
            return Some(&tail[start..start + index]);
        }
    }
    None
}

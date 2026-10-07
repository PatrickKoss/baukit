use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{Manifest, doctor_layout};

fn source(
    root: &Path,
    manifest: &Manifest,
    key: &str,
    paths: &[PathBuf],
    production: bool,
) -> Result<String> {
    let paths = match manifest.doctor.sources.get(key) {
        Some(relative) => vec![doctor_layout::product_path(root, relative)?],
        None => paths.to_vec(),
    };
    let mut code = String::new();
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(path)?;
        let text = if production {
            text.split("#[cfg(test)]").next().unwrap_or(&text)
        } else {
            &text
        };
        let symbols = doctor_layout::symbols(text, true);
        if key == "mcp_router"
            && !symbols
                .split_whitespace()
                .collect::<String>()
                .contains("baukit_mcp::router(")
        {
            continue;
        }
        let compact = symbols.split_whitespace().collect::<String>();
        if key == "mcp_drift"
            && !["tool_schema(", "service_schema(", "capability_schema("]
                .iter()
                .any(|name| compact.contains(name))
        {
            continue;
        }
        code.push_str(&symbols);
        code.push('\n');
    }
    Ok(code.split_whitespace().collect())
}

fn require(found: bool, label: &str, failures: &mut Vec<String>) {
    if !found {
        failures.push(format!(
            "missing remote MCP {label} in declared Cargo packages"
        ));
    }
}

pub(super) fn validate_wiring(
    root: &Path,
    manifest: &Manifest,
    failures: &mut Vec<String>,
) -> Result<()> {
    let crates = doctor_layout::rust_crates(root, manifest)?;
    let sources = crates
        .iter()
        .flat_map(|krate| krate.sources.iter().cloned())
        .collect::<Vec<_>>();
    let mut tools = Vec::new();
    let mut tests = crates
        .iter()
        .flat_map(|krate| krate.tests.iter().cloned())
        .collect::<Vec<_>>();
    tests.extend(doctor_layout::files(
        &doctor_layout::backend_manifest(root, manifest)?
            .parent()
            .expect("backend manifest parent")
            .join("tests"),
        "rs",
    )?);
    let mut dependency = false;
    for krate in &crates {
        let code = source(root, manifest, "", &krate.sources, true)?;
        if !krate.name.ends_with("-mcp")
            && !["implToolServicefor", "implbaukit_mcp::ToolServicefor"]
                .iter()
                .any(|symbol| code.contains(symbol))
        {
            continue;
        }
        tools.extend(krate.sources.iter().cloned());
        let cargo: toml::Value = toml::from_str(&fs::read_to_string(&krate.manifest)?)?;
        dependency |= cargo
            .get("dependencies")
            .and_then(toml::Value::as_table)
            .is_some_and(|dependencies| {
                dependencies.contains_key("baukit-mcp")
                    || dependencies.values().any(|value| {
                        value.get("package").and_then(toml::Value::as_str) == Some("baukit-mcp")
                    })
            });
    }
    require(dependency, "auth layer", failures);
    let router = source(root, manifest, "mcp_router", &sources, true)?;
    require(
        router.contains("baukit_mcp::router("),
        "router mount and auth layer",
        failures,
    );
    require(
        router.contains(".merge("),
        "router mount and auth layer (router merge)",
        failures,
    );
    let tools = source(root, manifest, "mcp_tools", &tools, true)?;
    require(
        ["implToolServicefor", "implbaukit_mcp::ToolServicefor"]
            .iter()
            .any(|symbol| tools.contains(symbol))
            && tools.split("fntools(").skip(1).any(|method| {
                method
                    .split_once('{')
                    .is_some_and(|(_, body)| !body.starts_with('}'))
            }),
        "tool registration and scope enforcement",
        failures,
    );
    require(
        tools.contains("required_scopes:"),
        "scope enforcement",
        failures,
    );
    let config = source(root, manifest, "mcp_config", &sources, true)?;
    let fields = [":baukit_mcp::McpConfig", ":McpConfig"]
        .into_iter()
        .flat_map(|kind| config.split(kind).take(config.matches(kind).count()))
        .filter_map(|prefix| {
            prefix
                .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
        })
        .map(|field| field.strip_prefix("pub").unwrap_or(field))
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    require(!fields.is_empty(), "resource configuration", failures);
    require(
        fields
            .iter()
            .any(|field| config.contains(&format!(".{field}.validate("))),
        "configuration validation",
        failures,
    );
    let drift = source(root, manifest, "mcp_drift", &tests, false)?;
    require(
        ["tool_schema(", "service_schema(", "capability_schema("]
            .iter()
            .any(|symbol| drift.contains(symbol))
            && drift.contains("include_str!")
            && drift.contains("assert_eq!"),
        "schema drift check",
        failures,
    );
    Ok(())
}

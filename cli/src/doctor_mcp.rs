use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{Manifest, doctor_layout, identity};

fn source(
    root: &Path,
    manifest: &Manifest,
    key: &str,
    paths: &[PathBuf],
    production: bool,
) -> Result<String> {
    let paths = match manifest.doctor.sources.get(key) {
        Some(relative) => identity::library_sources(&doctor_layout::product_path(root, relative)?)?
            .into_iter()
            .collect(),
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

fn config_bindings(code: &str, borrowed: bool) -> Vec<&str> {
    let kinds = if borrowed {
        [":&baukit_mcp::McpConfig", ":&McpConfig"]
    } else {
        [":baukit_mcp::McpConfig", ":McpConfig"]
    };
    kinds
        .into_iter()
        .flat_map(|kind| code.split(kind).take(code.matches(kind).count()))
        .filter_map(|prefix| {
            prefix
                .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
        })
        .map(|field| field.strip_prefix("pub").unwrap_or(field))
        .filter(|field| !field.is_empty())
        .collect()
}

fn validates_config(code: &str, bindings: &[&str]) -> bool {
    bindings.iter().any(|binding| {
        code.match_indices(&format!("{binding}.validate("))
            .any(|(index, _)| {
                index == 0
                    || !code[..index].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            })
    })
}

fn block(code: &str, open: char, close: char) -> Option<&str> {
    let mut depth = 1;
    code.char_indices().find_map(|(index, character)| {
        if character == open {
            depth += 1;
        } else if character == close {
            depth -= 1;
        }
        (depth == 0).then_some(&code[..index])
    })
}

fn functions(code: &str) -> impl Iterator<Item = (&str, &str, &str)> {
    code.match_indices("fn").filter_map(|(index, _)| {
        let function = &code[index + "fn".len()..];
        let (signature, body) = function.split_once('{')?;
        let (name, parameters) = signature.split_once('(')?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        Some((name, block(parameters, '(', ')')?, block(body, '{', '}')?))
    })
}

fn router_merged(router: &str, graph: &str) -> bool {
    if router.contains(".merge(baukit_mcp::router(") {
        return true;
    }
    let bindings = router
        .split("=baukit_mcp::router(")
        .take(router.matches("=baukit_mcp::router(").count())
        .filter_map(|prefix| {
            prefix
                .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
        })
        .map(|binding| binding.strip_prefix("let").unwrap_or(binding))
        .collect::<Vec<_>>();
    if bindings
        .iter()
        .any(|binding| router.contains(&format!(".merge({binding})")))
    {
        return true;
    }
    functions(graph).any(|(name, parameters, body)| {
        parameters.split(',').enumerate().any(|(index, parameter)| {
            let Some((parameter, _)) = parameter.split_once(':') else {
                return false;
            };
            if !body.contains(&format!(".merge({parameter})")) {
                return false;
            }
            router.split(&format!("{name}(")).skip(1).any(|call| {
                block(call, '(', ')')
                    .and_then(|arguments| arguments.split(',').nth(index))
                    .is_some_and(|argument| bindings.contains(&argument))
            })
        })
    })
}

fn wrapper_validates_config(code: &str) -> bool {
    functions(code).any(|(name, parameters, body)| {
        code.matches(&format!("{name}(")).count() >= 2
            && validates_config(body, &config_bindings(parameters, true))
    })
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
    let graph = source(root, manifest, "", &sources, true)?;
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
        router_merged(&router, &graph),
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
    let fields = config_bindings(&config, false);
    require(!fields.is_empty(), "resource configuration", failures);
    require(
        validates_config(&config, &fields) || wrapper_validates_config(&graph),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_follows_the_mcp_argument_past_other_router_merges_and_middleware() {
        let router = "letmcp=baukit_mcp::router(config);router_with_routes(state,mcp)";
        let graph = "pubfnrouter_with_routes(state:State,additional:Router)->Router{Router::new().merge(base).merge(additional).layer(middleware::from_fn(auth))}";
        assert!(router_merged(router, graph));
        assert!(!router_merged(
            router,
            &graph.replace(".merge(additional)", "")
        ));
    }

    #[test]
    fn validation_requires_a_called_wrapper_and_its_typed_parameter() {
        let wrapper = "pubfnvalidate_mcp(provider:Provider,config:&baukit_mcp::McpConfig)->Result<(),Error>{check_provider(provider)?;config.validate()}";
        assert!(!wrapper_validates_config(wrapper));
        let called = format!("{wrapper}validate_mcp(provider,&self.mcp)");
        assert!(wrapper_validates_config(&called));
        assert!(!wrapper_validates_config(
            &called.replace("config.validate()", "other_config.validate()")
        ));
    }
}

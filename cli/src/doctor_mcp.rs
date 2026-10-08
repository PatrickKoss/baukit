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
        let symbols = if production {
            production_symbols(&text)
        } else {
            doctor_layout::symbols(&text, true)
        };
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

fn production_symbols(text: &str) -> String {
    let symbols = doctor_layout::symbols(text, true);
    let compact = symbols.split_whitespace().collect::<String>();
    let mut remaining = compact.as_str();
    let mut code = String::new();
    while let Some((before, item)) = remaining.split_once("#[cfg(test)]") {
        code.push_str(before);
        let Some(index) = item.find(['{', ';']) else {
            return code;
        };
        let suffix = &item[index + 1..];
        if item.as_bytes()[index] == b';' {
            remaining = suffix;
        } else if let Some(body) = block(suffix, '{', '}') {
            remaining = &suffix[body.len() + 1..];
        } else {
            return code;
        }
    }
    code.push_str(remaining);
    code
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
    functions(router).any(|(_, _, caller)| {
        if caller.contains(".merge(baukit_mcp::router(") {
            return true;
        }
        let bindings = router_bindings(caller).collect::<Vec<_>>();
        if bindings.is_empty() {
            return false;
        }
        if bindings
            .iter()
            .any(|binding| caller.contains(&format!(".merge({binding})")))
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
                caller.split(&format!("{name}(")).skip(1).any(|call| {
                    block(call, '(', ')')
                        .and_then(|arguments| arguments.split(',').nth(index))
                        .is_some_and(|argument| bindings.contains(&argument))
                })
            })
        })
    })
}

fn top_level_parts(code: &str, delimiter: char) -> impl Iterator<Item = &str> {
    let mut depth = 0_usize;
    code.split(move |character| {
        let split = depth == 0 && character == delimiter;
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        split
    })
}

fn block_returns_router(body: &str) -> bool {
    top_level_parts(body, ';')
        .last()
        .is_some_and(router_expression)
}

fn router_expression(expression: &str) -> bool {
    if let Some(call) = expression.strip_prefix("baukit_mcp::router(") {
        return after_call(call) == Some("");
    }
    if let Some(body) = expression.strip_prefix('{') {
        return block(body, '{', '}')
            .is_some_and(|inner| inner.len() + 1 == body.len() && block_returns_router(inner));
    }
    if !expression.starts_with("if") && !expression.starts_with("match") {
        return false;
    }
    let header = top_level_parts(expression, '{')
        .next()
        .unwrap_or(expression);
    let Some(body) = expression.get(header.len() + 1..) else {
        return false;
    };
    let Some(inner) = block(body, '{', '}') else {
        return false;
    };
    let suffix = &body[inner.len() + 1..];
    if expression.starts_with("match") {
        return suffix.is_empty()
            && top_level_parts(inner, ',').any(|arm| {
                arm.split_once("=>")
                    .is_some_and(|(_, result)| router_expression(result))
            });
    }
    suffix
        .strip_prefix("else")
        .is_some_and(|alternative| block_returns_router(inner) || router_expression(alternative))
}

fn router_bindings(body: &str) -> impl Iterator<Item = &str> {
    top_level_parts(body, ';').filter_map(|statement| {
        let prefix = top_level_parts(statement, '=').next()?;
        let declaration = prefix
            .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next()?;
        let binding = declaration
            .strip_prefix("letmut")
            .or_else(|| declaration.strip_prefix("let"))?;
        let expression = statement.get(prefix.len() + 1..)?;
        (!binding.is_empty() && router_expression(expression)).then_some(binding)
    })
}

fn after_call(call: &str) -> Option<&str> {
    let arguments = block(call, '(', ')')?;
    let suffix = &call[arguments.len() + 1..];
    let suffix = suffix.strip_prefix(".await").unwrap_or(suffix);
    Some(suffix.strip_prefix('?').unwrap_or(suffix))
}

fn returns_mcp_router(body: &str) -> bool {
    let terminated = body.ends_with(';');
    let body = body.strip_suffix(';').unwrap_or(body);
    let Some((prefix, call)) = body.rsplit_once("baukit_mcp::router(") else {
        return false;
    };
    let prefix = prefix.rsplit([';', '{', '}']).next().unwrap_or(prefix);
    (!terminated || prefix.starts_with("return"))
        && matches!(
            (prefix, after_call(call)),
            ("" | "return", Some("")) | ("Ok(" | "returnOk(", Some(")"))
        )
}

fn called_result_merged(code: &str, name: &str) -> bool {
    let call = format!("{name}(");
    functions(code).any(|(_, _, body)| {
        body.match_indices(&call).any(|(index, _)| {
            let prefix = &body[..index];
            if prefix.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == ':') {
                return false;
            }
            let Some(suffix) = after_call(&body[index + call.len()..]) else {
                return false;
            };
            if prefix.ends_with(".merge(") && suffix.starts_with(')') {
                return true;
            }
            let Some(binding) = prefix.strip_suffix('=') else {
                return false;
            };
            if !suffix.starts_with(';') {
                return false;
            }
            let binding = binding
                .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
                .unwrap_or(binding);
            let binding = binding
                .strip_prefix("letmut")
                .or_else(|| binding.strip_prefix("let"))
                .unwrap_or(binding);
            !binding.is_empty() && body.contains(&format!(".merge({binding})"))
        })
    })
}

fn qualified_function(krate: &doctor_layout::RustCrate, path: &Path, name: &str) -> Option<String> {
    let mut parts = vec![krate.name.replace('-', "_")];
    if path != krate.library {
        let module = path
            .strip_prefix(krate.library.parent()?)
            .ok()?
            .with_extension("");
        parts.extend(
            module
                .components()
                .map(|component| component.as_os_str().to_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        );
        if parts.last().is_some_and(|part| part == "mod") {
            parts.pop();
        }
    }
    parts.push(name.to_owned());
    Some(parts.join("::"))
}

fn returned_router_merged(
    crates: &[doctor_layout::RustCrate],
    router: &str,
    graph: &str,
) -> Result<bool> {
    for krate in crates {
        for path in &krate.sources {
            let text = fs::read_to_string(path)?;
            let code = production_symbols(&text);
            for (name, _, body) in functions(&code) {
                if !router.contains(body) || !returns_mcp_router(body) {
                    continue;
                }
                if called_result_merged(&code, name)
                    || qualified_function(krate, path, name)
                        .is_some_and(|qualified| called_result_merged(graph, &qualified))
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
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
        router_merged(&router, &graph) || returned_router_merged(&crates, &router, &graph)?,
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
    fn production_scan_keeps_code_after_test_modules() {
        let source = r#"
#[cfg ( test )]
mod tests {
    mod nested {
        fn fake() { baukit_mcp::router(test_config); }
    }
    const BRACE: &str = "}";
}
fn run() { api.merge(baukit_mcp::router(config)); }
#[cfg(test)]
mod more_tests { fn fake() { test_config.validate(); } }
fn validate(config: &McpConfig) { config.validate(); }
"#;
        let code = production_symbols(source);
        assert_eq!(
            code,
            "fnrun(){api.merge(baukit_mcp::router(config));}fnvalidate(config:&McpConfig){config.validate();}"
        );
        assert!(router_merged(&code, &code));
        assert!(validates_config(&code, &["config"]));
        let tests_only = production_symbols(
            "#[cfg(test)] mod tests { fn run() { api.merge(baukit_mcp::router(config)); } }",
        );
        assert!(!router_merged(&tests_only, &tests_only));
    }

    #[test]
    fn conditional_router_follows_branch_results_and_the_merged_binding() {
        for expression in [
            "ifconfig.enabled{baukit_mcp::router(config).await?}else{Router::new()}",
            "ifcheck(Config{enabled:true}){Router::new()}elseifready{baukit_mcp::router(config).await?}else{Router::new()}",
            "matchconfig.enabled{true=>baukit_mcp::router(config).await?,false=>Router::new()}",
            "matchconfig.enabled{true=>{letchecked=check(config);baukit_mcp::router(checked).await?},false=>Router::new()}",
        ] {
            let code = format!("asyncfnrun(){{letmcp={expression};api.merge(mcp)}}");
            assert!(router_merged(&code, &code), "{expression}");
            let unmerged = code.replace(".merge(mcp)", ".merge(other)");
            assert!(!router_merged(&unmerged, &unmerged));
            let unmerged =
                format!("asyncfnrun(){{letmcp={expression};}}fnother(){{api.merge(mcp)}}");
            assert!(!router_merged(&unmerged, &unmerged));
        }
    }

    #[test]
    fn conditional_router_rejects_discarded_branch_values() {
        for expression in [
            "ifready{baukit_mcp::router(config).await?;Router::new()}else{Router::new()}",
            "matchmode{Mcp=>{letunused=baukit_mcp::router(config).await?;Router::new()},_=>Router::new()}",
            "ifready{consume(baukit_mcp::router(config).await?)}else{Router::new()}",
            "ifready{letmcp=baukit_mcp::router(config).await?;Router::new()}else{Router::new()}",
        ] {
            let code = format!("asyncfnrun(){{letmcp={expression};api.merge(mcp)}}");
            assert!(!router_merged(&code, &code), "{expression}");
        }
    }

    #[test]
    fn returned_router_must_be_the_function_result() {
        assert!(returns_mcp_router(
            "if!config.enabled{returnOk(Router::new());}Ok(baukit_mcp::router(config).await?)"
        ));
        assert!(returns_mcp_router("returnbaukit_mcp::router(config);"));
        assert!(!returns_mcp_router(
            "letunused=baukit_mcp::router(config).await?;Ok(Router::new())"
        ));
        assert!(!returns_mcp_router(
            "letunused=Ok(baukit_mcp::router(config).await?);"
        ));
        assert!(!returns_mcp_router("baukit_mcp::router(config);"));
        assert!(!returns_mcp_router(
            "Ok(baukit_mcp::router(config).await?);"
        ));
    }

    #[test]
    fn wrapper_result_must_be_merged_in_its_caller() {
        let name = "product_bin::compose::mcp::router";
        let code = format!("asyncfnrun(){{letmcp={name}(config).await?;api.merge(mcp)}}");
        assert!(called_result_merged(&code, name));
        assert!(called_result_merged(
            &format!("asyncfnrun(){{api.merge({name}(config).await?)}}"),
            name
        ));
        for missing in [
            code.replace(".merge(mcp)", ""),
            code.replace(".merge(mcp)", ".merge(other)"),
            code.replace(name, "product_api::router"),
            code.replace(name, &format!("other_{name}")),
            format!("asyncfnrun(){{letmcp={name}(config).await?;}}fnother(){{api.merge(mcp)}}"),
        ] {
            assert!(!called_result_merged(&missing, name), "{missing}");
        }
    }

    #[test]
    fn composition_follows_the_mcp_argument_past_other_router_merges_and_middleware() {
        let router = "fnrun(){letmcp=baukit_mcp::router(config);router_with_routes(state,mcp)}";
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

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{AuthProvider, Manifest, quoted_string, source_constant};

fn sources(directory: &Path, extension: &str, output: &mut Vec<PathBuf>) -> Result<()> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            sources(&path, extension, output)?;
        } else if path.extension().is_some_and(|value| value == extension)
            && !path.file_name().is_some_and(|value| {
                value.to_string_lossy().contains(".test.")
                    || value.to_string_lossy().contains(".spec.")
            })
        {
            output.push(path);
        }
    }
    output.sort();
    Ok(())
}

pub(super) fn uncommented(source: &str, rust: bool) -> String {
    let mut result = String::new();
    let mut chars = source.chars().peekable();
    while let Some(character) = chars.next() {
        let lifetime = rust
            && character == '\''
            && chars
                .peek()
                .is_some_and(|value| value.is_ascii_alphabetic() || *value == '_')
            && chars.clone().nth(1) != Some('\'');
        if matches!(character, '\'' | '"' | '`') && !lifetime {
            result.push(character);
            let mut escaped = false;
            for next in chars.by_ref() {
                result.push(next);
                if escaped {
                    escaped = false;
                } else if next == '\\' {
                    escaped = true;
                } else if next == character {
                    break;
                }
            }
        } else if character == '/' && chars.next_if_eq(&'/').is_some() {
            chars.by_ref().find(|value| *value == '\n');
            result.push('\n');
        } else if character == '/' && chars.next_if_eq(&'*').is_some() {
            crate::skip_typescript_block_comment(&mut chars);
            result.push(' ');
        } else {
            result.push(character);
        }
    }
    result
}

pub(super) fn identity_code(source: &str, rust: bool) -> String {
    let mut code = source.as_bytes().to_vec();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let quote = bytes[index];
        let lifetime = rust
            && quote == b'\''
            && bytes
                .get(index + 1)
                .is_some_and(|value| value.is_ascii_alphabetic() || *value == b'_')
            && bytes.get(index + 2) != Some(&b'\'');
        if !matches!(quote, b'\'' | b'"' | b'`') || lifetime {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index += 2;
            } else if bytes[index] == quote {
                index += 1;
                break;
            } else {
                index += 1;
            }
        }
        let end = index.min(bytes.len());
        for character in &mut code[start..end] {
            if *character != b'\n' {
                *character = b' ';
            }
        }
    }
    String::from_utf8(code).expect("masking complete strings preserves UTF-8")
}

fn identity_constant<'a>(source: &'a str, name: &str, rust: bool) -> Option<&'a str> {
    let code = identity_code(source, rust);
    code.match_indices("const ").find_map(|(index, _)| {
        let statement = &source[index..];
        let declaration = &code[index + "const ".len()..];
        let binding = declaration.split_once('=')?.0.split(':').next()?.trim();
        (binding == name)
            .then(|| source_constant(statement, name))
            .flatten()
    })
}

fn consumer<'a>(source: &'a str, code: &str, name: &str) -> Option<&'a str> {
    let index = code.find(name)? + name.len();
    Some(&source[index..])
}

fn field<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    source
        .split_once(&format!("{name}:"))?
        .1
        .trim()
        .split([',', '\n'])
        .next()
        .map(str::trim)
}

fn module_path(directory: &Path, module: &str) -> Option<PathBuf> {
    let path = directory.join(module);
    let stem = if path.extension().is_some_and(|value| value == "js") {
        path.with_extension("ts")
    } else {
        path
    };
    [
        stem.clone(),
        stem.with_extension("ts"),
        stem.with_extension("tsx"),
        stem.join("index.ts"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn imported_binding(source: &str, name: &str) -> Option<(String, String)> {
    for (index, _) in identity_code(source, false).match_indices("import ") {
        let statement = &source[index + "import ".len()..];
        let Some((bindings, tail)) = statement.split_once(" from ") else {
            continue;
        };
        let Some(bindings) = bindings
            .strip_prefix('{')
            .and_then(|value| value.split_once('}').map(|parts| parts.0))
        else {
            continue;
        };
        let module = tail.trim().split(';').next()?.trim();
        let Some(module) = quoted_string(module) else {
            continue;
        };
        for binding in bindings.split(',').map(str::trim) {
            let (exported, local) = binding.split_once(" as ").unwrap_or((binding, binding));
            if local.trim() == name {
                return Some((module.to_owned(), exported.trim().to_owned()));
            }
        }
    }
    None
}

fn resolve_binding(path: &Path, source: &str, binding: &str) -> Result<Option<String>> {
    if let Some(value) = identity_constant(
        source,
        binding,
        path.extension().is_some_and(|extension| extension == "rs"),
    )
    .and_then(quoted_string)
    {
        return Ok(Some(value.to_owned()));
    }
    let Some((module, exported)) = imported_binding(source, binding) else {
        return Ok(None);
    };
    if !module.starts_with('.') {
        return Ok(None);
    }
    let Some(module) = module_path(path.parent().unwrap_or(Path::new("")), &module) else {
        return Ok(None);
    };
    let source = uncommented(&fs::read_to_string(module)?, false);
    Ok(identity_constant(&source, &exported, false)
        .and_then(quoted_string)
        .map(str::to_owned))
}

fn resolve_identity(path: &Path, source: &str, expression: &str) -> Result<Option<String>> {
    if let Some(value) = quoted_string(expression) {
        return Ok(Some(value.to_owned()));
    }
    if let Some(template) = expression
        .strip_prefix("`${")
        .and_then(|value| value.strip_suffix('`'))
    {
        let Some((binding, suffix)) = template.split_once('}') else {
            return Ok(None);
        };
        return Ok(resolve_binding(path, source, binding)?.map(|value| format!("{value}{suffix}")));
    }
    resolve_binding(path, source, expression)
}

fn check_identity(
    root: &Path,
    path: &Path,
    source: &str,
    expression: &str,
    expected: Option<&str>,
    aliases: &BTreeSet<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let relative = path.strip_prefix(root)?.display();
    match resolve_identity(path, source, expression)? {
        None => failures.push(format!("product identity `{expression}` consumed by `{relative}` has no literal source")),
        Some(value) if expected.is_some_and(|expected| value != expected && !aliases.contains(&value)) => failures.push(format!("product identity `{expression}` consumed by `{relative}` does not match application name `{}`", expected.unwrap_or_default())),
        Some(value) if value.is_empty() => failures.push(format!("product identity consumed by `{relative}` is empty")),
        Some(_) => {}
    }
    Ok(())
}

fn typescript_identities(
    root: &Path,
    manifest: &Manifest,
    aliases: &BTreeSet<String>,
    failures: &mut Vec<String>,
) -> Result<usize> {
    let mut count = 0;
    let no_aliases = BTreeSet::new();
    for (directory, enabled) in [
        ("mobile", manifest.capabilities.mobile),
        ("web", manifest.capabilities.web),
        ("mcp", manifest.capabilities.mcp.is_some()),
    ] {
        if !enabled {
            continue;
        }
        let mut paths = Vec::new();
        sources(&root.join(directory).join("src"), "ts", &mut paths)?;
        sources(&root.join(directory).join("src"), "tsx", &mut paths)?;
        if directory == "mobile" {
            paths.push(root.join("mobile/app.config.ts"));
        }
        for path in paths {
            if !path.is_file() {
                continue;
            }
            let source = uncommented(&fs::read_to_string(&path)?, false);
            let code = identity_code(&source, false);
            let consumer = if let Some(context) = consumer(&source, &code, "new AnalyticsClient") {
                field(context, "app").map(|value| (value, Some(manifest.app.name.clone())))
            } else if let Some(server) = consumer(&source, &code, "new McpServer(") {
                field(server, "name")
                    .map(|value| (value, Some(format!("{}-mcp", manifest.app.name))))
            } else if path.ends_with("app.config.ts") {
                field(&source, "slug").map(|value| (value, None))
            } else {
                None
            };
            if let Some((expression, expected)) = consumer {
                count += 1;
                check_identity(
                    root,
                    &path,
                    &source,
                    expression,
                    expected.as_deref(),
                    if code.contains("new AnalyticsClient") {
                        aliases
                    } else {
                        &no_aliases
                    },
                    failures,
                )?;
            }
            if directory == "mcp" {
                for (index, _) in code.match_indices("process.env[") {
                    let Some(statement) =
                        source[index + "process.env[".len()..].strip_prefix("`${")
                    else {
                        continue;
                    };
                    let Some((binding, _)) = statement.split_once('}') else {
                        continue;
                    };
                    check_identity(
                        root,
                        &path,
                        &source,
                        binding,
                        Some(&manifest.app.name.replace('-', "_").to_ascii_uppercase()),
                        &no_aliases,
                        failures,
                    )?;
                }
            }
        }
    }
    Ok(count)
}

fn imports_rust_binding(source: &str, crate_name: &str, binding: &str) -> bool {
    source.split("use ").skip(1).any(|statement| {
        let Some(import) = statement
            .split(';')
            .next()
            .and_then(|value| value.strip_prefix(&format!("{crate_name}::")))
        else {
            return false;
        };
        import.trim() == binding
            || import
                .trim()
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
                .is_some_and(|names| names.split(',').any(|name| name.trim() == binding))
    })
}

fn rust_identity(
    root: &Path,
    manifest: &Manifest,
    failures: &mut Vec<String>,
) -> Result<(usize, BTreeSet<String>)> {
    if !manifest.capabilities.backend {
        return Ok((0, BTreeSet::new()));
    }
    let crates = crate::doctor_layout::rust_crates(root, manifest)?;
    let mut names = BTreeSet::new();
    let mut count = 0;
    for krate in crates {
        for path in krate.sources {
            let source = fs::read_to_string(&path)?;
            let source = uncommented(source.split("#[cfg(test)]").next().unwrap_or(&source), true);
            if manifest.capabilities.auth == Some(AuthProvider::Oidc) {
                for declaration in source.split("identity_admin_realm:").skip(1) {
                    if let Some(expression) = declaration
                        .trim()
                        .split([',', '\n'])
                        .next()
                        .and_then(|value| value.strip_suffix(".to_owned()"))
                    {
                        check_identity(
                            root,
                            &path,
                            &source,
                            expression,
                            Some(&manifest.app.name),
                            &BTreeSet::new(),
                            failures,
                        )?;
                    }
                }
            }
            for (index, _) in identity_code(&source, true).match_indices("ConfigLoader::new(") {
                let call = &source[index + "ConfigLoader::new(".len()..];
                let Some(expression) = call.split(',').next().map(str::trim) else {
                    continue;
                };
                count += 1;
                let value = quoted_string(expression)
                    .or_else(|| {
                        identity_constant(&source, expression, true).and_then(quoted_string)
                    })
                    .map(str::to_owned);
                let value = match value {
                    Some(value) => Some(value),
                    None => {
                        let library = &krate.library;
                        let library_source = if library.is_file() {
                            uncommented(&fs::read_to_string(library)?, true)
                        } else {
                            String::new()
                        };
                        let crate_name = krate.name.replace('-', "_");
                        if imports_rust_binding(&source, &crate_name, expression) {
                            identity_constant(&library_source, expression, true)
                                .and_then(quoted_string)
                                .map(str::to_owned)
                        } else {
                            None
                        }
                    }
                };
                if let Some(value) = value {
                    if manifest.capabilities.auth == Some(AuthProvider::Oidc)
                        && value != manifest.app.name
                    {
                        failures.push(format!("product identity `{expression}` consumed by `{}` does not match application name `{}`", path.strip_prefix(root)?.display(), manifest.app.name));
                    }
                    names.insert(value);
                } else {
                    failures.push(format!(
                        "product identity `{expression}` consumed by `{}` has no literal source",
                        path.strip_prefix(root)?.display()
                    ));
                }
            }
        }
    }
    if names.len() > 1 {
        failures.push("backend config consumers use inconsistent product identities".to_owned());
    }
    Ok((count, names))
}

pub(super) fn validate(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let initial = failures.len();
    let (backend_count, aliases) = rust_identity(root, manifest, failures)?;
    let count = backend_count + typescript_identities(root, manifest, &aliases, failures)?;
    if count == 0 {
        failures.push("no consumed product identity source was found".to_owned());
    }
    if failures.len() == initial {
        successes.push("consumed product identities are consistent".to_owned());
    }
    Ok(())
}

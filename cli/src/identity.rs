use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

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

fn skip_block_comment(chars: &mut std::iter::Peekable<impl Iterator<Item = char>>) {
    while let Some(character) = chars.next() {
        if character == '*' && chars.next_if_eq(&'/').is_some() {
            return;
        }
    }
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
            skip_block_comment(&mut chars);
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

pub(super) fn module_path(directory: &Path, module: &str) -> Option<PathBuf> {
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
        stem.join("index.tsx"),
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
        }
    }
    Ok(count)
}

pub(super) fn validate_admin_realm(
    root: &Path,
    manifest: &Manifest,
    realm: &str,
    failures: &mut Vec<String>,
) -> Result<()> {
    for krate in crate::doctor_layout::backend_crates(root, manifest)? {
        for path in krate.sources {
            let source = fs::read_to_string(&path)?;
            let source = uncommented(source.split("#[cfg(test)]").next().unwrap_or(&source), true);
            for declaration in source.split("identity_admin_realm:").skip(1) {
                let Some(expression) = declaration
                    .trim()
                    .split([',', '\n'])
                    .next()
                    .and_then(|value| value.strip_suffix(".to_owned()"))
                else {
                    continue;
                };
                let value = quoted_string(expression)
                    .map(str::to_owned)
                    .or(resolve_binding(&path, &source, expression)?);
                if let Some(value) = value
                    && value != realm
                {
                    failures.push(format!("OIDC admin realm `{expression}` consumed by `{}` does not match selected Keycloak realm `{realm}`", path.strip_prefix(root)?.display()));
                }
            }
        }
    }
    Ok(())
}

fn imported_rust_binding(source: &str, crate_name: &str, expression: &str) -> Option<String> {
    let prefix = format!("{crate_name}::");
    if let Some(binding) = expression.strip_prefix(&prefix) {
        return Some(binding.to_owned());
    }
    for statement in source.split("use ").skip(1) {
        let Some(import) = statement
            .split(';')
            .next()
            .and_then(|value| value.strip_prefix(&prefix))
        else {
            continue;
        };
        let import = import.trim().trim_start_matches('{').trim_end_matches('}');
        for name in import.split(',') {
            let mut parts = name.trim().split(" as ");
            let binding = parts.next()?;
            let local = parts.next().unwrap_or(binding);
            if local == expression {
                return Some(binding.to_owned());
            }
        }
    }
    None
}

pub(super) fn library_sources(library: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut sources = BTreeSet::new();
    let mut pending = vec![library.to_owned()];
    while let Some(path) = pending.pop() {
        if !path.is_file() || !sources.insert(path.clone()) {
            continue;
        }
        let source = identity_code(&uncommented(&fs::read_to_string(&path)?, true), true);
        let parent = path.parent().context("Rust module has no parent")?;
        let directory = if matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("lib.rs" | "mod.rs")
        ) {
            parent.to_owned()
        } else {
            parent.join(path.file_stem().context("Rust module has no name")?)
        };
        for declaration in source.split(';') {
            let words = declaration.split_whitespace().collect::<Vec<_>>();
            if let [.., "mod", name] = words.as_slice()
                && name
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || value == '_')
            {
                for candidate in [
                    directory.join(format!("{name}.rs")),
                    directory.join(name).join("mod.rs"),
                ] {
                    if candidate.is_file() {
                        pending.push(candidate);
                    }
                }
            }
        }
    }
    Ok(sources)
}

fn rust_identity(
    root: &Path,
    manifest: &Manifest,
    failures: &mut Vec<String>,
) -> Result<(usize, BTreeSet<String>)> {
    if !manifest.capabilities.backend {
        return Ok((0, BTreeSet::new()));
    }
    let crates = crate::doctor_layout::backend_crates(root, manifest)?;
    let mut names = BTreeSet::new();
    let mut count = 0;
    for krate in crates {
        let modules = library_sources(&krate.library)?;
        for path in krate.sources {
            let source = fs::read_to_string(&path)?;
            let source = uncommented(source.split("#[cfg(test)]").next().unwrap_or(&source), true);
            let code = identity_code(&source, true);
            let consumers = ["ConfigLoader::new(", "ServiceInfo::new("];
            for (start, is_loader) in consumers.into_iter().flat_map(|consumer| {
                code.match_indices(consumer).map(move |(index, _)| {
                    (index + consumer.len(), consumer == "ConfigLoader::new(")
                })
            }) {
                let call = &source[start..];
                let Some(expression) = call.split(',').next().map(str::trim) else {
                    continue;
                };
                if !is_loader && !static_rust_identity(expression) {
                    continue;
                }
                count += 1;
                let local_binding = expression
                    .strip_prefix("crate::")
                    .filter(|_| {
                        path == krate.library
                            || path.file_name().is_some_and(|name| name == "main.rs")
                            || path.parent().is_some_and(|parent| {
                                parent.file_name().is_some_and(|name| name == "bin")
                            })
                    })
                    .unwrap_or(expression);
                let value = quoted_string(expression)
                    .or_else(|| {
                        identity_constant(&source, local_binding, true).and_then(quoted_string)
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
                        let binding = imported_rust_binding(&source, &crate_name, expression)
                            .or_else(|| {
                                modules
                                    .contains(&path)
                                    .then(|| imported_rust_binding(&source, "crate", expression))
                                    .flatten()
                            });
                        binding.and_then(|binding| {
                            identity_constant(&library_source, &binding, true)
                                .and_then(quoted_string)
                                .map(str::to_owned)
                        })
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

fn static_rust_identity(expression: &str) -> bool {
    quoted_string(expression).is_some()
        || expression.rsplit("::").next().is_some_and(|name| {
            name.chars().any(|c| c.is_ascii_uppercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
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

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use globset::Glob;
use serde::{Deserialize, Serialize};

use crate::{DoctorHost, Manifest, identity};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DoctorPaths {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_manifest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_dockerfile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_dockerignore: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrations: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keycloak_realm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keycloak_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keycloak_reconcile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keycloak_policy_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keycloak_reconcile_tool: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<String, String>,
}

impl DoctorPaths {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

pub(super) fn product_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path.components().any(|part| part == Component::ParentDir)
    {
        bail!("doctor path `{relative}` must be relative to the product root without `..`");
    }
    Ok(root.join(path))
}

fn cargo_path(root: &Path, directory: &Path, relative: &str) -> Result<PathBuf> {
    let path = directory.join(relative);
    if path.exists() && !path.canonicalize()?.starts_with(root.canonicalize()?) {
        bail!("Cargo target `{relative}` is outside the product root");
    }
    Ok(path)
}

pub(super) fn validate_paths(root: &Path, manifest: &Manifest) -> Result<()> {
    const SOURCE_KEYS: &[&str] = &[
        "backend_limits",
        "mcp_router",
        "mcp_tools",
        "mcp_config",
        "mcp_drift",
        "worker_entry",
        "worker_tests",
        "auth_tests",
        "pkce_login",
        "mobile_sign_in",
        "mobile_auth",
        "mobile_auth_tests",
        "mobile_local_data",
        "mobile_persistence",
        "keycloak_policy_tests",
        "keycloak_reconcile_tests",
    ];
    for (key, relative) in &manifest.doctor.sources {
        if !SOURCE_KEYS.contains(&key.as_str()) {
            bail!("unknown doctor.sources key `{key}`");
        }
        product_path(root, relative)?;
    }
    Ok(())
}

pub(super) fn backend_manifest(root: &Path, manifest: &Manifest) -> Result<PathBuf> {
    product_path(
        root,
        manifest
            .doctor
            .backend_manifest
            .as_deref()
            .unwrap_or("backend/Cargo.toml"),
    )
}

pub(super) fn cargo_manifests(root: &Path) -> Result<Vec<PathBuf>> {
    Ok(product_files(root, "toml")?
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .collect())
}

pub(super) fn cargo_workspaces(root: &Path, manifest: &Manifest) -> Result<Vec<PathBuf>> {
    let mut workspaces = Vec::new();
    for path in cargo_manifests(root)? {
        let cargo: toml::Value =
            toml::from_str(&fs::read_to_string(&path)?).with_context(|| {
                format!(
                    "could not parse `{}`",
                    path.strip_prefix(root).unwrap_or(&path).display()
                )
            })?;
        if cargo.get("workspace").is_some() {
            workspaces.push(path);
        }
    }
    if manifest.capabilities.backend {
        let backend = backend_manifest(root, manifest)?;
        if backend.is_file() {
            workspaces.push(backend);
        }
    }
    workspaces.sort();
    workspaces.dedup();
    Ok(workspaces)
}

pub(super) fn migrations(root: &Path, manifest: &Manifest) -> Result<PathBuf> {
    product_path(
        root,
        manifest
            .doctor
            .migrations
            .as_deref()
            .unwrap_or("backend/migrations"),
    )
}

pub(super) fn files(directory: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if !directory.is_dir() {
        return Ok(paths);
    }
    let repository = match Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
    {
        Ok(output) => Some(output),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if directory
                .ancestors()
                .any(|parent| parent.join(".git").exists())
            {
                return Err(error).context("Git is required to scan files in a Git repository");
            }
            None
        }
        Err(error) => return Err(error.into()),
    };
    if repository.is_none_or(|output| !output.status.success() || output.stdout != b"true\n") {
        return product_files(directory, extension);
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            ".",
        ])
        .output()?;
    if !output.status.success() {
        bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for name in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let path = directory.join(std::str::from_utf8(name).context("non-UTF-8 Git path")?);
        if path.strip_prefix(directory)?.components().any(
            |part| matches!(part, Component::Normal(name) if excluded_directory(name.to_str())),
        ) {
            continue;
        }
        if path.extension().is_some_and(|value| value == extension)
            && path
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_file())
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub(super) fn product_files(directory: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if directory.is_dir() {
        walk_files(directory, extension, &mut paths)?;
    }
    paths.sort();
    Ok(paths)
}

fn walk_files(directory: &Path, extension: &str, paths: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if !excluded_directory(entry.file_name().to_str()) {
                walk_files(&path, extension, paths)?;
            }
        } else if kind.is_file() && path.extension().is_some_and(|value| value == extension) {
            paths.push(path);
        }
    }
    Ok(())
}

fn excluded_directory(name: Option<&str>) -> bool {
    matches!(
        name,
        Some(
            "target"
                | "node_modules"
                | ".git"
                | ".generated-fixture"
                | ".playwright-browsers"
                | "dist"
                | "coverage"
        )
    )
}

pub(super) struct RustCrate {
    pub manifest: PathBuf,
    pub name: String,
    pub library: PathBuf,
    pub sources: Vec<PathBuf>,
    pub tests: Vec<PathBuf>,
}

pub(super) fn rust_crates(root: &Path, manifest: &Manifest) -> Result<Vec<RustCrate>> {
    workspace_crates(root, &cargo_workspaces(root, manifest)?)
}

pub(super) fn backend_crates(root: &Path, manifest: &Manifest) -> Result<Vec<RustCrate>> {
    workspace_crates(root, &[backend_manifest(root, manifest)?])
}

fn workspace_crates(root: &Path, workspaces: &[PathBuf]) -> Result<Vec<RustCrate>> {
    let mut manifests = Vec::new();
    for workspace_path in workspaces {
        manifests.extend(workspace_members(root, workspace_path)?);
    }
    manifests.sort();
    manifests.dedup();
    manifests
        .into_iter()
        .map(|path| rust_crate(root, path))
        .collect()
}

fn workspace_members(root: &Path, workspace_path: &Path) -> Result<Vec<PathBuf>> {
    let workspace: toml::Value = toml::from_str(&fs::read_to_string(workspace_path)?)?;
    let directory = workspace_path
        .parent()
        .context("backend manifest has no parent")?;
    let mut manifests = Vec::new();
    if workspace.get("package").is_some() {
        manifests.push(workspace_path.to_owned());
    }
    let candidates = product_files(directory, "toml")?;
    if let Some(members) = workspace
        .get("workspace")
        .and_then(|value| value.get("members"))
        .and_then(toml::Value::as_array)
    {
        for member in members {
            let member = member
                .as_str()
                .context("Cargo workspace members must be paths")?;
            product_path(directory, member)?;
            if member == "." {
                manifests.push(workspace_path.to_owned());
                continue;
            }
            let pattern = Glob::new(member)?.compile_matcher();
            let matched = candidates
                .iter()
                .filter(|path| {
                    path.file_name().is_some_and(|name| name == "Cargo.toml")
                        && path
                            .parent()
                            .and_then(|parent| parent.strip_prefix(directory).ok())
                            .is_some_and(|relative| pattern.is_match(relative))
                })
                .cloned()
                .collect::<Vec<_>>();
            if matched.is_empty() {
                bail!(
                    "`{}`: declared Cargo workspace member `{member}` has no manifest",
                    workspace_path.strip_prefix(root)?.display()
                );
            }
            manifests.extend(matched);
        }
    }
    Ok(manifests)
}

fn rust_crate(root: &Path, path: PathBuf) -> Result<RustCrate> {
    let cargo: toml::Value = toml::from_str(&fs::read_to_string(&path)?)?;
    let directory = path.parent().context("crate manifest has no parent")?;
    let name = cargo
        .get("package")
        .and_then(|value| value.get("name"))
        .and_then(toml::Value::as_str)
        .context("declared crate has no package name")?
        .to_owned();
    let library = cargo_path(
        root,
        directory,
        cargo
            .get("lib")
            .and_then(|value| value.get("path"))
            .and_then(toml::Value::as_str)
            .unwrap_or("src/lib.rs"),
    )?;
    let mut sources = files(&directory.join("src"), "rs")?;
    let mut tests = files(&directory.join("tests"), "rs")?;
    if library.is_file() {
        sources.push(library.clone());
    }
    for (key, output) in [("bin", &mut sources), ("test", &mut tests)] {
        if let Some(targets) = cargo.get(key).and_then(toml::Value::as_array) {
            for target in targets {
                if let Some(relative) = target.get("path").and_then(toml::Value::as_str) {
                    let path = cargo_path(root, directory, relative)?;
                    if path.is_file() {
                        output.push(path);
                    }
                }
            }
        }
    }
    sources.sort();
    sources.dedup();
    tests.sort();
    tests.dedup();
    Ok(RustCrate {
        manifest: path,
        name,
        library,
        sources,
        tests,
    })
}

pub(super) fn symbols(source: &str, rust: bool) -> String {
    identity::identity_code(&identity::uncommented(source, rust), rust)
}

fn has_symbol(source: &str, names: &[&str], rust: bool) -> bool {
    let source = if rust {
        source.split("#[cfg(test)]").next().unwrap_or(source)
    } else {
        source
    };
    let code = symbols(source, rust);
    let compact = code.split_whitespace().collect::<String>();
    let words = code
        .split(|value: char| !value.is_ascii_alphanumeric() && value != '_')
        .collect::<Vec<_>>();
    names.iter().any(|name| {
        if name.contains("::") {
            compact.contains(name)
        } else {
            words.contains(name)
        }
    })
}

pub(super) fn require_source(
    root: &Path,
    manifest: &Manifest,
    key: &str,
    candidates: &[PathBuf],
    names: &[&str],
    rust: bool,
    failures: &mut Vec<String>,
) -> Result<()> {
    let paths = if let Some(relative) = manifest.doctor.sources.get(key) {
        vec![product_path(root, relative)?]
    } else {
        candidates.to_vec()
    };
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(path)?;
        let entry = key != "worker_entry"
            || symbols(&source, true)
                .split_whitespace()
                .collect::<String>()
                .contains("fnmain(");
        if entry && has_symbol(&source, names, rust) {
            return Ok(());
        }
    }
    failures.push(format!(
        "missing {key} wiring (expected {})",
        names.join(" or ")
    ));
    Ok(())
}

pub(super) fn uses_redis(root: &Path, manifest: &Manifest) -> Result<bool> {
    for krate in rust_crates(root, manifest)? {
        for path in krate.sources {
            if has_symbol(
                &fs::read_to_string(path)?,
                &["RedisRateLimitStore", "redis::"],
                true,
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn validate_backend_wiring(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let initial = failures.len();
    let cargo = backend_manifest(root, manifest)?;
    let directory = cargo.parent().context("backend manifest has no parent")?;
    for (declared, name) in [
        (manifest.doctor.backend_dockerfile.as_deref(), "Dockerfile"),
        (
            manifest.doctor.backend_dockerignore.as_deref(),
            ".dockerignore",
        ),
    ] {
        let path = match declared {
            Some(relative) => product_path(root, relative)?,
            None => directory.join(name),
        };
        if !path.is_file() {
            failures.push(format!(
                "missing expected backend file `{}`",
                path.strip_prefix(root)?.display()
            ));
        }
    }
    let crates = rust_crates(root, manifest)?;
    if crates.is_empty() {
        failures.push("backend has no declared Cargo packages".to_owned());
    }
    let sources = crates
        .iter()
        .flat_map(|krate| krate.sources.iter().cloned())
        .collect::<Vec<_>>();
    let mut tests = crates
        .iter()
        .flat_map(|krate| krate.tests.iter().cloned())
        .collect::<Vec<_>>();
    tests.extend(files(
        &backend_manifest(root, manifest)?
            .parent()
            .context("backend manifest has no parent")?
            .join("tests"),
        "rs",
    )?);
    require_source(
        root,
        manifest,
        "backend_limits",
        &sources,
        &[
            "check_measurement",
            "check_trimmed_unicode_scalars",
            "check_compact_json_utf8_bytes",
            "trimmed_unicode_scalar_count",
            "compact_json_utf8_bytes",
        ],
        true,
        failures,
    )?;
    if manifest.capabilities.worker {
        require_source(
            root,
            manifest,
            "worker_entry",
            &sources,
            &["ProcessKind::Worker"],
            true,
            failures,
        )?;
        require_source(
            root,
            manifest,
            "worker_tests",
            &tests,
            &["WorkerRunner", "PostgresTestDatabases"],
            true,
            failures,
        )?;
    }
    if manifest.capabilities.auth.is_some() {
        require_source(
            root,
            manifest,
            "auth_tests",
            &tests,
            &["check_auth_router_conformance", "MockOidcServer"],
            true,
            failures,
        )?;
        if let Some(relative) = manifest.doctor.sources.get("pkce_login") {
            let path = product_path(root, relative)?;
            let found = if path.is_file() {
                let source = fs::read_to_string(path)?;
                source.contains("code_challenge") && source.contains("S256")
            } else {
                false
            };
            if !found {
                failures.push(
                    "missing pkce_login wiring (expected code_challenge and S256)".to_owned(),
                );
            }
        }
    }
    if initial == failures.len() {
        successes.push("backend wiring is present in declared Cargo packages".to_owned());
    }
    Ok(())
}

fn screen_imports(source: &str) -> Vec<&str> {
    let code = identity::identity_code(source, false);
    let mut modules = Vec::new();
    for keyword in ["import ", "export "] {
        for (index, _) in code.match_indices(keyword) {
            let statement = &source[index + keyword.len()..];
            if statement.starts_with("type ")
                || (keyword == "export " && !statement.starts_with(['{', '*']))
            {
                continue;
            }
            let Some((bindings, tail)) = statement.split_once(" from ") else {
                continue;
            };
            if bindings.contains(';') {
                continue;
            }
            let tail = tail.trim_start();
            let Some(quote @ ('\'' | '"')) = tail.chars().next() else {
                continue;
            };
            let Some((module, _)) = tail[1..].split_once(quote) else {
                continue;
            };
            if !module.starts_with('.') {
                continue;
            }
            let rendered = bindings.split(['{', '}', ',']).any(|binding| {
                let Some(local) = binding.split_whitespace().last() else {
                    return false;
                };
                code.match_indices(&format!("<{local}"))
                    .any(|(index, matched)| {
                        code[index + matched.len()..].starts_with(|character: char| {
                            character.is_whitespace() || matches!(character, '>' | '/' | '.')
                        })
                    })
            });
            if keyword == "export " || rendered {
                modules.push(module);
            }
        }
    }
    modules
}

fn screen_modules(root: &Path, screens: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    const MAX_IMPORT_DEPTH: usize = 16;
    let mut pending = screens
        .into_iter()
        .map(|path| (path, 0))
        .collect::<VecDeque<_>>();
    let mut modules = BTreeSet::new();
    let root = root.canonicalize()?;
    while let Some((path, depth)) = pending.pop_front() {
        let path = path.canonicalize()?;
        if !path.starts_with(&root) || !modules.insert(path.clone()) {
            continue;
        }
        let source = identity::uncommented(&fs::read_to_string(&path)?, false);
        if depth == MAX_IMPORT_DEPTH {
            continue;
        }
        for module in screen_imports(&source) {
            if let Some(imported) =
                identity::module_path(path.parent().context("screen has no parent")?, module)
            {
                pending.push_back((imported, depth + 1));
            }
        }
    }
    Ok(modules.into_iter().collect())
}

pub(super) fn validate_mobile_auth_wiring(
    root: &Path,
    manifest: &Manifest,
    failures: &mut Vec<String>,
) -> Result<()> {
    let mut paths = files(&root.join("mobile"), "ts")?;
    paths.extend(files(&root.join("mobile"), "tsx")?);
    let (tests, sources): (Vec<_>, Vec<_>) = paths.into_iter().partition(|path| {
        path.file_name().is_some_and(|name| {
            name.to_string_lossy().contains(".test.") || name.to_string_lossy().contains(".spec.")
        })
    });
    let screens = sources
        .iter()
        .filter(|path| {
            path.starts_with(root.join("mobile/app"))
                && path.file_stem().is_none_or(|stem| stem != "_layout")
        })
        .cloned()
        .collect::<Vec<_>>();
    let screens = screen_modules(root, screens)?;
    for (key, candidates, names) in [
        (
            "mobile_sign_in",
            &screens,
            &["signIn", "signInWithOidc", "login"][..],
        ),
        (
            "mobile_auth",
            &sources,
            &[
                "createExpoOidcClient",
                "createNativeOidcClient",
                "NativeOidcClient",
                "createClerkExpoClient",
                "createWorkOsNativeClient",
            ][..],
        ),
        (
            "mobile_auth_tests",
            &tests,
            &["signIn", "signInWithOidc"][..],
        ),
        (
            "mobile_local_data",
            &sources,
            &["ScopedPersistenceRegistryStore"][..],
        ),
        (
            "mobile_persistence",
            &sources,
            &["ScopedPersistenceLifecycle"][..],
        ),
    ] {
        require_source(root, manifest, key, candidates, names, false, failures)?;
    }
    Ok(())
}

fn json_path(
    root: &Path,
    declared: Option<&str>,
    paths: &[PathBuf],
    label: &str,
    matches: impl Fn(&serde_json::Value) -> bool,
    failures: &mut Vec<String>,
) -> Result<Option<PathBuf>> {
    if let Some(relative) = declared {
        let path = product_path(root, relative)?;
        if path.is_file() {
            return Ok(Some(path));
        }
        failures.push(format!("missing {label} file `{relative}`"));
        return Ok(None);
    }
    let mut found = Vec::new();
    for path in paths {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path)?)
            && matches(&value)
        {
            found.push(path.clone());
        }
    }
    match found.as_slice() {
        [path] => Ok(Some(path.clone())),
        [] => {
            failures.push(format!(
                "missing {label}; declare its path in baukit.toml [doctor]"
            ));
            Ok(None)
        }
        _ => {
            failures.push(format!(
                "multiple {label} files; declare its path in baukit.toml [doctor]"
            ));
            Ok(None)
        }
    }
}

fn tool_path(
    root: &Path,
    declared: Option<&str>,
    paths: &[PathBuf],
    name: &str,
    failures: &mut Vec<String>,
) -> Result<Option<PathBuf>> {
    if let Some(relative) = declared {
        let path = product_path(root, relative)?;
        if path.is_file() {
            return Ok(Some(path));
        }
        failures.push(format!("missing Keycloak tool `{relative}`"));
        return Ok(None);
    }
    let mut found = Vec::new();
    let marker = if name == "keycloak_policy.py" {
        "def validate_realm("
    } else {
        "def load_reconcile_config("
    };
    for path in paths {
        let source = fs::read_to_string(path)?;
        if source.contains(marker) && source.contains("argparse.ArgumentParser") {
            found.push(path);
        }
    }
    match found.as_slice() {
        [path] => Ok(Some((*path).clone())),
        [] => {
            failures.push(format!("missing Keycloak tool `{name}`"));
            Ok(None)
        }
        _ => {
            failures.push(format!(
                "multiple Keycloak tools `{name}`; declare its path in baukit.toml [doctor]"
            ));
            Ok(None)
        }
    }
}

pub(super) fn validate_keycloak_realm_tools(
    root: &Path,
    manifest: &Manifest,
    host: &dyn DoctorHost,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let json = files(root, "json")?
        .into_iter()
        .filter(|path| {
            !path
                .strip_prefix(root)
                .expect("discovered product file")
                .components()
                .any(|part| {
                    matches!(
                        part.as_os_str().to_str(),
                        Some("tests" | "test" | "fixtures")
                    )
                })
        })
        .collect::<Vec<_>>();
    let python = files(root, "py")?;
    let paths = &manifest.doctor;
    let realm = json_path(
        root,
        paths.keycloak_realm.as_deref(),
        &json,
        "Keycloak realm",
        |value| {
            value
                .get("realm")
                .and_then(serde_json::Value::as_str)
                .is_some()
                && value.get("clients").is_some()
        },
        failures,
    )?;
    let policy = json_path(
        root,
        paths.keycloak_policy.as_deref(),
        &json,
        "Keycloak policy",
        |value| {
            value.get("requireBruteForceProtection").is_some()
                && value.get("redirectUris").is_some()
        },
        failures,
    )?;
    let config = json_path(
        root,
        paths.keycloak_reconcile.as_deref(),
        &json,
        "Keycloak reconciliation config",
        |value| value.get("realmFields").is_some() && value.get("clients").is_some(),
        failures,
    )?;
    let policy_tool = tool_path(
        root,
        paths.keycloak_policy_tool.as_deref(),
        &python,
        "keycloak_policy.py",
        failures,
    )?;
    let reconcile_tool = tool_path(
        root,
        paths.keycloak_reconcile_tool.as_deref(),
        &python,
        "reconcile_keycloak.py",
        failures,
    )?;
    for (key, markers) in [
        ("keycloak_policy_tests", &["validate_realm"][..]),
        (
            "keycloak_reconcile_tests",
            &[
                "load_reconcile_config",
                "validate_inputs",
                "RealmReconciler",
            ][..],
        ),
    ] {
        let candidates = if let Some(relative) = manifest.doctor.sources.get(key) {
            vec![product_path(root, relative)?]
        } else {
            python.clone()
        };
        let mut found = false;
        for path in candidates {
            if path.is_file() {
                let source = fs::read_to_string(path)?;
                found |= source.contains("unittest")
                    && markers.iter().any(|marker| source.contains(marker));
            }
        }
        if !found {
            failures.push(format!(
                "missing {key} wiring (expected unittest coverage of {})",
                markers.join(" or ")
            ));
        }
    }
    let (Some(realm), Some(policy), Some(config), Some(policy_tool), Some(reconcile_tool)) =
        (realm, policy, config, policy_tool, reconcile_tool)
    else {
        return Ok(());
    };
    let realm_document: serde_json::Value = serde_json::from_str(&fs::read_to_string(&realm)?)?;
    if let Some(name) = realm_document
        .get("realm")
        .and_then(serde_json::Value::as_str)
    {
        identity::validate_admin_realm(root, manifest, name, failures)?;
    }
    let relative = |path: &Path| -> Result<String> {
        Ok(path.strip_prefix(root)?.to_string_lossy().into_owned())
    };
    for (label, arguments) in [
        (
            "Keycloak development realm policy",
            vec![
                relative(&policy_tool)?,
                "--realm".to_owned(),
                relative(&realm)?,
                "--policy".to_owned(),
                relative(&policy)?,
                "--environment-class".to_owned(),
                "development".to_owned(),
            ],
        ),
        (
            "Keycloak reconciliation inputs",
            vec![
                relative(&reconcile_tool)?,
                "--realm".to_owned(),
                relative(&realm)?,
                "--policy".to_owned(),
                relative(&policy)?,
                "--config".to_owned(),
                relative(&config)?,
                "--check".to_owned(),
            ],
        ),
    ] {
        match host.run_command("python3", &arguments, Some(root)) {
            Ok(output) if output.success => successes.push(format!("{label} passed")),
            Ok(output) => failures.push(format!("{label} failed: {}", output.stderr)),
            Err(error) => failures.push(format!("could not run {label}: {error}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_imports_follow_rendered_bindings_and_reexports() {
        let source = "import { LoginScreen as Screen } from './screen'\nimport unused from './unused'\nimport type { Props } from './props';\nexport { default } from './exported';\nexport default function Route() { return <Screen />; }";
        assert_eq!(screen_imports(source), vec!["./screen", "./exported"]);
    }

    #[test]
    fn screen_imports_ignore_unused_imports_after_a_default_export() {
        let source =
            "export default function Route() { return null }\nimport Screen from './unused'\n";
        assert!(screen_imports(source).is_empty());
    }

    #[test]
    fn screen_imports_accept_multiline_jsx() {
        for separator in ["\n", "\r\n", "\t"] {
            let source = format!(
                "import Screen from './screen';\nexport default function Route() {{ return <Screen{separator}label=\"Sign in\" />; }}"
            );
            assert_eq!(screen_imports(&source), vec!["./screen"], "{source}");
        }
    }

    #[test]
    fn screen_modules_resolve_directory_components() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("screen"))?;
        fs::write(
            root.path().join("route.tsx"),
            "import Screen from './screen'; export default function Route() { return <Screen />; }",
        )?;
        fs::write(
            root.path().join("screen/index.tsx"),
            "export default function Screen() { return null; }",
        )?;
        let modules = screen_modules(root.path(), vec![root.path().join("route.tsx")])?;
        assert_eq!(
            modules,
            vec![
                root.path().join("route.tsx"),
                root.path().join("screen/index.tsx")
            ]
        );
        Ok(())
    }

    #[test]
    fn screen_modules_bound_import_depth() -> Result<()> {
        let root = tempfile::tempdir()?;
        for depth in 0..=17 {
            fs::write(
                root.path().join(format!("{depth}.tsx")),
                format!("export {{ default }} from './{}';", depth + 1),
            )?;
        }
        let modules = screen_modules(root.path(), vec![root.path().join("0.tsx")])?;
        assert_eq!(modules.len(), 17);
        assert!(modules.iter().any(|path| path.ends_with("16.tsx")));
        assert!(!modules.iter().any(|path| path.ends_with("17.tsx")));
        Ok(())
    }

    #[test]
    fn git_files_include_tracked_ignored_files_and_scope_subdirectories() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("src"))?;
        fs::write(root.path().join(".gitignore"), "*.py\n")?;
        fs::write(root.path().join("src/tracked.py"), "")?;
        fs::write(root.path().join("src/ignored.py"), "")?;
        fs::write(root.path().join("outside.py"), "")?;
        for arguments in [
            vec!["init", "--quiet"],
            vec!["add", "--force", "src/tracked.py"],
        ] {
            assert!(
                Command::new("git")
                    .args(arguments)
                    .current_dir(root.path())
                    .status()?
                    .success()
            );
        }
        assert_eq!(
            files(&root.path().join("src"), "py")?,
            vec![root.path().join("src/tracked.py")]
        );
        Ok(())
    }
}

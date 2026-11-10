use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSetBuilder};
use toml::{Table, Value};

use super::{CheckContext, Finding, Rule, relative, walk};

const NAME: &str = "lints";
const BASELINE_LINTS: &str = include_str!("../../../lint/rust/lints.toml");
const BASELINE_CLIPPY: &str = include_str!("../../../lint/rust/clippy.toml");
const ESLINT_CONFIG_NAMES: &[&str] = &["eslint.config.js", "eslint.config.mjs", "eslint.config.ts"];
const ESLINT_PACKAGE: &str = "@baukit/eslint-config";
const LOCAL_ESLINT_PACKAGE: &str = "eslint-config";
const LOCAL_TYPESCRIPT_ROOT: &str = "typescript/";
const MAX_TOO_MANY_LINES: i64 = 80;
const LEVELS: &[&str] = &["allow", "warn", "deny", "forbid"];

pub struct Lints;

impl Rule for Lints {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        let baseline = Baseline::embedded()?;
        let mut findings = Vec::new();
        let mut manifests = Vec::new();
        for path in walk::files(context.root, |path| walk::file_name(path) == "Cargo.toml")? {
            match read_toml(&path) {
                Ok(manifest) => manifests.push((manifest, path)),
                Err(error) => findings.push(skipped_manifest(context.root, &path, &error)),
            }
        }
        let workspace_roots = manifests
            .iter()
            .filter(|(manifest, _)| {
                table(manifest, "workspace")
                    .is_some_and(|workspace| workspace.contains_key("members"))
            })
            .filter_map(|(_, path)| path.parent())
            .collect::<Vec<_>>();
        for (manifest, path) in &manifests {
            let is_member = path.parent().is_some_and(|directory| {
                workspace_roots
                    .iter()
                    .any(|root| directory != *root && directory.starts_with(root))
            });
            findings.extend(check_manifest(
                context.root,
                path,
                manifest,
                is_member,
                &baseline,
            )?);
        }
        for config in walk::files(context.root, |path| {
            ESLINT_CONFIG_NAMES.contains(&walk::file_name(path))
        })? {
            findings.extend(check_eslint_config(context.root, &config)?);
        }
        Ok(findings)
    }
}

struct Baseline {
    lints: Table,
    clippy: Table,
}

impl Baseline {
    fn embedded() -> Result<Self> {
        let lints: Table = toml::from_str(BASELINE_LINTS).context("invalid embedded lints.toml")?;
        let lints = table(&lints, "lints")
            .cloned()
            .context("embedded lints.toml has no [lints] table")?;
        let clippy = toml::from_str(BASELINE_CLIPPY).context("invalid embedded clippy.toml")?;
        Ok(Self { lints, clippy })
    }
}

fn table<'a>(parent: &'a Table, key: &str) -> Option<&'a Table> {
    parent.get(key).and_then(Value::as_table)
}

fn skipped_manifest(root: &Path, path: &Path, error: &anyhow::Error) -> Finding {
    Finding::note(
        NAME,
        relative(root, path),
        format!("skipped, not a valid Cargo manifest: {error:#}"),
    )
}

fn read_toml(path: &Path) -> Result<Table> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&source).with_context(|| format!("invalid TOML in {}", path.display()))
}

fn check_manifest(
    root: &Path,
    path: &Path,
    manifest: &Table,
    is_member: bool,
    baseline: &Baseline,
) -> Result<Vec<Finding>> {
    let display = relative(root, path);
    let workspace = table(manifest, "workspace");
    let lints = match workspace {
        Some(workspace) if is_detached_fuzz_workspace(manifest, workspace) => {
            return Ok(Vec::new());
        }
        Some(workspace) if workspace.contains_key("members") => table(workspace, "lints"),
        None if is_member || !manifest.contains_key("package") => return Ok(Vec::new()),
        _ => table(manifest, "lints"),
    };
    let mut findings = lint_table_findings(&display, lints, &baseline.lints);
    let directory = path.parent().unwrap_or(root);
    findings.extend(clippy_findings(root, directory, &baseline.clippy)?);
    if let Some(workspace) = workspace {
        findings.extend(member_findings(root, directory, workspace)?);
    }
    Ok(findings)
}

/// `cargo fuzz` crates declare an empty `[workspace]` only to detach from the parent workspace.
fn is_detached_fuzz_workspace(manifest: &Table, workspace: &Table) -> bool {
    let is_fuzz = table(manifest, "package")
        .and_then(|package| table(package, "metadata"))
        .and_then(|metadata| metadata.get("cargo-fuzz"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    workspace.is_empty() && is_fuzz
}

fn lint_table_findings(display: &str, lints: Option<&Table>, baseline: &Table) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (group, expected) in baseline {
        let Some(expected) = expected.as_table() else {
            continue;
        };
        let actual = lints.and_then(|lints| table(lints, group));
        for (lint, expected_level) in expected {
            let found = actual.and_then(|actual| actual.get(lint)).and_then(level);
            let wanted = level(expected_level).unwrap_or("warn");
            if !found.is_some_and(|found| rank(found) >= rank(wanted)) {
                findings.push(Finding::error(
                    NAME,
                    display,
                    format!(
                        "{group}::{lint} must be at least `{wanted}`, found `{}`",
                        found.unwrap_or("unset")
                    ),
                ));
            }
        }
    }
    findings
}

fn level(value: &Value) -> Option<&str> {
    match value {
        Value::String(level) => Some(level),
        Value::Table(entry) => entry.get("level").and_then(Value::as_str),
        _ => None,
    }
}

fn rank(level: &str) -> Option<usize> {
    LEVELS.iter().position(|known| *known == level)
}

fn clippy_findings(root: &Path, directory: &Path, baseline: &Table) -> Result<Vec<Finding>> {
    let path = directory.join("clippy.toml");
    let display = relative(root, &path);
    if !path.is_file() {
        return Ok(vec![Finding::error(
            NAME,
            display,
            "missing; copy baukit's lint/rust/clippy.toml",
        )]);
    }
    let actual = read_toml(&path)?;
    let mut findings = Vec::new();
    for (key, expected) in baseline {
        let message = match (key.as_str(), actual.get(key)) {
            (_, None) => Some(format!("`{key}` is missing")),
            ("too-many-lines-threshold", Some(value)) => value
                .as_integer()
                .filter(|threshold| *threshold <= MAX_TOO_MANY_LINES)
                .is_none()
                .then(|| format!("`{key}` must be at most {MAX_TOO_MANY_LINES}")),
            ("disallowed-methods", Some(value)) => missing_disallowed(expected, value),
            (key, Some(value)) if key.starts_with("allow-") => value
                .as_bool()
                .is_none()
                .then(|| format!("`{key}` must be true or false")),
            (_, Some(value)) => (value != expected).then(|| format!("`{key}` must be {expected}")),
        };
        findings.extend(message.map(|message| Finding::error(NAME, display.clone(), message)));
    }
    Ok(findings)
}

fn disallowed_paths(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| match entry {
                    Value::String(path) => Some(path.as_str()),
                    Value::Table(entry) => entry.get("path").and_then(Value::as_str),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn missing_disallowed(expected: &Value, actual: &Value) -> Option<String> {
    let actual = disallowed_paths(actual);
    let missing = disallowed_paths(expected)
        .into_iter()
        .filter(|path| !actual.contains(path))
        .collect::<Vec<_>>();
    (!missing.is_empty()).then(|| format!("`disallowed-methods` lacks {}", missing.join(", ")))
}

fn member_findings(root: &Path, directory: &Path, workspace: &Table) -> Result<Vec<Finding>> {
    let members = string_list(workspace, "members");
    if members.is_empty() {
        return Ok(Vec::new());
    }
    let matcher = glob_set(&members)?;
    let excluded = glob_set(&string_list(workspace, "exclude"))?;
    let mut findings = Vec::new();
    for manifest in member_manifests(directory)? {
        let Some(member) = manifest
            .parent()
            .and_then(|dir| dir.strip_prefix(directory).ok())
        else {
            continue;
        };
        if !matcher.is_match(member) || excluded.is_match(member) {
            continue;
        }
        let Ok(parsed) = read_toml(&manifest) else {
            continue;
        };
        let inherits = table(&parsed, "lints")
            .and_then(|lints| lints.get("workspace"))
            .and_then(Value::as_bool)
            == Some(true);
        if !inherits {
            findings.push(Finding::error(
                NAME,
                relative(root, &manifest),
                "member crate must set `[lints] workspace = true`",
            ));
        }
    }
    Ok(findings)
}

fn member_manifests(directory: &Path) -> Result<Vec<PathBuf>> {
    let own = directory.join("Cargo.toml");
    Ok(
        walk::files(directory, |path| walk::file_name(path) == "Cargo.toml")?
            .into_iter()
            .filter(|manifest| *manifest != own)
            .collect(),
    )
}

fn string_list<'a>(parent: &'a Table, key: &str) -> Vec<&'a str> {
    parent
        .get(key)
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn glob_set(patterns: &[&str]) -> Result<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern.trim_end_matches('/'))
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid workspace glob `{pattern}`"))?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}

fn check_eslint_config(root: &Path, path: &Path) -> Result<Option<Finding>> {
    let display = relative(root, path);
    let Some(source) = walk::read_text(path)? else {
        return Ok(None);
    };
    let local_import = display.starts_with(LOCAL_TYPESCRIPT_ROOT)
        && source.lines().any(|line| {
            line.contains("import") && line.contains("./") && line.contains(LOCAL_ESLINT_PACKAGE)
        });
    if source.contains(ESLINT_PACKAGE) || local_import {
        return Ok(None);
    }
    Ok(Some(Finding::error(
        NAME,
        display,
        format!("must import the shared `{ESLINT_PACKAGE}`"),
    )))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::check::{CheckConfig, CheckOptions};

    const MEMBER: &str = "[package]\nname = \"a\"\n\n[lints]\nworkspace = true\n";

    fn run(root: &Path) -> Result<Vec<String>> {
        let config = CheckConfig::default();
        let context = CheckContext {
            root,
            config: &config,
            options: CheckOptions::default(),
        };
        Ok(Lints
            .run(&context)?
            .iter()
            .map(ToString::to_string)
            .collect())
    }

    fn workspace_manifest(lints: &str) -> String {
        format!(
            "[workspace]\nmembers = [\"crates/*\"]\n\n{}",
            lints.replace("[lints.", "[workspace.lints.")
        )
    }

    fn write(root: &Path, path: &str, content: &str) -> Result<()> {
        let file = root.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, content)?;
        Ok(())
    }

    fn clean_tree() -> Result<tempfile::TempDir> {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            "Cargo.toml",
            &workspace_manifest(BASELINE_LINTS),
        )?;
        write(root.path(), "clippy.toml", BASELINE_CLIPPY)?;
        write(root.path(), "crates/a/Cargo.toml", MEMBER)?;
        write(
            root.path(),
            "fuzz/Cargo.toml",
            "[package]\nname = \"f\"\n\n[package.metadata]\ncargo-fuzz = true\n\n[workspace]\n",
        )?;
        write(
            root.path(),
            "web/eslint.config.js",
            "import baukit from '@baukit/eslint-config';\nexport default baukit;\n",
        )?;
        Ok(root)
    }

    #[test]
    fn clean_tree_passes() -> Result<()> {
        let root = clean_tree()?;
        assert_eq!(run(root.path())?, Vec::<String>::new());
        Ok(())
    }

    #[test]
    fn stricter_levels_pass_and_weaker_levels_fail() -> Result<()> {
        let root = clean_tree()?;
        let lints = BASELINE_LINTS
            .replace("unsafe_code = \"deny\"", "unsafe_code = \"forbid\"")
            .replace("todo = \"warn\"", "todo = \"allow\"")
            .replace("panic = \"warn\"\n", "");
        write(root.path(), "Cargo.toml", &workspace_manifest(&lints))?;
        assert_eq!(
            run(root.path())?,
            vec![
                "lints: Cargo.toml: clippy::panic must be at least `warn`, found `unset`",
                "lints: Cargo.toml: clippy::todo must be at least `warn`, found `allow`",
            ]
        );
        Ok(())
    }

    #[test]
    fn standalone_package_uses_its_own_lints_table() -> Result<()> {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            "Cargo.toml",
            &format!("[package]\nname = \"s\"\n\n[workspace]\n\n{BASELINE_LINTS}"),
        )?;
        write(root.path(), "clippy.toml", BASELINE_CLIPPY)?;
        assert_eq!(run(root.path())?, Vec::<String>::new());
        write(root.path(), "Cargo.toml", "[package]\nname = \"s\"\n")?;
        assert_eq!(run(root.path())?.len(), 24);
        Ok(())
    }

    #[test]
    fn clippy_toml_must_cover_the_baseline() -> Result<()> {
        let root = clean_tree()?;
        let clippy = BASELINE_CLIPPY
            .replace(
                "too-many-lines-threshold = 80",
                "too-many-lines-threshold = 100",
            )
            .replace(
                "allow-print-in-tests = true",
                "allow-print-in-tests = false",
            )
            .replace("allow-panic-in-tests = true\n", "")
            .replace("std::thread::sleep", "std::process::exit");
        write(root.path(), "clippy.toml", &clippy)?;
        assert_eq!(
            run(root.path())?,
            vec![
                "lints: clippy.toml: `allow-panic-in-tests` is missing",
                "lints: clippy.toml: `disallowed-methods` lacks std::thread::sleep",
                "lints: clippy.toml: `too-many-lines-threshold` must be at most 80",
            ]
        );
        fs::remove_file(root.path().join("clippy.toml"))?;
        assert_eq!(
            run(root.path())?,
            vec!["lints: clippy.toml: missing; copy baukit's lint/rust/clippy.toml"]
        );
        Ok(())
    }

    #[test]
    fn members_must_inherit_workspace_lints() -> Result<()> {
        let root = clean_tree()?;
        write(
            root.path(),
            "crates/b/Cargo.toml",
            "[package]\nname = \"b\"\n",
        )?;
        assert_eq!(
            run(root.path())?,
            vec!["lints: crates/b/Cargo.toml: member crate must set `[lints] workspace = true`"]
        );
        Ok(())
    }

    #[test]
    fn eslint_configs_must_import_the_shared_config() -> Result<()> {
        let root = clean_tree()?;
        write(
            root.path(),
            "mobile/eslint.config.mjs",
            "export default [];\n",
        )?;
        write(
            root.path(),
            "typescript/eslint.config.js",
            "import baukit from './packages/eslint-config/index.js';\n",
        )?;
        write(
            root.path(),
            "other/eslint.config.ts",
            "import baukit from './packages/eslint-config/index.js';\n",
        )?;
        assert_eq!(
            run(root.path())?,
            vec![
                "lints: mobile/eslint.config.mjs: must import the shared `@baukit/eslint-config`",
                "lints: other/eslint.config.ts: must import the shared `@baukit/eslint-config`",
            ]
        );
        Ok(())
    }
}

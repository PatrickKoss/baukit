use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use serde_json::Value;

const PACKAGE_MANIFESTS: &[&str] =
    include!(concat!(env!("OUT_DIR"), "/baukit_package_manifests.rs"));
const SCOPE: &str = "@baukit/";
const PNPM_MARKERS: [&str; 2] = ["pnpm-workspace.yaml", "pnpm-lock.yaml"];
const DIRECT_SECTIONS: [&str; 2] = ["dependencies", "devDependencies"];
const TRANSITIVE_SECTIONS: [&str; 3] = ["dependencies", "peerDependencies", "optionalDependencies"];
const SKIPPED_DIRECTORIES: [&str; 2] = ["node_modules", "target"];

type PackageGraph = BTreeMap<String, BTreeSet<String>>;

#[derive(Deserialize)]
struct WorkspaceFile {
    #[serde(default)]
    packages: Vec<String>,
}

/// Prints the `minimumReleaseAgeExclude` entries each pnpm app of the product
/// needs while its `@baukit/*` pins are younger than pnpm's release age: every
/// pinned package and every `@baukit/*` package those pull in, at the pinned version.
///
/// # Errors
///
/// Fails when a `package.json` cannot be read or parsed, or when the `@baukit/*`
/// pins of one app name different versions.
pub fn release_age_excludes(root: &Path) -> Result<Vec<String>> {
    let graph = package_graph(PACKAGE_MANIFESTS)?;
    let directories = directories(root)?;
    let mut lines = Vec::new();
    for workspace in directories
        .iter()
        .filter(|dir| is_pnpm_app(&root.join(dir)))
    {
        let label = workspace.join(PNPM_MARKERS[0]).display().to_string();
        let packages = workspace_manifests(root, workspace, &directories)?
            .iter()
            .map(|manifest| read_package(&root.join(manifest)))
            .collect::<Result<Vec<_>>>()?;
        let Some((version, entries)) =
            app_excludes(&packages, &graph).with_context(|| label.clone())?
        else {
            continue;
        };
        lines.push(format!("# {label}, until @baukit/*@{version} is a day old"));
        lines.push("minimumReleaseAgeExclude:".to_owned());
        lines.extend(entries.iter().map(|name| format!("  - '{name}@{version}'")));
    }
    Ok(lines)
}

fn read_package(manifest: &Path) -> Result<Value> {
    let source = fs::read_to_string(manifest)
        .with_context(|| format!("could not read {}", manifest.display()))?;
    serde_json::from_str(&source).with_context(|| format!("could not parse {}", manifest.display()))
}

/// Every directory below `root` relative to it, sorted, with the root itself first.
fn directories(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = vec![PathBuf::new()];
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let absolute = root.join(&relative);
        let entries = fs::read_dir(&absolute)
            .with_context(|| format!("could not read {}", absolute.display()))?;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !entry.file_type()?.is_dir() || is_skipped(name) {
                continue;
            }
            found.push(relative.join(name));
            pending.push(relative.join(name));
        }
    }
    found.sort();
    Ok(found)
}

fn is_skipped(name: &str) -> bool {
    name.starts_with('.') || SKIPPED_DIRECTORIES.contains(&name)
}

/// The `package.json` of the workspace and of each member its `packages:` globs name.
fn workspace_manifests(
    root: &Path,
    workspace: &Path,
    directories: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    let members = Members::read(&root.join(workspace).join(PNPM_MARKERS[0]))?;
    let mut manifests = vec![workspace.join("package.json")];
    for directory in directories {
        let Ok(relative) = directory.strip_prefix(workspace) else {
            continue;
        };
        let manifest = directory.join("package.json");
        if relative.as_os_str().is_empty()
            || !members.is_match(relative)
            || !root.join(&manifest).is_file()
        {
            continue;
        }
        manifests.push(manifest);
    }
    Ok(manifests)
}

/// The `packages:` globs of a `pnpm-workspace.yaml`; a `!` glob removes members.
struct Members {
    included: GlobSet,
    excluded: GlobSet,
}

impl Members {
    fn read(workspace_file: &Path) -> Result<Self> {
        let mut included = GlobSetBuilder::new();
        let mut excluded = GlobSetBuilder::new();
        if workspace_file.is_file() {
            let source = fs::read_to_string(workspace_file)
                .with_context(|| format!("could not read {}", workspace_file.display()))?;
            let workspace: WorkspaceFile = serde_yaml_ng::from_str(&source)
                .with_context(|| format!("could not parse {}", workspace_file.display()))?;
            for pattern in &workspace.packages {
                let (set, pattern) = match pattern.strip_prefix('!') {
                    Some(negated) => (&mut excluded, negated),
                    None => (&mut included, pattern.as_str()),
                };
                let pattern = pattern.trim_start_matches("./").trim_end_matches('/');
                set.add(GlobBuilder::new(pattern).literal_separator(true).build()?);
            }
        }
        Ok(Self {
            included: included.build()?,
            excluded: excluded.build()?,
        })
    }

    fn is_match(&self, relative: &Path) -> bool {
        self.included.is_match(relative) && !self.excluded.is_match(relative)
    }
}

fn package_graph(manifests: &[&str]) -> Result<PackageGraph> {
    let mut graph = PackageGraph::new();
    for source in manifests {
        let package: Value = serde_json::from_str(source)?;
        let Some(name) = package.get("name").and_then(Value::as_str) else {
            continue;
        };
        graph.insert(
            name.to_owned(),
            scoped_names(&package, &TRANSITIVE_SECTIONS)
                .into_keys()
                .collect(),
        );
    }
    Ok(graph)
}

fn is_pnpm_app(directory: &Path) -> bool {
    directory.join("package.json").is_file()
        && PNPM_MARKERS
            .iter()
            .any(|marker| directory.join(marker).is_file())
}

fn app_excludes(
    packages: &[Value],
    graph: &PackageGraph,
) -> Result<Option<(String, BTreeSet<String>)>> {
    let direct = packages
        .iter()
        .flat_map(|package| scoped_names(package, &DIRECT_SECTIONS))
        .collect::<Vec<_>>();
    let versions = direct
        .iter()
        .map(|(_, version)| version)
        .collect::<BTreeSet<_>>();
    let Some(version) = versions.first() else {
        return Ok(None);
    };
    if versions.len() > 1 {
        bail!("the @baukit/* pins name more than one version: {versions:?}");
    }
    let version = (*version).clone();
    let mut pending = direct.into_iter().map(|(name, _)| name).collect::<Vec<_>>();
    let mut closure = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !closure.insert(name.clone()) {
            continue;
        }
        pending.extend(graph.get(&name).into_iter().flatten().cloned());
    }
    Ok(Some((version, closure)))
}

fn scoped_names(package: &Value, sections: &[&str]) -> BTreeMap<String, String> {
    sections
        .iter()
        .filter_map(|section| package.get(section).and_then(Value::as_object))
        .flatten()
        .filter(|(name, _)| name.starts_with(SCOPE))
        .map(|(name, spec)| {
            let version = spec
                .as_str()
                .unwrap_or_default()
                .trim_start_matches(['^', '~']);
            (name.clone(), version.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_cover_the_baukit_packages_a_pin_pulls_in() -> Result<()> {
        let graph = package_graph(&[
            r#"{"name":"@baukit/suite-client","dependencies":{"@baukit/type-guards":"^0.30.3","@noble/curves":"1.9.7"},"peerDependencies":{"@baukit/integrations-client":"^0.30.3"}}"#,
            r#"{"name":"@baukit/integrations-client","dependencies":{"@baukit/type-guards":"^0.30.3"}}"#,
            r#"{"name":"@baukit/type-guards"}"#,
        ])?;
        let app = r#"{"dependencies":{"@baukit/suite-client":"0.30.3","react":"19.2.3"}}"#;
        let (version, names) =
            app_excludes(&[serde_json::from_str(app)?], &graph)?.context("excludes")?;
        assert_eq!(version, "0.30.3");
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            [
                "@baukit/integrations-client",
                "@baukit/suite-client",
                "@baukit/type-guards"
            ]
        );
        Ok(())
    }

    #[test]
    fn mixed_baukit_pins_are_rejected() -> Result<()> {
        let app = r#"{"dependencies":{"@baukit/csp":"0.30.3"},"devDependencies":{"@baukit/eslint-config":"0.30.2"}}"#;
        let error = app_excludes(&[serde_json::from_str(app)?], &PackageGraph::new())
            .expect_err("mixed pins");
        assert!(
            error.to_string().contains("more than one version"),
            "{error}"
        );
        Ok(())
    }

    #[test]
    fn the_embedded_graph_knows_the_suite_client_dependencies() -> Result<()> {
        let graph = package_graph(PACKAGE_MANIFESTS)?;
        let suite = graph.get("@baukit/suite-client").context("suite-client")?;
        assert!(suite.contains("@baukit/type-guards"), "{suite:?}");
        assert!(suite.contains("@baukit/integrations-client"), "{suite:?}");
        Ok(())
    }

    #[test]
    fn products_print_one_block_per_pnpm_app() -> Result<()> {
        let root = tempfile::tempdir()?;
        pnpm_app(root.path(), "mobile", "pnpm-workspace.yaml")?;
        let lines = release_age_excludes(root.path())?;
        assert_eq!(
            lines[0],
            "# mobile/pnpm-workspace.yaml, until @baukit/*@0.30.3 is a day old"
        );
        assert_eq!(lines[1], "minimumReleaseAgeExclude:");
        assert!(
            lines.contains(&"  - '@baukit/type-guards@0.30.3'".to_owned()),
            "{lines:?}"
        );
        Ok(())
    }

    #[test]
    fn every_top_level_pnpm_app_gets_a_block() -> Result<()> {
        let root = tempfile::tempdir()?;
        pnpm_app(root.path(), "web", "pnpm-workspace.yaml")?;
        pnpm_app(root.path(), "extension", "pnpm-lock.yaml")?;
        let headers = release_age_excludes(root.path())?
            .into_iter()
            .filter(|line| line.starts_with('#'))
            .collect::<Vec<_>>();
        assert_eq!(
            headers,
            [
                "# extension/pnpm-workspace.yaml, until @baukit/*@0.30.3 is a day old",
                "# web/pnpm-workspace.yaml, until @baukit/*@0.30.3 is a day old",
            ]
        );
        Ok(())
    }

    #[test]
    fn directories_without_pnpm_are_skipped() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("tools"))?;
        fs::write(
            root.path().join("tools/package.json"),
            r#"{"dependencies":{"@baukit/suite-client":"0.30.3"}}"#,
        )?;
        fs::write(root.path().join("package.json"), "{}")?;
        assert!(release_age_excludes(root.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn a_root_workspace_collects_its_member_pins() -> Result<()> {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - mobile\n  - packages/*\n",
        )?;
        write(
            root.path(),
            "package.json",
            r#"{"devDependencies":{"@baukit/eslint-config":"0.30.3"}}"#,
        )?;
        write(
            root.path(),
            "mobile/package.json",
            r#"{"dependencies":{"@baukit/csp":"0.30.3"}}"#,
        )?;
        write(
            root.path(),
            "packages/theme/package.json",
            r#"{"dependencies":{"@baukit/ui-tokens":"0.30.3"}}"#,
        )?;
        write(
            root.path(),
            "tools/package.json",
            r#"{"dependencies":{"@baukit/navigation":"0.30.3"}}"#,
        )?;
        write(root.path(), "node_modules/x/pnpm-workspace.yaml", "")?;
        write(
            root.path(),
            "node_modules/x/package.json",
            r#"{"dependencies":{"@baukit/csp":"0.30.3"}}"#,
        )?;
        assert_eq!(
            release_age_excludes(root.path())?,
            [
                "# pnpm-workspace.yaml, until @baukit/*@0.30.3 is a day old",
                "minimumReleaseAgeExclude:",
                "  - '@baukit/csp@0.30.3'",
                "  - '@baukit/eslint-config@0.30.3'",
                "  - '@baukit/type-guards@0.30.3'",
                "  - '@baukit/ui-tokens@0.30.3'",
            ]
        );
        Ok(())
    }

    #[test]
    fn nested_workspaces_get_their_own_block() -> Result<()> {
        let root = tempfile::tempdir()?;
        pnpm_app(root.path(), "web", "pnpm-workspace.yaml")?;
        write(root.path(), "web/e2e/pnpm-workspace.yaml", "")?;
        write(
            root.path(),
            "web/e2e/package.json",
            r#"{"devDependencies":{"@baukit/csp":"0.30.3"}}"#,
        )?;
        let lines = release_age_excludes(root.path())?;
        let e2e = lines
            .iter()
            .position(|line| line.starts_with("# web/e2e/"))
            .context("e2e block")?;
        assert_eq!(
            lines[e2e],
            "# web/e2e/pnpm-workspace.yaml, until @baukit/*@0.30.3 is a day old"
        );
        assert_eq!(lines[e2e + 2..], ["  - '@baukit/csp@0.30.3'"]);
        assert!(
            lines[0].starts_with("# web/pnpm-workspace.yaml"),
            "{lines:?}"
        );
        Ok(())
    }

    #[test]
    fn member_package_pins_count_for_their_workspace() -> Result<()> {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            "packages/pnpm-workspace.yaml",
            "packages:\n  - game-client\n",
        )?;
        write(
            root.path(),
            "packages/package.json",
            r#"{"devDependencies":{"@baukit/eslint-config":"0.30.3"}}"#,
        )?;
        write(
            root.path(),
            "packages/game-client/package.json",
            r#"{"dependencies":{"@baukit/csp":"0.30.3"}}"#,
        )?;
        assert_eq!(
            release_age_excludes(root.path())?,
            [
                "# packages/pnpm-workspace.yaml, until @baukit/*@0.30.3 is a day old",
                "minimumReleaseAgeExclude:",
                "  - '@baukit/csp@0.30.3'",
                "  - '@baukit/eslint-config@0.30.3'",
            ]
        );
        Ok(())
    }

    #[test]
    fn negated_globs_drop_members() -> Result<()> {
        let root = tempfile::tempdir()?;
        write(
            root.path(),
            "pnpm-workspace.yaml",
            "packages:\n  - packages/*\n  - '!packages/legacy'\n",
        )?;
        write(root.path(), "package.json", "{}")?;
        write(
            root.path(),
            "packages/theme/package.json",
            r#"{"dependencies":{"@baukit/csp":"0.30.3"}}"#,
        )?;
        write(
            root.path(),
            "packages/legacy/package.json",
            r#"{"dependencies":{"@baukit/csp":"0.30.2"}}"#,
        )?;
        assert_eq!(
            release_age_excludes(root.path())?,
            [
                "# pnpm-workspace.yaml, until @baukit/*@0.30.3 is a day old",
                "minimumReleaseAgeExclude:",
                "  - '@baukit/csp@0.30.3'",
            ]
        );
        Ok(())
    }

    #[test]
    fn packages_without_a_workspace_and_no_root_workspace_are_skipped() -> Result<()> {
        let root = tempfile::tempdir()?;
        pnpm_app(root.path(), "web", "pnpm-workspace.yaml")?;
        write(
            root.path(),
            "tools/ci/pnpm-workspace.yaml",
            "packages: []\n",
        )?;
        write(root.path(), "tools/ci/pnpm-lock.yaml", "")?;
        write(root.path(), "tools/ci/package.json", "{}")?;
        write(
            root.path(),
            "packages/sdk-typescript/package.json",
            r#"{"dependencies":{"@baukit/csp":"0.30.3"}}"#,
        )?;
        let headers = release_age_excludes(root.path())?
            .into_iter()
            .filter(|line| line.starts_with('#'))
            .collect::<Vec<_>>();
        assert_eq!(
            headers,
            ["# web/pnpm-workspace.yaml, until @baukit/*@0.30.3 is a day old"]
        );
        Ok(())
    }

    fn write(root: &Path, file: &str, contents: &str) -> Result<()> {
        let path = root.join(file);
        fs::create_dir_all(path.parent().context("parent")?)?;
        fs::write(path, contents)?;
        Ok(())
    }

    fn pnpm_app(root: &Path, directory: &str, marker: &str) -> Result<()> {
        let app = root.join(directory);
        fs::create_dir_all(&app)?;
        fs::write(
            app.join("package.json"),
            r#"{"dependencies":{"@baukit/suite-client":"0.30.3"}}"#,
        )?;
        fs::write(app.join(marker), "")?;
        Ok(())
    }
}

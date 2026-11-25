use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

const PACKAGE_MANIFESTS: &[&str] =
    include!(concat!(env!("OUT_DIR"), "/baukit_package_manifests.rs"));
const SCOPE: &str = "@baukit/";
const APP_DIRECTORIES: [&str; 2] = ["mobile", "web"];
const DIRECT_SECTIONS: [&str; 2] = ["dependencies", "devDependencies"];
const TRANSITIVE_SECTIONS: [&str; 3] = ["dependencies", "peerDependencies", "optionalDependencies"];

type PackageGraph = BTreeMap<String, BTreeSet<String>>;

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
    let mut lines = Vec::new();
    for directory in APP_DIRECTORIES {
        let manifest = root.join(directory).join("package.json");
        if !manifest.is_file() {
            continue;
        }
        let source = fs::read_to_string(&manifest)
            .with_context(|| format!("could not read {}", manifest.display()))?;
        let Some((version, entries)) =
            app_excludes(&source, &graph).with_context(|| format!("{directory}/package.json"))?
        else {
            continue;
        };
        lines.push(format!(
            "# {directory}/pnpm-workspace.yaml, until @baukit/*@{version} is a day old"
        ));
        lines.push("minimumReleaseAgeExclude:".to_owned());
        lines.extend(entries.iter().map(|name| format!("  - '{name}@{version}'")));
    }
    Ok(lines)
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

fn app_excludes(source: &str, graph: &PackageGraph) -> Result<Option<(String, BTreeSet<String>)>> {
    let package: Value = serde_json::from_str(source)?;
    let direct = scoped_names(&package, &DIRECT_SECTIONS);
    let versions = direct.values().collect::<BTreeSet<_>>();
    let Some(version) = versions.first() else {
        return Ok(None);
    };
    if versions.len() > 1 {
        bail!("the @baukit/* pins name more than one version: {versions:?}");
    }
    let version = (*version).clone();
    let mut pending = direct.into_keys().collect::<Vec<_>>();
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
        let (version, names) = app_excludes(app, &graph)?.context("excludes")?;
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
        let error = app_excludes(app, &PackageGraph::new()).expect_err("mixed pins");
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
        fs::create_dir_all(root.path().join("mobile"))?;
        fs::write(
            root.path().join("mobile/package.json"),
            r#"{"dependencies":{"@baukit/suite-client":"0.30.3"}}"#,
        )?;
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
}

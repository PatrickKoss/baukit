use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::manifest::Manifest;

const GITOPS_DIR: &str = "deploy/gitops";
const GITOPS_BASE: &str = "base";
const DEDICATED_COMPONENT: &str = "postgres-dedicated";

#[derive(Deserialize)]
struct Kustomization {
    #[serde(default)]
    components: Vec<String>,
}

/// `[postgres] owner_create_role` gives the owner `CREATEROLE`. Role names are global to a
/// Postgres cluster, so on the shared platform cluster two tenants could collide on them; only
/// dedicated Postgres may use the flag.
pub(crate) fn validate(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    if !manifest.postgres.owner_create_role {
        return Ok(());
    }
    if !manifest.capabilities.components.backend() {
        failures.push("postgres.owner_create_role requires the backend capability".to_owned());
        return Ok(());
    }
    let shared = shared_postgres_environments(&root.join(GITOPS_DIR))?;
    if shared.is_empty() {
        successes.push(
            "postgres.owner_create_role: every deploy/gitops environment runs dedicated Postgres"
                .to_owned(),
        );
        return Ok(());
    }
    failures.push(format!(
        "postgres.owner_create_role needs dedicated Postgres, but {} use shared Postgres; \
         list backing/{DEDICATED_COMPONENT} in their kustomization.yaml components or drop the flag",
        shared
            .iter()
            .map(|environment| format!("{GITOPS_DIR}/{environment}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    Ok(())
}

/// Environments under `gitops` whose overlay does not list the dedicated Postgres component.
fn shared_postgres_environments(gitops: &Path) -> Result<Vec<String>> {
    if !gitops.is_dir() {
        return Ok(Vec::new());
    }
    let mut shared = Vec::new();
    for entry in
        fs::read_dir(gitops).with_context(|| format!("could not list {}", gitops.display()))?
    {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let overlay = entry.path().join("kustomization.yaml");
        if name == GITOPS_BASE || !overlay.is_file() {
            continue;
        }
        if !uses_dedicated_postgres(&overlay)? {
            shared.push(name);
        }
    }
    shared.sort();
    Ok(shared)
}

fn uses_dedicated_postgres(overlay: &Path) -> Result<bool> {
    let source = fs::read_to_string(overlay)
        .with_context(|| format!("could not read {}", overlay.display()))?;
    let kustomization: Kustomization = serde_yaml_ng::from_str(&source)
        .with_context(|| format!("could not parse {}", overlay.display()))?;
    Ok(kustomization.components.iter().any(|component| {
        Path::new(component)
            .file_name()
            .is_some_and(|name| name == DEDICATED_COMPONENT)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(root: &Path, environment: &str, components: &str) -> Result<()> {
        let directory = root.join(GITOPS_DIR).join(environment);
        fs::create_dir_all(&directory)?;
        fs::write(
            directory.join("kustomization.yaml"),
            format!(
                "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\ncomponents: {components}\n"
            ),
        )?;
        Ok(())
    }

    #[test]
    fn lists_environments_without_the_dedicated_component() -> Result<()> {
        let root = tempfile::tempdir()?;
        overlay(root.path(), "base", "[]")?;
        overlay(
            root.path(),
            "testing",
            "[backing/postgres-dedicated, backing/redis-dedicated]",
        )?;
        overlay(root.path(), "production", "[backing/redis-shared]")?;
        overlay(root.path(), "staging", "[]")?;
        assert_eq!(
            shared_postgres_environments(&root.path().join(GITOPS_DIR))?,
            ["production", "staging"]
        );
        Ok(())
    }

    #[test]
    fn a_product_without_overlays_has_no_shared_environment() -> Result<()> {
        let root = tempfile::tempdir()?;
        assert!(shared_postgres_environments(&root.path().join(GITOPS_DIR))?.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_an_overlay_that_does_not_parse() -> Result<()> {
        let root = tempfile::tempdir()?;
        overlay(root.path(), "testing", "{unclosed")?;
        assert!(shared_postgres_environments(&root.path().join(GITOPS_DIR)).is_err());
        Ok(())
    }
}

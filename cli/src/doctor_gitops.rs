use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::manifest::Manifest;

const CHART_SOURCE: &str = "deploy/gitops/base/chart-source.yaml";

#[derive(Deserialize)]
struct GitRepository {
    spec: Spec,
}

#[derive(Deserialize)]
struct Spec {
    #[serde(rename = "ref")]
    reference: Reference,
}

#[derive(Deserialize)]
struct Reference {
    tag: Option<String>,
}

/// Flux renders the product with the chart at the chart source's tag, so a re-pin that leaves
/// the tag behind keeps deploying the old chart.
pub(crate) fn validate(
    root: &Path,
    manifest: &Manifest,
    successes: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Result<()> {
    match chart_tag_finding(root, &manifest.template_version)? {
        Some(Ok(success)) => successes.push(success),
        Some(Err(failure)) => failures.push(failure),
        None => {}
    }
    Ok(())
}

/// `None` when the product has no gitops chart source.
fn chart_tag_finding(
    root: &Path,
    template_version: &str,
) -> Result<Option<Result<String, String>>> {
    let path = root.join(CHART_SOURCE);
    if !path.is_file() {
        return Ok(None);
    }
    let expected = format!("v{template_version}");
    Ok(Some(match chart_tag(&path)? {
        Some(tag) if tag == expected => Ok(format!("{CHART_SOURCE} pins the chart at {expected}")),
        Some(tag) => Err(format!(
            "{CHART_SOURCE} sets ref.tag {tag}, but template_version is {template_version}; set ref.tag to {expected}"
        )),
        None => Err(format!(
            "{CHART_SOURCE} has no ref.tag; set it to {expected}"
        )),
    }))
}

fn chart_tag(path: &Path) -> Result<Option<String>> {
    let source =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let repository: GitRepository = serde_yaml_ng::from_str(&source)
        .with_context(|| format!("could not parse {}", path.display()))?;
    Ok(repository.spec.reference.tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_product_without_a_chart_source_skips_the_check() -> Result<()> {
        let root = tempfile::tempdir()?;
        assert_eq!(chart_tag_finding(root.path(), "0.30.1")?, None);
        Ok(())
    }

    #[test]
    fn a_chart_source_without_a_tag_fails() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join(CHART_SOURCE);
        fs::create_dir_all(path.parent().unwrap_or(root.path()))?;
        fs::write(&path, "spec:\n  ref:\n    branch: main\n")?;
        assert_eq!(
            chart_tag_finding(root.path(), "0.30.1")?,
            Some(Err(format!(
                "{CHART_SOURCE} has no ref.tag; set it to v0.30.1"
            )))
        );
        Ok(())
    }
}

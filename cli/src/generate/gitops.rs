use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::manifest::read_manifest;

use super::template_files::render_matching;

/// Files `baukit generate gitops` adds to an existing product: the overlays, the release and
/// promote workflows, and the scripts they call.
const GITOPS_FILES: &[&str] = &[
    "deploy/gitops/**",
    "deploy/cosign-signing-config.json",
    ".github/workflows/release.yml",
    ".github/workflows/promote.yml",
    "scripts/release.sh",
    "scripts/promote.sh",
    "scripts/image-signing.sh",
    "scripts/gitops-git.sh",
    "scripts/gitops-pins.py",
    "scripts/gitops-pins.test.py",
    "scripts/release-dry-run.test.py",
    "scripts/docker.sh",
];

fn gitops_set() -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in GITOPS_FILES {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

/// Writes the GitOps files missing from the product at `root` and returns one line per file:
/// `added <path>` or `kept <path>` for an existing file that differs from the template.
///
/// # Errors
///
/// Fails when `baukit.toml` is missing or invalid, the product has no backend, or a file cannot
/// be read or written.
pub fn generate_gitops(root: &Path) -> Result<Vec<String>> {
    if !read_manifest(root)?.capabilities.components.backend() {
        bail!("GitOps overlays deploy backend images; baukit.toml has backend = false");
    }
    let mut lines = Vec::new();
    for (relative, contents) in render_matching(root, &gitops_set()?)? {
        let path = root.join(&relative);
        if path.exists() {
            let existing =
                fs::read(&path).with_context(|| format!("could not read {}", path.display()))?;
            if existing != contents {
                lines.push(format!("kept {relative} (differs from the template)"));
            }
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, contents)
            .with_context(|| format!("could not write {}", path.display()))?;
        lines.push(format!("added {relative}"));
    }
    lines.push(
        "next: run `baukit check template-files --update` and follow deploy/gitops/README.md"
            .to_owned(),
    );
    Ok(lines)
}

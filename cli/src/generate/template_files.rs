use std::{collections::BTreeMap, path::Path};

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::{
    check::{TEMPLATE_FILES_MANIFEST, render_manifest, sha256_hex},
    manifest::read_manifest,
};

use super::{NewOptions, render::RenderedFiles, render_for_destination};

/// Rendered files the product extends through hooks instead of editing. The criterion: baukit
/// tooling whose logic is the same in every product, so a fork stops template fixes from
/// reaching it. Product code, product configuration and docs about the product stay out.
const TEMPLATE_OWNED: &[&str] = &[
    // Gate, OpenAPI, lockfile, Keycloak reconcile and lint scripts with their tests and fixtures.
    "scripts/**",
    // Web and mobile build, token and QA harness scripts.
    "web/scripts/**",
    "mobile/scripts/**",
    // CI; product jobs go through the quality-gate.local.sh hook.
    ".github/workflows/**",
    // Docs that describe the owned tooling.
    "docs/quality-gate.md",
    "docs/observability-lint.md",
    "docs/openapi-drift.md",
    // Baukit's agent skills.
    ".agents/skills/**",
    ".claude/skills/**",
];

fn owned_set() -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in TEMPLATE_OWNED {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

/// Renders the template-owned files `baukit new` would write for the product at `root`, keyed by
/// product-relative path.
pub(crate) fn render_template_owned(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let manifest = read_manifest(root)?;
    let options = NewOptions {
        name: manifest.app.name,
        directory: root.to_owned(),
        components: manifest.capabilities.components,
        auth: manifest.capabilities.auth,
        force: false,
        into_existing: true,
        resolve_lockfiles: false,
        baukit_path: None,
        port_offset: manifest.port_offset,
        quality: manifest.quality.profile,
    };
    let owned = owned_set()?;
    Ok(render_for_destination(&options, root)?
        .into_iter()
        .filter(|(path, _)| owned.is_match(path))
        .map(|(path, contents)| (path.to_string_lossy().replace('\\', "/"), contents))
        .collect())
}

/// Adds `.baukit/template-files.toml` with the hash of every template-owned rendered file.
pub(crate) fn add_template_files_manifest(rendered: &mut RenderedFiles) -> Result<()> {
    let owned = owned_set()?;
    let files: BTreeMap<String, String> = rendered
        .iter()
        .filter(|(path, _)| owned.is_match(path))
        .map(|(path, contents)| {
            (
                path.to_string_lossy().replace('\\', "/"),
                sha256_hex(contents),
            )
        })
        .collect();
    rendered.insert(
        TEMPLATE_FILES_MANIFEST.into(),
        render_manifest(files)?.into_bytes(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn manifest_lists_only_template_owned_files() -> Result<()> {
        let mut rendered = RenderedFiles::new();
        for path in [
            "scripts/quality-gate.sh",
            ".github/workflows/ci.yml",
            "backend/Cargo.toml",
            "docs/adr/0001.md",
        ] {
            rendered.insert(PathBuf::from(path), b"x\n".to_vec());
        }
        add_template_files_manifest(&mut rendered)?;
        let manifest = rendered
            .get(&PathBuf::from(TEMPLATE_FILES_MANIFEST))
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        assert!(
            manifest.contains("\"scripts/quality-gate.sh\" = "),
            "{manifest}"
        );
        assert!(
            manifest.contains("\".github/workflows/ci.yml\" = "),
            "{manifest}"
        );
        assert!(!manifest.contains("backend/Cargo.toml"), "{manifest}");
        assert!(!manifest.contains("docs/adr"), "{manifest}");
        Ok(())
    }
}

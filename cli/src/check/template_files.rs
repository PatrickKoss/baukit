use std::{collections::BTreeMap, fs, io, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{CheckContext, Finding, Rule};
use crate::generate::render_template_owned;

const NAME: &str = "template-files";
/// Written by `baukit new`, rewritten by `baukit check template-files --update`.
pub const MANIFEST_PATH: &str = ".baukit/template-files.toml";
const MANIFEST_HEADER: &str = "\
# Files the baukit template owns, with the SHA-256 of their rendered content.
# `baukit check template-files` fails when one of them changes. Put product additions in
# scripts/quality-gate.local.sh instead of editing these files. After re-rendering the
# template for a baukit upgrade, run `baukit check template-files --update`.
";

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    files: BTreeMap<String, String>,
}

/// Lower-case hex SHA-256 of `contents`.
pub fn sha256_hex(contents: &[u8]) -> String {
    Sha256::digest(contents)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Renders the manifest for `files`, a map from product-relative path to file content hash.
///
/// # Errors
///
/// Returns an error when the map cannot be serialized as TOML.
pub fn render_manifest(files: BTreeMap<String, String>) -> Result<String> {
    let body = toml::to_string(&Manifest { files })?;
    Ok(format!("{MANIFEST_HEADER}\n{body}"))
}

pub struct TemplateFiles;

impl Rule for TemplateFiles {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        let manifest_path = context.root.join(MANIFEST_PATH);
        let source = match fs::read_to_string(&manifest_path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(vec![Finding::note(
                    NAME,
                    MANIFEST_PATH,
                    "not configured; `baukit new` writes this manifest for generated products",
                )]);
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", manifest_path.display()));
            }
        };
        let manifest: Manifest = toml::from_str(&source)
            .with_context(|| format!("invalid {}", manifest_path.display()))?;
        if context.options.update {
            let owned = render_template_owned(context.root)?;
            return update(context.root, &manifest, &owned);
        }
        check(context.root, &manifest)
    }
}

fn current_hash(root: &Path, path: &str) -> Result<Option<String>> {
    match fs::read(root.join(path)) {
        Ok(contents) => Ok(Some(sha256_hex(&contents))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {path}")),
    }
}

fn check(root: &Path, manifest: &Manifest) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for (path, expected) in &manifest.files {
        let message = match current_hash(root, path)? {
            None => "template-owned file is missing; restore it from the template",
            Some(actual) if &actual != expected => {
                "template-owned file differs from the template; move the change into scripts/quality-gate.local.sh and restore the file"
            }
            Some(_) => continue,
        };
        findings.push(Finding::error(NAME, path.as_str(), message));
    }
    Ok(findings)
}

/// Rewrites every hash from the current file, drops entries whose file is gone, and adds the
/// `owned` files the manifest lacks when the product holds them unchanged.
fn update(
    root: &Path,
    manifest: &Manifest,
    owned: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<Finding>> {
    let mut files = BTreeMap::new();
    for path in manifest.files.keys() {
        if let Some(hash) = current_hash(root, path)? {
            files.insert(path.clone(), hash);
        }
    }
    let mut findings = Vec::new();
    for (path, rendered) in owned
        .iter()
        .filter(|(path, _)| !manifest.files.contains_key(*path))
    {
        let Some(actual) = current_hash(root, path)? else {
            continue;
        };
        if actual == sha256_hex(rendered) {
            files.insert(path.clone(), actual);
        } else {
            findings.push(Finding::note(
                NAME,
                path.as_str(),
                "template-owned but diverged, not tracked",
            ));
        }
    }
    let target = root.join(MANIFEST_PATH);
    fs::write(&target, render_manifest(files)?)
        .with_context(|| format!("failed to write {}", target.display()))?;
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_product(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("scripts"))?;
        fs::create_dir_all(root.join(".baukit"))?;
        fs::write(root.join("scripts/gate.sh"), "echo gate\n")?;
        fs::write(root.join("scripts/openapi.sh"), "echo openapi\n")?;
        let files = BTreeMap::from([
            ("scripts/gate.sh".to_owned(), sha256_hex(b"echo gate\n")),
            (
                "scripts/openapi.sh".to_owned(),
                sha256_hex(b"echo openapi\n"),
            ),
        ]);
        fs::write(root.join(MANIFEST_PATH), render_manifest(files)?)?;
        Ok(())
    }

    fn manifest(root: &Path) -> Result<Manifest> {
        Ok(toml::from_str(&fs::read_to_string(
            root.join(MANIFEST_PATH),
        )?)?)
    }

    fn messages(root: &Path) -> Result<Vec<String>> {
        Ok(check(root, &manifest(root)?)?
            .iter()
            .map(ToString::to_string)
            .collect())
    }

    #[test]
    fn unchanged_files_pass() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_product(root.path())?;
        assert!(messages(root.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn edited_and_missing_files_are_named() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_product(root.path())?;
        fs::write(root.path().join("scripts/gate.sh"), "echo forked\n")?;
        fs::remove_file(root.path().join("scripts/openapi.sh"))?;
        assert_eq!(
            messages(root.path())?,
            vec![
                "template-files: scripts/gate.sh: template-owned file differs from the template; move the change into scripts/quality-gate.local.sh and restore the file",
                "template-files: scripts/openapi.sh: template-owned file is missing; restore it from the template",
            ]
        );
        Ok(())
    }

    fn owned(entries: &[(&str, &str)]) -> BTreeMap<String, Vec<u8>> {
        entries
            .iter()
            .map(|(path, contents)| ((*path).to_owned(), contents.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn update_rewrites_hashes_and_drops_removed_files() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_product(root.path())?;
        fs::write(root.path().join("scripts/gate.sh"), "echo upgraded\n")?;
        fs::remove_file(root.path().join("scripts/openapi.sh"))?;
        let findings = update(root.path(), &manifest(root.path())?, &BTreeMap::new())?;
        assert!(findings.is_empty());
        assert!(messages(root.path())?.is_empty());
        assert_eq!(
            manifest(root.path())?.files.keys().collect::<Vec<_>>(),
            vec!["scripts/gate.sh"]
        );
        Ok(())
    }

    #[test]
    fn update_adds_identical_new_template_files() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_product(root.path())?;
        fs::write(root.path().join("scripts/gate_locks.py"), "locks\n")?;
        let owned = owned(&[("scripts/gate_locks.py", "locks\n")]);
        let findings = update(root.path(), &manifest(root.path())?, &owned)?;
        assert!(findings.is_empty());
        assert_eq!(
            manifest(root.path())?.files.get("scripts/gate_locks.py"),
            Some(&sha256_hex(b"locks\n"))
        );
        Ok(())
    }

    #[test]
    fn update_reports_diverged_new_template_files_without_tracking_them() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_product(root.path())?;
        fs::write(root.path().join("scripts/gate_locks.py"), "forked\n")?;
        let owned = owned(&[("scripts/gate_locks.py", "locks\n")]);
        let findings = update(root.path(), &manifest(root.path())?, &owned)?;
        assert_eq!(
            findings.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec![
                "template-files: scripts/gate_locks.py: note: template-owned but diverged, not tracked"
            ]
        );
        assert!(
            !manifest(root.path())?
                .files
                .contains_key("scripts/gate_locks.py")
        );
        Ok(())
    }

    #[test]
    fn update_ignores_new_template_files_the_product_lacks() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_product(root.path())?;
        let owned = owned(&[("scripts/gate_locks.py", "locks\n")]);
        let findings = update(root.path(), &manifest(root.path())?, &owned)?;
        assert!(findings.is_empty());
        assert!(
            !manifest(root.path())?
                .files
                .contains_key("scripts/gate_locks.py")
        );
        Ok(())
    }

    #[test]
    fn update_keeps_tracked_rows_out_of_the_divergence_report() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_product(root.path())?;
        fs::write(root.path().join("scripts/gate.sh"), "echo upgraded\n")?;
        let owned = owned(&[("scripts/gate.sh", "echo template\n")]);
        let findings = update(root.path(), &manifest(root.path())?, &owned)?;
        assert!(findings.is_empty());
        assert_eq!(
            manifest(root.path())?.files.get("scripts/gate.sh"),
            Some(&sha256_hex(b"echo upgraded\n"))
        );
        Ok(())
    }

    #[test]
    fn update_restores_rows_dropped_from_a_generated_product() -> Result<()> {
        use crate::{Component, Components, NewOptions, QualityProfile, generate_new};

        let parent = tempfile::tempdir()?;
        let root = generate_new(&NewOptions {
            name: "rows-product".to_owned(),
            directory: parent.path().to_owned(),
            components: Components::from([Component::Backend]),
            auth: None,
            force: false,
            into_existing: false,
            resolve_lockfiles: false,
            baukit_path: None,
            port_offset: 0,
            quality: QualityProfile::Strict,
        })?;
        let full = manifest(&root)?;
        let mut trimmed = full.files.clone();
        assert!(trimmed.remove("scripts/gate_locks.py").is_some());
        fs::write(root.join(MANIFEST_PATH), render_manifest(trimmed)?)?;
        let owned = render_template_owned(&root)?;
        let findings = update(&root, &manifest(&root)?, &owned)?;
        assert!(findings.is_empty(), "{findings:?}");
        assert_eq!(manifest(&root)?.files, full.files);
        Ok(())
    }
}

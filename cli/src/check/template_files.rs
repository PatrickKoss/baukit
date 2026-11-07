use std::{collections::BTreeMap, fs, io, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{CheckContext, Finding, Rule};

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
            update(context.root, &manifest)?;
            return Ok(Vec::new());
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

/// Rewrites every hash from the current file and drops entries whose file is gone.
fn update(root: &Path, manifest: &Manifest) -> Result<()> {
    let mut files = BTreeMap::new();
    for path in manifest.files.keys() {
        if let Some(hash) = current_hash(root, path)? {
            files.insert(path.clone(), hash);
        }
    }
    let target = root.join(MANIFEST_PATH);
    fs::write(&target, render_manifest(files)?)
        .with_context(|| format!("failed to write {}", target.display()))
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

    #[test]
    fn update_rewrites_hashes_and_drops_removed_files() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_product(root.path())?;
        fs::write(root.path().join("scripts/gate.sh"), "echo upgraded\n")?;
        fs::remove_file(root.path().join("scripts/openapi.sh"))?;
        update(root.path(), &manifest(root.path())?)?;
        assert!(messages(root.path())?.is_empty());
        assert_eq!(
            manifest(root.path())?.files.keys().collect::<Vec<_>>(),
            vec!["scripts/gate.sh"]
        );
        Ok(())
    }
}

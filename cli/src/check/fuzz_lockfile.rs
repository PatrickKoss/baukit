use std::{fs, io, path::Path, process::Command};

use anyhow::{Context, Result, bail};

use super::{CheckContext, Finding, Rule};

const NAME: &str = "fuzz-lockfile";
const FUZZ_MANIFEST: &str = "backend/fuzz/Cargo.toml";
const FUZZ_LOCKFILE: &str = "backend/fuzz/Cargo.lock";

/// Fails when a strict product commits `backend/fuzz/Cargo.lock`. The gate's fuzz step copies
/// `backend/Cargo.lock` over it, so a committed copy drifts from the backend's pins.
pub struct FuzzLockfile;

impl Rule for FuzzLockfile {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        let root = context.root;
        if !strict_product(root)? || !root.join(FUZZ_MANIFEST).is_file() || !tracked(root)? {
            return Ok(Vec::new());
        }
        Ok(vec![Finding::error(
            NAME,
            FUZZ_LOCKFILE,
            format!(
                "tracked by git; the fuzz step copies backend/Cargo.lock over it, so run \
                 `git rm --cached {FUZZ_LOCKFILE}` and keep it ignored"
            ),
        )])
    }
}

fn strict_product(root: &Path) -> Result<bool> {
    let path = root.join("baukit.toml");
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let manifest: toml::Table =
        toml::from_str(&source).with_context(|| format!("invalid {}", path.display()))?;
    Ok(manifest
        .get("quality")
        .and_then(|quality| quality.get("profile"))
        .and_then(toml::Value::as_str)
        == Some("strict"))
}

fn tracked(root: &Path) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--", FUZZ_LOCKFILE])
        .output()
        .context("Git is required to check whether backend/fuzz/Cargo.lock is tracked")?;
    if !output.status.success() {
        if root.join(".git").exists() {
            bail!(
                "git ls-files failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return Ok(false);
    }
    Ok(!output.stdout.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{CheckConfig, CheckOptions};

    fn git(root: &Path, arguments: &[&str]) -> Result<()> {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .status()?;
        anyhow::ensure!(status.success(), "git {arguments:?} failed");
        Ok(())
    }

    fn product(profile: &str, track: bool) -> Result<tempfile::TempDir> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("backend/fuzz"))?;
        fs::write(
            root.path().join("baukit.toml"),
            format!("[quality]\nprofile = \"{profile}\"\n"),
        )?;
        fs::write(root.path().join(FUZZ_MANIFEST), "[package]\n")?;
        fs::write(root.path().join(FUZZ_LOCKFILE), "version = 4\n")?;
        git(root.path(), &["init", "-q"])?;
        if track {
            git(root.path(), &["add", FUZZ_LOCKFILE])?;
        }
        Ok(root)
    }

    fn findings(root: &Path) -> Result<Vec<Finding>> {
        let config = CheckConfig::default();
        FuzzLockfile.run(&CheckContext {
            root,
            config: &config,
            options: CheckOptions::default(),
        })
    }

    #[test]
    fn tracked_fuzz_lockfile_fails_in_a_strict_product() -> Result<()> {
        let root = product("strict", true)?;
        let found = findings(root.path())?;
        assert_eq!(found.len(), 1);
        assert!(
            found[0]
                .to_string()
                .contains("git rm --cached backend/fuzz/Cargo.lock")
        );
        Ok(())
    }

    #[test]
    fn untracked_fuzz_lockfile_passes() -> Result<()> {
        let root = product("strict", false)?;
        assert!(findings(root.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn standard_products_and_other_repositories_are_skipped() -> Result<()> {
        let standard = product("standard", true)?;
        assert!(findings(standard.path())?.is_empty());
        let plain = tempfile::tempdir()?;
        assert!(findings(plain.path())?.is_empty());
        Ok(())
    }
}

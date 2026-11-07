//! Regenerates the golden trees in `tests/snapshots/`. Run with `cargo run --example bless_snapshots`.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use baukit_cli::{NewOptions, generate_new};
use sha2::{Digest, Sha256};

#[path = "../tests/support/snapshot_flavors.rs"]
mod snapshot_flavors;

use snapshot_flavors::snapshot_flavors;

fn read_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut tree = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            if is_python_cache_artifact(&path) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root)?.to_path_buf();
                tree.insert(
                    relative,
                    fs::read(&path).with_context(|| format!("read {}", path.display()))?,
                );
            }
        }
    }
    Ok(tree)
}

fn is_python_cache_artifact(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__pycache__")
        || path.extension().is_some_and(|extension| extension == "pyc")
}

fn render(tree: &BTreeMap<PathBuf, Vec<u8>>) -> String {
    let mut out = String::new();
    for (path, contents) in tree {
        out.push_str(&format!("{}  {}\n", sha256_hex(contents), path.display()));
    }
    out
}

fn sha256_hex(contents: &[u8]) -> String {
    Sha256::digest(contents)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bless(name: &str, options: &NewOptions) -> Result<PathBuf> {
    let root = generate_new(options)?;
    let snapshot = render(&read_tree(&root)?);
    let target = snapshot_path(name);
    fs::write(&target, snapshot).with_context(|| format!("write {}", target.display()))?;
    Ok(target)
}

#[expect(
    clippy::print_stdout,
    reason = "developer tool output: names each snapshot file it rewrote"
)]
fn main() -> Result<()> {
    let parent = tempfile::tempdir()?;
    for (name, options) in snapshot_flavors(parent.path()) {
        let target = bless(name, &options)?;
        println!("blessed {}", target.display());
        fs::remove_dir_all(parent.path().join(&options.name))?;
    }
    Ok(())
}

fn snapshot_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.tree"))
}

#[test]
fn snapshot_path_uses_the_cli_manifest_directory() {
    let target = snapshot_path("backend");
    assert!(target.is_absolute());
    assert_eq!(
        target.parent(),
        Some(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/snapshots")
                .as_path()
        )
    );
}

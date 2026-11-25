//! Regenerates the golden trees in `tests/snapshots/`. Run with `cargo run --example bless_snapshots`.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use baukit_cli::{NewOptions, generate_new};

#[path = "../tests/support/snapshot_flavors.rs"]
mod snapshot_flavors;

#[path = "../tests/support/snapshot_tree.rs"]
mod snapshot_tree;

#[path = "../tests/support/snapshot_wiring.rs"]
mod snapshot_wiring;

use snapshot_flavors::snapshot_flavors;
use snapshot_tree::{read_tree, render_hash_snapshot};
use snapshot_wiring::{render_wiring_snapshot, wiring_snapshot_path};

fn bless(name: &str, options: &NewOptions) -> Result<PathBuf> {
    let root = generate_new(options)?;
    let snapshot = render_hash_snapshot(&read_tree(&root)?);
    let target = snapshot_path(name);
    fs::write(&target, snapshot).with_context(|| format!("write {}", target.display()))?;
    if let Some(wiring) = render_wiring_snapshot(&root)? {
        let path = wiring_snapshot_path(name);
        fs::write(&path, wiring).with_context(|| format!("write {}", path.display()))?;
    }
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

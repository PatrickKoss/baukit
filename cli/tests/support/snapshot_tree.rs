//! Reads a generated tree and renders the golden `tests/snapshots/*.tree` format.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use sha2::{Digest, Sha256};

const SYMLINK_MARKER: &[u8] = b"\0symlink\0";

/// Maps each file to its bytes. A symlink maps to a marker plus its target, so the snapshot records
/// the link instead of the file it points at.
pub fn read_tree(root: &Path) -> anyhow::Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut tree = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
            let path = entry?.path();
            if is_python_cache_artifact(&path) {
                continue;
            }
            let relative = path.strip_prefix(root)?.to_path_buf();
            if path.symlink_metadata()?.file_type().is_symlink() {
                let mut contents = SYMLINK_MARKER.to_vec();
                contents.extend_from_slice(fs::read_link(&path)?.as_os_str().as_encoded_bytes());
                tree.insert(relative, contents);
            } else if path.is_dir() {
                stack.push(path);
            } else {
                let contents =
                    fs::read(&path).with_context(|| format!("read {}", path.display()))?;
                tree.insert(relative, contents);
            }
        }
    }
    Ok(tree)
}

/// Renders one `<sha256>  <path>` line per file and `-> <target>  <path>` per symlink.
pub fn render_hash_snapshot(tree: &BTreeMap<PathBuf, Vec<u8>>) -> String {
    let mut snapshot = String::new();
    for (path, contents) in tree {
        let fingerprint = match symlink_target(contents) {
            Some(target) => format!("-> {}", String::from_utf8_lossy(target)),
            None => sha256_hex(contents),
        };
        snapshot.push_str(&format!("{fingerprint}  {}\n", path.display()));
    }
    snapshot
}

/// Returns the link target when `contents` came from a symlink in [`read_tree`].
pub fn symlink_target(contents: &[u8]) -> Option<&[u8]> {
    contents.strip_prefix(SYMLINK_MARKER)
}

pub fn sha256_hex(contents: &[u8]) -> String {
    Sha256::digest(contents)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_python_cache_artifact(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__pycache__")
        || path.extension().is_some_and(|extension| extension == "pyc")
}

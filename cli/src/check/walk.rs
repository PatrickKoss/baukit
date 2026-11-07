use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "vendor",
    ".generated-fixture",
];

/// Lists every regular file under `root` that `keep` accepts, sorted, without following
/// symlinks and without entering build, dependency or VCS directories.
pub fn files(root: &Path, keep: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in read_dir(&directory)? {
            let entry = entry.with_context(|| format!("failed to list {}", directory.display()))?;
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_dir() && !is_skipped(&path) {
                pending.push(path);
            } else if file_type.is_file() && keep(&path) {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn read_dir(directory: &Path) -> Result<fs::ReadDir> {
    fs::read_dir(directory).with_context(|| format!("failed to list {}", directory.display()))
}

fn is_skipped(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| SKIPPED_DIRECTORIES.contains(&name))
}

/// Reads a text file; a file that is not UTF-8 yields `None`.
pub fn read_text(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

pub fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_build_and_dependency_directories() -> Result<()> {
        let root = tempfile::tempdir()?;
        for directory in ["src", "target/debug", "node_modules/x", "vendor/y"] {
            fs::create_dir_all(root.path().join(directory))?;
            fs::write(root.path().join(directory).join("a.rs"), "")?;
        }
        let listed = files(root.path(), |_| true)?;
        assert_eq!(listed, vec![root.path().join("src/a.rs")]);
        Ok(())
    }
}

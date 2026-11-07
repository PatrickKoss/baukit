use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use ignore::WalkBuilder;

const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "vendor",
    ".generated-fixture",
];

/// Lists every regular file under `root` that `keep` accepts, sorted. It honours `.gitignore`
/// files (also outside a git repository), `.git/info/exclude` and the global excludes, does not
/// follow symlinks, and never enters build, dependency or VCS directories.
pub fn files(root: &Path, keep: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>> {
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| !is_skipped(entry.path()))
        .build();
    let mut found = Vec::new();
    for entry in walker {
        let entry = entry.with_context(|| format!("failed to list {}", root.display()))?;
        let is_file = entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file());
        if is_file && keep(entry.path()) {
            found.push(entry.into_path());
        }
    }
    found.sort();
    Ok(found)
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

    #[test]
    fn honours_gitignore_for_directories_outside_the_fixed_list() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join(".gitignore"), "target-msrv/\n")?;
        fs::create_dir_all(root.path().join("deploy/.local-state/snap"))?;
        fs::write(root.path().join("deploy/.gitignore"), ".local-state/\n")?;
        for path in [
            "target-msrv/out/lib.rs",
            "deploy/.local-state/snap/a.rs",
            "src/.hidden.rs",
        ] {
            let file = root.path().join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(file, "")?;
        }
        let listed = files(root.path(), |path| {
            path.extension().is_some_and(|ext| ext == "rs")
        })?;
        assert_eq!(listed, vec![root.path().join("src/.hidden.rs")]);
        Ok(())
    }
}

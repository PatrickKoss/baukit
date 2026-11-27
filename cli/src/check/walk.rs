use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
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

/// Lists every regular file under `root` that `keep` accepts, sorted.
///
/// Inside a git work tree it lists the tracked files plus the untracked ones that are not
/// ignored, so a force-added file under an ignored directory is still checked. Elsewhere it
/// walks the tree honouring `.gitignore` files. Either way it skips symlinks and build, VCS,
/// vendored and fixture directories.
pub fn files(root: &Path, keep: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>> {
    let listed = match git_files(root) {
        Some(listed) => listed,
        None => walked_files(root)?,
    };
    let mut found: Vec<PathBuf> = listed
        .into_iter()
        .filter(|path| is_regular_file(path) && keep(path))
        .collect();
    found.sort();
    found.dedup();
    Ok(found)
}

fn git_files(root: &Path) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(
        text.split('\0')
            .filter(|relative| !relative.is_empty())
            .map(Path::new)
            .filter(|relative| !relative.components().any(is_skipped_component))
            .map(|relative| root.join(relative))
            .collect(),
    )
}

fn walked_files(root: &Path) -> Result<Vec<PathBuf>> {
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
        found.push(entry.into_path());
    }
    Ok(found)
}

fn is_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

fn is_skipped_component(component: Component<'_>) -> bool {
    component
        .as_os_str()
        .to_str()
        .is_some_and(|name| SKIPPED_DIRECTORIES.contains(&name))
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
        for directory in [
            "src",
            "target/debug",
            "node_modules/x",
            "vendor/y",
            "a/b/vendor",
        ] {
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

    #[test]
    fn lists_force_added_ignored_files_but_not_vendor_in_a_git_repository() -> Result<()> {
        let root = tempfile::tempdir()?;
        let git = |args: &[&str]| -> Result<()> {
            let status = Command::new("git")
                .arg("-C")
                .arg(root.path())
                .args(args)
                .status()?;
            anyhow::ensure!(status.success(), "git {args:?} failed");
            Ok(())
        };
        git(&["init", "-q"])?;
        fs::write(root.path().join(".gitignore"), "ignored/\n")?;
        for path in [
            "ignored/forced.rs",
            "ignored/skipped.rs",
            "deep/vendor/v.rs",
            "src/new.rs",
        ] {
            let file = root.path().join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(file, "")?;
        }
        git(&["add", "-f", "ignored/forced.rs", "deep/vendor/v.rs"])?;
        let listed = files(root.path(), |path| {
            path.extension().is_some_and(|ext| ext == "rs")
        })?;
        assert_eq!(
            listed,
            vec![
                root.path().join("ignored/forced.rs"),
                root.path().join("src/new.rs"),
            ]
        );
        Ok(())
    }
}

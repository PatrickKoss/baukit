use std::{fs, io::ErrorKind, path::Path};

use anyhow::{Context, Result};

/// Visits every regular file under `directory`, skipping dependency and build
/// output directories. Entries that other processes delete during the walk are
/// treated as gone.
pub(super) fn walk_product_tree(
    directory: &Path,
    visit: &mut dyn FnMut(&Path) -> Result<()>,
) -> Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not list {}", directory.display()));
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not list {}", directory.display()));
            }
        };
        let path = entry.path();
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("could not inspect {}", path.display()));
            }
        };
        if kind.is_dir() {
            if !skipped_directory(&path) {
                walk_product_tree(&path, visit)?;
            }
        } else if kind.is_file() {
            visit(&path)?;
        }
    }
    Ok(())
}

/// Reads a file found by a walk; `None` when it vanished since it was listed.
pub(super) fn read_walked(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

pub(super) fn read_text(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))
}

pub(super) fn skipped_directory(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    excluded_directory(Some(name)) || (name == "build" && is_native_build_output(path))
}

fn is_native_build_output(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    parent.join("build.gradle").is_file()
        || parent.join("build.gradle.kts").is_file()
        || parent
            .file_name()
            .is_some_and(|name| name == "android" || name == "ios")
}

fn excluded_directory(name: Option<&str>) -> bool {
    let Some(name) = name else {
        return false;
    };
    name.starts_with("maestro-debug")
        || matches!(
            name,
            "target"
                | "node_modules"
                | ".git"
                | "__pycache__"
                | ".generated-fixture"
                | ".playwright-browsers"
                | "dist"
                | "coverage"
                | ".expo"
                | ".qa"
                | ".quality-gate"
                | ".ci-gate"
                | ".gradle"
                | ".cxx"
                | ".turbo"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor_layout::product_files;

    #[test]
    fn walks_skip_generated_build_output() -> Result<()> {
        let root = tempfile::tempdir()?;
        for directory in [
            "mobile/android/build",
            "mobile/android/app/build",
            "mobile/.expo",
            "mobile/.qa",
            "mobile/maestro-debug-1",
            ".quality-gate",
            "scripts/build",
        ] {
            fs::create_dir_all(root.path().join(directory))?;
            fs::write(root.path().join(directory).join("file.sh"), "")?;
        }
        fs::write(root.path().join("mobile/android/app/build.gradle"), "")?;
        assert_eq!(
            product_files(root.path(), "sh")?,
            vec![root.path().join("scripts/build/file.sh")]
        );
        Ok(())
    }

    #[test]
    fn walks_treat_entries_removed_mid_walk_as_gone() -> Result<()> {
        let root = tempfile::tempdir()?;
        for name in ["a", "b", "c"] {
            fs::create_dir(root.path().join(name))?;
            fs::write(root.path().join(name).join("inner.txt"), "x")?;
            fs::write(root.path().join(format!("{name}.txt")), "x")?;
        }
        let mut visited = 0;
        walk_product_tree(root.path(), &mut |path| {
            visited += 1;
            for name in ["a", "b", "c"] {
                let _ = fs::remove_dir_all(root.path().join(name));
                let _ = fs::remove_file(root.path().join(format!("{name}.txt")));
            }
            assert_eq!(read_walked(path)?, None);
            Ok(())
        })?;
        assert!(visited >= 1);
        assert_eq!(read_walked(&root.path().join("b.txt"))?, None);
        Ok(())
    }

    #[test]
    fn io_errors_name_the_path() {
        let missing = Path::new("/nonexistent/baukit/doctor/file.toml");
        let error = read_text(missing).err().map(|error| format!("{error:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains("/nonexistent/baukit/doctor/file.toml")),
            "{error:?}"
        );
        let mut file_as_directory =
            tempfile::NamedTempFile::new().map(|file| file.into_temp_path());
        let Ok(path) = file_as_directory.as_mut() else {
            return;
        };
        let error = walk_product_tree(path, &mut |_| Ok(()))
            .err()
            .map(|error| format!("{error:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains(&path.display().to_string())),
            "{error:?}"
        );
    }
}

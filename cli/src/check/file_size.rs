use std::path::Path;

use anyhow::Result;

use super::{CONFIG_FILE, CheckContext, Finding, Rule, relative, walk};

const NAME: &str = "file-size";
const RUST_EXTENSIONS: &[&str] = &["rs"];
const TS_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts"];
const TEST_DIRECTORIES: &[&str] = &["tests", "__tests__"];
const GENERATED_DIRECTORIES: &[&str] = &["generated", "__generated__"];
const GENERATED_MARKERS: &[&str] = &["@generated", "do not edit"];
const GENERATED_MARKER_LINES: usize = 5;

pub struct FileSize;

impl Rule for FileSize {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        let settings = &context.config.file_size;
        let mut findings = Vec::new();
        for path in walk::files(context.root, |path| limit_kind(path).is_some())? {
            let display = relative(context.root, &path);
            let Some(kind) = limit_kind(&path) else {
                continue;
            };
            if is_excluded_path(&display) {
                continue;
            }
            let Some(source) = walk::read_text(&path)? else {
                continue;
            };
            if is_generated(&source) {
                continue;
            }
            let limit = match kind {
                SourceKind::Rust => settings.rust_max_lines,
                SourceKind::TypeScript => settings.ts_max_lines,
            };
            let lines = source.lines().count();
            findings.extend(check_file(
                &display,
                lines,
                limit,
                settings.allowlist.get(&display),
            ));
        }
        for listed in settings.allowlist.keys() {
            if !context.root.join(listed).is_file() {
                findings.push(Finding::error(
                    NAME,
                    CONFIG_FILE,
                    format!("allowlist entry `{listed}` no longer exists; drop it"),
                ));
            }
        }
        Ok(findings)
    }
}

#[derive(Debug, Clone, Copy)]
enum SourceKind {
    Rust,
    TypeScript,
}

fn limit_kind(path: &Path) -> Option<SourceKind> {
    let extension = path.extension()?.to_str()?;
    if RUST_EXTENSIONS.contains(&extension) {
        Some(SourceKind::Rust)
    } else if TS_EXTENSIONS.contains(&extension) {
        Some(SourceKind::TypeScript)
    } else {
        None
    }
}

fn is_excluded_path(display: &str) -> bool {
    let mut components = display.split('/').collect::<Vec<_>>();
    let Some(name) = components.pop() else {
        return true;
    };
    let in_excluded_directory = components.iter().any(|directory| {
        TEST_DIRECTORIES.contains(directory) || GENERATED_DIRECTORIES.contains(directory)
    });
    in_excluded_directory
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.contains(".generated.")
        || name.ends_with(".d.ts")
        || name == "tests.rs"
        || name.ends_with("_tests.rs")
}

fn is_generated(source: &str) -> bool {
    source.lines().take(GENERATED_MARKER_LINES).any(|line| {
        let line = line.to_ascii_lowercase();
        GENERATED_MARKERS.iter().any(|marker| line.contains(marker))
    })
}

fn check_file(
    display: &str,
    lines: usize,
    limit: usize,
    recorded: Option<&usize>,
) -> Option<Finding> {
    let message = match recorded {
        None if lines > limit => {
            format!("{lines} lines exceeds the limit of {limit}; split the file")
        }
        Some(_) if lines <= limit => format!(
            "{lines} lines is within the limit of {limit}; drop its {CONFIG_FILE} allowlist entry"
        ),
        Some(&recorded) if lines > recorded => {
            format!("{lines} lines grew past its allowlisted {recorded}; split the file instead")
        }
        Some(&recorded) if lines < recorded => {
            return Some(Finding::note(
                NAME,
                display,
                format!(
                    "shrank to {lines} lines; lower its allowlist entry from {recorded} to {lines}"
                ),
            ));
        }
        _ => return None,
    };
    Some(Finding::error(NAME, display, message))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::check::{CheckConfig, CheckOptions};

    fn run(root: &Path) -> Result<Vec<String>> {
        let config = CheckConfig::load(root)?;
        let context = CheckContext {
            root,
            config: &config,
            options: CheckOptions::default(),
        };
        Ok(FileSize
            .run(&context)?
            .iter()
            .map(ToString::to_string)
            .collect())
    }

    fn write_lines(root: &Path, path: &str, count: usize) -> Result<()> {
        let file = root.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, "x\n".repeat(count))?;
        Ok(())
    }

    #[test]
    fn clean_tree_and_exclusions_pass() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_lines(root.path(), "src/lib.rs", 1000)?;
        write_lines(root.path(), "web/app.tsx", 500)?;
        write_lines(root.path(), "src/tests/big.rs", 2000)?;
        write_lines(root.path(), "src/doctor_tests.rs", 2000)?;
        write_lines(root.path(), "web/app.test.ts", 2000)?;
        write_lines(root.path(), "web/api.d.ts", 2000)?;
        write_lines(root.path(), "target/x.rs", 2000)?;
        assert!(run(root.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn oversized_files_fail() -> Result<()> {
        let root = tempfile::tempdir()?;
        write_lines(root.path(), "src/lib.rs", 1001)?;
        write_lines(root.path(), "web/app.ts", 501)?;
        assert_eq!(
            run(root.path())?,
            vec![
                "file-size: src/lib.rs: 1001 lines exceeds the limit of 1000; split the file",
                "file-size: web/app.ts: 501 lines exceeds the limit of 500; split the file",
            ]
        );
        Ok(())
    }

    #[test]
    fn lowered_limit_applies() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join(CONFIG_FILE),
            "[file_size]\nts-max-lines = 10\n",
        )?;
        write_lines(root.path(), "a.ts", 11)?;
        assert_eq!(run(root.path())?.len(), 1);
        Ok(())
    }

    #[test]
    fn allowlist_only_shrinks() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join(CONFIG_FILE),
            "[file_size.allowlist]\n\"grew.rs\" = 1100\n\"same.rs\" = 1100\n\"shrank.rs\" = 1100\n\"small.rs\" = 1100\n\"gone.rs\" = 1100\n",
        )?;
        write_lines(root.path(), "grew.rs", 1101)?;
        write_lines(root.path(), "same.rs", 1100)?;
        write_lines(root.path(), "shrank.rs", 1050)?;
        write_lines(root.path(), "small.rs", 900)?;
        assert_eq!(
            run(root.path())?,
            vec![
                "file-size: grew.rs: 1101 lines grew past its allowlisted 1100; split the file instead",
                "file-size: shrank.rs: note: shrank to 1050 lines; lower its allowlist entry from 1100 to 1050",
                "file-size: small.rs: 900 lines is within the limit of 1000; drop its baukit-check.toml allowlist entry",
                "file-size: baukit-check.toml: allowlist entry `gone.rs` no longer exists; drop it",
            ]
        );
        Ok(())
    }

    #[test]
    fn generated_marker_excludes_a_file() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join("schema.ts"),
            format!("// @generated by openapi-typescript\n{}", "x\n".repeat(600)),
        )?;
        assert!(run(root.path())?.is_empty());
        Ok(())
    }
}

use std::path::Path;

use anyhow::Result;

use crate::doctor_layout;

const APP_SCHEME_HELPER: &str = "mobile/src/app-scheme.ts";
const SUITE_SOURCE_DIRECTORIES: [&str; 2] = ["mobile/src", "mobile/app"];

/// Warns about suite links built on the plain scheme: the dev variant registers
/// `<scheme>-dev`, so a hardcoded `<scheme>://suite/...` return opens the plain app.
pub(crate) fn suite_scheme_notes(
    root: &Path,
    app: &str,
    successes: &mut Vec<String>,
) -> Result<()> {
    if !root.join(APP_SCHEME_HELPER).is_file() {
        return Ok(());
    }
    let literals = [
        format!("{app}://suite"),
        format!("{app}:///suite"),
        format!("createSuiteNativeIntentValidator('{app}')"),
        format!("createSuiteNativeIntentValidator(\"{app}\")"),
    ];
    for directory in SUITE_SOURCE_DIRECTORIES {
        doctor_layout::walk_product_tree(&root.join(directory), &mut |path| {
            if !is_production_script(path) {
                return Ok(());
            }
            let Some(bytes) = doctor_layout::read_walked(path)? else {
                return Ok(());
            };
            let source = String::from_utf8_lossy(&bytes);
            if let Some(literal) = literals.iter().find(|literal| source.contains(*literal)) {
                successes.push(format!(
                    "note: warning: `{}` builds a suite link from the fixed scheme (`{literal}`); take the scheme from `appScheme()` in `{APP_SCHEME_HELPER}`, or the dev variant's suite returns on `{app}-dev://suite/...` open the plain app",
                    path.strip_prefix(root)?.display()
                ));
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn is_production_script(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let script = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"]
        .iter()
        .any(|extension| name.ends_with(extension));
    script && !name.contains(".test.") && !name.contains(".spec.")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn product(files: &[(&str, &str)]) -> Result<tempfile::TempDir> {
        let root = tempfile::tempdir()?;
        for (path, source) in files {
            let path = root.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, source)?;
        }
        Ok(root)
    }

    #[test]
    fn hardcoded_suite_schemes_are_reported() -> Result<()> {
        let root = product(&[
            (APP_SCHEME_HELPER, "export function appScheme() {}"),
            (
                "mobile/src/features/suite/runtime.tsx",
                "const origin = 'demo://suite';",
            ),
            (
                "mobile/app/+native-intent.ts",
                "const intent = createSuiteNativeIntentValidator('demo');",
            ),
            (
                "mobile/src/features/suite/runtime.test.tsx",
                "'demo://suite'",
            ),
            ("mobile/src/other.ts", "`${appScheme()}://suite`"),
        ])?;
        let mut notes = Vec::new();
        suite_scheme_notes(root.path(), "demo", &mut notes)?;
        notes.sort();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].starts_with("note: warning: `mobile/app/+native-intent.ts`"));
        assert!(notes[0].contains("createSuiteNativeIntentValidator('demo')"));
        assert!(notes[1].contains("`mobile/src/features/suite/runtime.tsx`"));
        assert!(notes[1].contains("demo-dev://suite/"));
        Ok(())
    }

    #[test]
    fn products_without_the_scheme_helper_get_no_note() -> Result<()> {
        let root = product(&[("mobile/src/runtime.ts", "'demo://suite'")])?;
        let mut notes = Vec::new();
        suite_scheme_notes(root.path(), "demo", &mut notes)?;
        assert!(notes.is_empty());
        Ok(())
    }
}

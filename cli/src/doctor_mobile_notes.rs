use std::path::Path;

use anyhow::Result;

use crate::{doctor_layout, ports::QaPorts};

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

const QA_HOOKS: &str = "mobile/scripts/qa/local.sh";

/// Warns about QA ports pinned to the fixed defaults of templates before 0.30.3,
/// which defeat the per-product defaults and collide between products on one host.
pub(crate) fn pinned_qa_port_notes(
    root: &Path,
    qa_ports: &QaPorts,
    successes: &mut Vec<String>,
) -> Result<()> {
    let path = root.join(QA_HOOKS);
    let Some(bytes) = doctor_layout::read_walked(&path)? else {
        return Ok(());
    };
    let legacy = [
        (15432, "postgres", qa_ports.postgres),
        (16379, "redis", qa_ports.redis),
        (18080, "api", qa_ports.api),
        (19090, "ops", qa_ports.ops),
        (18081, "keycloak", qa_ports.keycloak),
    ];
    for (index, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
        let code = shell_code(line);
        for (port, service, derived) in legacy {
            if port == derived || !contains_number(code, port) {
                continue;
            }
            let variable = assigned_variable(code).unwrap_or("a QA port");
            successes.push(format!(
                "note: warning: `{QA_HOOKS}:{}` pins `{variable}` to {port}, the fixed QA {service} port of templates before 0.30.3; it collides with other products on the host. Drop the assignment to use this product's default {derived}",
                index + 1
            ));
        }
    }
    Ok(())
}

fn shell_code(line: &str) -> &str {
    if line.trim_start().starts_with('#') {
        return "";
    }
    line.split_once(" #").map_or(line, |(code, _)| code)
}

fn contains_number(code: &str, number: u16) -> bool {
    code.split(|character: char| !character.is_ascii_digit())
        .any(|token| token.parse() == Ok(number))
}

fn assigned_variable(code: &str) -> Option<&str> {
    let (left, _) = code.split_once('=')?;
    let name = left
        .trim()
        .trim_start_matches("export ")
        .trim_start_matches("local ")
        .trim_start_matches("readonly ")
        .trim();
    let identifier = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    identifier.then_some(name)
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

    fn qa_ports() -> QaPorts {
        QaPorts {
            postgres: 15832,
            redis: 16779,
            api: 18480,
            ops: 19490,
            keycloak: 18081,
        }
    }

    #[test]
    fn pinned_legacy_qa_ports_are_reported_with_the_derived_default() -> Result<()> {
        let root = product(&[(
            QA_HOOKS,
            "# postgres_port=15432 is the old default\npostgres_port=15432\nexport api_port=\"${BAUKIT_QA_API_PORT:-18080}\"\nqa_reverse_ports=(19090)\nkeycloak_port=18081\nredis_port=163790\n",
        )])?;
        let mut notes = Vec::new();
        pinned_qa_port_notes(root.path(), &qa_ports(), &mut notes)?;
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes[0].starts_with(
            "note: warning: `mobile/scripts/qa/local.sh:2` pins `postgres_port` to 15432"
        ));
        assert!(notes[0].ends_with("this product's default 15832"));
        assert!(notes[1].contains(":3` pins `api_port` to 18080"));
        assert!(notes[1].ends_with("default 18480"));
        assert!(notes[2].contains(":4` pins `qa_reverse_ports` to 19090"));
        Ok(())
    }

    #[test]
    fn products_without_qa_hooks_get_no_port_note() -> Result<()> {
        let root = product(&[])?;
        let mut notes = Vec::new();
        pinned_qa_port_notes(root.path(), &qa_ports(), &mut notes)?;
        assert!(notes.is_empty());
        Ok(())
    }
}

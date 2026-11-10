use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{identity, ts_aliases::TsPathAliases};

fn screen_imports(source: &str) -> Vec<&str> {
    let code = identity::identity_code(source, false);
    let mut modules = Vec::new();
    for keyword in ["import ", "export "] {
        for (index, _) in code.match_indices(keyword) {
            let statement = &source[index + keyword.len()..];
            if statement.starts_with("type ")
                || (keyword == "export " && !statement.starts_with(['{', '*']))
            {
                continue;
            }
            let Some((bindings, tail)) = statement.split_once(" from ") else {
                continue;
            };
            if bindings.contains(';') {
                continue;
            }
            let tail = tail.trim_start();
            let Some(quote @ ('\'' | '"')) = tail.chars().next() else {
                continue;
            };
            let Some((module, _)) = tail[1..].split_once(quote) else {
                continue;
            };
            let rendered = bindings.split(['{', '}', ',']).any(|binding| {
                let Some(local) = binding.split_whitespace().last() else {
                    return false;
                };
                code.match_indices(&format!("<{local}"))
                    .any(|(index, matched)| {
                        code[index + matched.len()..].starts_with(|character: char| {
                            character.is_whitespace() || matches!(character, '>' | '/' | '.')
                        })
                    })
            });
            if keyword == "export " || rendered {
                modules.push(module);
            }
        }
    }
    modules
}

pub(super) fn screen_modules(root: &Path, screens: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    const MAX_IMPORT_DEPTH: usize = 16;
    let mut pending = screens
        .into_iter()
        .map(|path| (path, 0))
        .collect::<VecDeque<_>>();
    let mut modules = BTreeSet::new();
    let root = root.canonicalize()?;
    while let Some((path, depth)) = pending.pop_front() {
        let path = path.canonicalize()?;
        if !path.starts_with(&root) || !modules.insert(path.clone()) {
            continue;
        }
        let source = identity::uncommented(&fs::read_to_string(&path)?, false);
        if depth == MAX_IMPORT_DEPTH {
            continue;
        }
        for module in screen_imports(&source) {
            if let Some(imported) = TsPathAliases::nearest(&path)?
                .resolve(path.parent().context("screen has no parent")?, module)
            {
                pending.push_back((imported, depth + 1));
            }
        }
    }
    Ok(modules.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_imports_follow_rendered_bindings_and_reexports() {
        let source = "import { LoginScreen as Screen } from './screen'\nimport unused from './unused'\nimport type { Props } from './props';\nexport { default } from './exported';\nexport default function Route() { return <Screen />; }";
        assert_eq!(screen_imports(source), vec!["./screen", "./exported"]);
    }

    #[test]
    fn screen_imports_ignore_unused_imports_after_a_default_export() {
        let source =
            "export default function Route() { return null }\nimport Screen from './unused'\n";
        assert!(screen_imports(source).is_empty());
    }

    #[test]
    fn screen_imports_accept_multiline_jsx() {
        for separator in ["\n", "\r\n", "\t"] {
            let source = format!(
                "import Screen from './screen';\nexport default function Route() {{ return <Screen{separator}label=\"Sign in\" />; }}"
            );
            assert_eq!(screen_imports(&source), vec!["./screen"], "{source}");
        }
    }

    #[test]
    fn screen_modules_resolve_directory_components() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir(root.path().join("screen"))?;
        fs::write(
            root.path().join("route.tsx"),
            "import Screen from './screen'; export default function Route() { return <Screen />; }",
        )?;
        fs::write(
            root.path().join("screen/index.tsx"),
            "export default function Screen() { return null; }",
        )?;
        let modules = screen_modules(root.path(), vec![root.path().join("route.tsx")])?;
        assert_eq!(
            modules,
            vec![
                root.path().join("route.tsx"),
                root.path().join("screen/index.tsx")
            ]
        );
        Ok(())
    }

    #[test]
    fn screen_modules_bound_import_depth() -> Result<()> {
        let root = tempfile::tempdir()?;
        for depth in 0..=17 {
            fs::write(
                root.path().join(format!("{depth}.tsx")),
                format!("export {{ default }} from './{}';", depth + 1),
            )?;
        }
        let modules = screen_modules(root.path(), vec![root.path().join("0.tsx")])?;
        assert_eq!(modules.len(), 17);
        assert!(modules.iter().any(|path| path.ends_with("16.tsx")));
        assert!(!modules.iter().any(|path| path.ends_with("17.tsx")));
        Ok(())
    }

    fn alias_product(route_import: &str) -> Result<tempfile::TempDir> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("mobile/app/(auth)"))?;
        fs::create_dir_all(root.path().join("mobile/src/auth"))?;
        fs::write(
            root.path().join("mobile/tsconfig.json"),
            "{\n  // aliases\n  \"compilerOptions\": { \"paths\": { \"@/*\": [\"./src/*\"] } }\n}",
        )?;
        fs::write(
            root.path().join("mobile/app/(auth)/sign-in.tsx"),
            format!(
                "import {{ SignIn }} from '{route_import}'; export default function Route() {{ return <SignIn />; }}"
            ),
        )?;
        fs::write(
            root.path().join("mobile/src/auth/sign-in.tsx"),
            "export function SignIn() { return null; }",
        )?;
        Ok(root)
    }

    fn reached(root: &tempfile::TempDir) -> Result<bool> {
        let route = root.path().join("mobile/app/(auth)/sign-in.tsx");
        let target = root
            .path()
            .join("mobile/src/auth/sign-in.tsx")
            .canonicalize()?;
        Ok(screen_modules(root.path(), vec![route])?.contains(&target))
    }

    #[test]
    fn screen_modules_follow_path_alias_imports() -> Result<()> {
        assert!(reached(&alias_product("@/auth/sign-in")?)?);
        Ok(())
    }

    #[test]
    fn screen_modules_skip_alias_imports_without_a_target() -> Result<()> {
        assert!(!reached(&alias_product("@/auth/missing")?)?);
        Ok(())
    }

    #[test]
    fn screen_modules_follow_relative_imports_beside_aliases() -> Result<()> {
        assert!(reached(&alias_product("../../src/auth/sign-in")?)?);
        Ok(())
    }
}

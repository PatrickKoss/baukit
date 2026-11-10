use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::identity;

const TSCONFIG: &str = "tsconfig.json";

#[derive(Debug, Default)]
pub(crate) struct TsPathAliases {
    directory: PathBuf,
    rules: Vec<AliasRule>,
}

#[derive(Debug)]
struct AliasRule {
    prefix: String,
    suffix: Option<String>,
    targets: Vec<String>,
}

impl AliasRule {
    fn new(pattern: &str, targets: Vec<String>) -> Self {
        match pattern.split_once('*') {
            Some((prefix, suffix)) => Self {
                prefix: prefix.to_owned(),
                suffix: Some(suffix.to_owned()),
                targets,
            },
            None => Self {
                prefix: pattern.to_owned(),
                suffix: None,
                targets,
            },
        }
    }

    fn capture<'a>(&self, module: &'a str) -> Option<&'a str> {
        let rest = module.strip_prefix(self.prefix.as_str())?;
        match &self.suffix {
            Some(suffix) => rest.strip_suffix(suffix.as_str()),
            None => rest.is_empty().then_some(rest),
        }
    }
}

impl TsPathAliases {
    pub(crate) fn nearest(file: &Path) -> Result<Self> {
        for directory in file.ancestors().skip(1) {
            let config = directory.join(TSCONFIG);
            if config.is_file() {
                return Ok(Self::parse(directory, &fs::read_to_string(config)?));
            }
        }
        Ok(Self::default())
    }

    fn parse(directory: &Path, source: &str) -> Self {
        let json = without_trailing_commas(&identity::uncommented(source, false));
        let Ok(config) = serde_json::from_str::<serde_json::Value>(&json) else {
            return Self::default();
        };
        let options = config.get("compilerOptions");
        let base = options
            .and_then(|options| options.get("baseUrl"))
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| directory.to_path_buf(), |base| directory.join(base));
        let rules = options
            .and_then(|options| options.get("paths"))
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
            .map(|(pattern, targets)| {
                let targets = targets
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|target| target.as_str().map(str::to_owned))
                    .collect();
                AliasRule::new(pattern, targets)
            })
            .collect();
        Self {
            directory: base,
            rules,
        }
    }

    pub(crate) fn resolve(&self, importer_directory: &Path, module: &str) -> Option<PathBuf> {
        if module.starts_with('.') {
            return identity::module_path(importer_directory, module);
        }
        let rule = self
            .rules
            .iter()
            .filter(|rule| rule.capture(module).is_some())
            .max_by_key(|rule| rule.prefix.len())?;
        let captured = rule.capture(module)?;
        rule.targets.iter().find_map(|target| {
            let target = target.replacen('*', captured, 1);
            identity::module_path(&self.directory, &target)
        })
    }
}

fn without_trailing_commas(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '"' {
            result.push(character);
            let mut escaped = false;
            for inner in chars.by_ref() {
                result.push(inner);
                if inner == '"' && !escaped {
                    break;
                }
                escaped = inner == '\\' && !escaped;
            }
        } else if character == ','
            && chars
                .clone()
                .find(|next| !next.is_whitespace())
                .is_some_and(|next| matches!(next, '}' | ']'))
        {
            continue;
        } else {
            result.push(character);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliases(source: &str) -> TsPathAliases {
        TsPathAliases::parse(Path::new("/product/mobile"), source)
    }

    #[test]
    fn parse_accepts_comments_and_trailing_commas() {
        let parsed = aliases(
            "{\n  // alias\n  \"compilerOptions\": { \"paths\": { \"@/*\": [\"./src/*\",], }, },\n}",
        );
        assert_eq!(parsed.rules.len(), 1);
    }

    #[test]
    fn parse_ignores_unparseable_config() {
        assert!(aliases("not json").rules.is_empty());
    }

    #[test]
    fn longest_prefix_wins_and_bare_packages_stay_unresolved() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("src"))?;
        fs::create_dir_all(root.path().join("app"))?;
        fs::write(root.path().join("src/a.ts"), "")?;
        fs::write(root.path().join("app/a.tsx"), "")?;
        let parsed = TsPathAliases::parse(
            root.path(),
            r#"{"compilerOptions":{"paths":{"@/*":["./src/*"],"@/app/*":["./app/*"]}}}"#,
        );
        assert_eq!(
            parsed.resolve(root.path(), "@/app/a"),
            Some(root.path().join("app/a.tsx"))
        );
        assert_eq!(
            parsed.resolve(root.path(), "@/a"),
            Some(root.path().join("src/a.ts"))
        );
        assert_eq!(parsed.resolve(root.path(), "react"), None);
        Ok(())
    }
}

//! gitleaks allowlists that silence a pattern everywhere instead of one value or one file.

use toml::Value;

const ALLOWLIST_KEYS: &[&str] = &["allowlist", "allowlists"];
const REGEX_META: &[char] = &[
    '.', '^', '*', '+', '?', '(', ')', '[', ']', '{', '}', '|', '$',
];

/// `(line, message)` for every allowlist whose pattern regexes apply to the whole repository.
pub fn findings(source: &str) -> Vec<(usize, String)> {
    let Ok(document) = source.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut allowlists = Vec::new();
    let document = Value::Table(document);
    collect(&document, &mut allowlists);
    let mut found = Vec::new();
    for allowlist in allowlists {
        if is_scoped(allowlist) {
            continue;
        }
        for regex in regexes(allowlist).filter(|regex| !is_exact_value(regex)) {
            found.push((line_of(source, regex), message(regex)));
        }
    }
    found.sort();
    found
}

fn collect<'a>(value: &'a Value, found: &mut Vec<&'a toml::Table>) {
    match value {
        Value::Table(table) => {
            for (key, child) in table {
                if ALLOWLIST_KEYS.contains(&key.as_str()) {
                    allowlist_tables(child, found);
                } else {
                    collect(child, found);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect(item, found)),
        _ => {}
    }
}

fn allowlist_tables<'a>(value: &'a Value, found: &mut Vec<&'a toml::Table>) {
    match value {
        Value::Table(table) => found.push(table),
        Value::Array(items) => items.iter().for_each(|item| allowlist_tables(item, found)),
        _ => {}
    }
}

fn is_scoped(allowlist: &toml::Table) -> bool {
    ["paths", "commits"].iter().any(|key| {
        allowlist
            .get(*key)
            .and_then(Value::as_array)
            .is_some_and(|entries| !entries.is_empty())
    })
}

fn regexes(allowlist: &toml::Table) -> impl Iterator<Item = &str> {
    allowlist
        .get("regexes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// `^literal$`: anchored at both ends, with no metacharacter outside an escape.
fn is_exact_value(regex: &str) -> bool {
    let Some(body) = regex
        .strip_prefix('^')
        .and_then(|rest| rest.strip_suffix('$'))
    else {
        return false;
    };
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        let literal = if character == '\\' {
            characters
                .next()
                .is_some_and(|escaped| !escaped.is_alphanumeric())
        } else {
            !REGEX_META.contains(&character)
        };
        if !literal {
            return false;
        }
    }
    true
}

fn line_of(source: &str, regex: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(regex))
        .map_or(1, |index| index + 1)
}

fn message(regex: &str) -> String {
    format!(
        "allowlist regex `{regex}` matches anywhere in the repository; add `paths` or `commits`, or anchor the exact value as `^value$`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(source: &str) -> Vec<usize> {
        findings(source).into_iter().map(|(line, _)| line).collect()
    }

    #[test]
    fn unscoped_pattern_regexes_are_findings() {
        let source = [
            "[[allowlists]]",
            "regexes = ['''^[a-z]+_[0-9]+$''']",
            "",
            "[[allowlists]]",
            "regexes = ['''EXPO_PUBLIC_KEY''', '''^value$''']",
            "paths = []",
        ]
        .join("\n");
        assert_eq!(lines(&source), vec![2, 5]);
    }

    #[test]
    fn legacy_and_rule_allowlists_are_checked() {
        let source = [
            "[allowlist]",
            "regexes = ['''.*''']",
            "[[rules]]",
            "id = \"x\"",
            "[[rules.allowlists]]",
            "regexes = ['''^a.c$''']",
        ]
        .join("\n");
        assert_eq!(lines(&source), vec![2, 6]);
    }

    #[test]
    fn exact_anchored_values_pass() {
        let source = [
            "[[allowlists]]",
            "regexes = [",
            "  '''^e3e32936605bb2fa7e1c19aa16ebc5ff12e5ee1283021b34e1271a02669598e7$''',",
            "  '''^EXCLUDED\\.token,partition_id=EXCLUDED\\.partition_id\"$''',",
            "  '''^dedupe_key\":\\ \"6e8f\"$''',",
            "]",
        ]
        .join("\n");
        assert!(lines(&source).is_empty());
    }

    #[test]
    fn paths_or_commits_scope_any_regex() {
        let source = [
            "[[allowlists]]",
            "regexes = ['''^[0-9a-f]{64}$''']",
            "paths = ['''^fixtures/''']",
            "[[allowlists]]",
            "regexes = ['''(?i)token''']",
            "commits = [\"abc\"]",
        ]
        .join("\n");
        assert!(lines(&source).is_empty());
    }

    #[test]
    fn escaped_classes_and_unanchored_literals_are_patterns() {
        assert!(!is_exact_value(r"^\d+$"));
        assert!(!is_exact_value("abc"));
        assert!(!is_exact_value("^abc"));
        assert!(!is_exact_value(r"^abc\$"));
        assert!(is_exact_value(r"^a\.b\\$"));
    }

    #[test]
    fn invalid_toml_and_allowlists_without_regexes_pass() {
        assert!(findings("[[allowlists").is_empty());
        assert!(lines("[[allowlists]]\ndescription = \"x\"\nstopwords = [\"a\"]\n").is_empty());
    }
}

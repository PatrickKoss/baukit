//! golangci-lint settings that switch linters off or hide findings wholesale.

use super::bulk::is_generated_output;

const DISABLING_KEYS: &[&str] = &["disable", "disable-all"];
const HIDING_KEYS: &[&str] = &[
    "exclude",
    "exclude-rules",
    "exclude-files",
    "skip-files",
    "presets",
    "rules",
];
const DIRECTORY_KEYS: &[&str] = &["exclude-dirs", "skip-dirs", "paths"];
const BASELINE_KEYS: &[&str] = &[
    "new",
    "new-from-rev",
    "new-from-patch",
    "new-from-merge-base",
];
const CONFIG_NAMES: &[&str] = &[".golangci.yml", ".golangci.yaml", ".golangci.toml"];

pub fn is_config(name: &str) -> bool {
    CONFIG_NAMES.contains(&name)
}

/// One `key: value` (or `key = value`) with the keys of the tables around it.
struct Entry {
    line: usize,
    path: Vec<String>,
    value: String,
}

/// One list element and the key path of the list that holds it.
struct Item {
    line: usize,
    path: Vec<String>,
    value: String,
}

#[derive(Default)]
struct Parsed {
    entries: Vec<Entry>,
    items: Vec<Item>,
}

/// `(line, message)` for every weakening setting in a golangci config.
pub fn findings(name: &str, source: &str) -> Vec<(usize, String)> {
    let parsed = if name.ends_with(".toml") {
        parse_toml(source)
    } else {
        parse_yaml(source)
    };
    let mut found: Vec<(usize, String)> = parsed.entries.iter().filter_map(entry_finding).collect();
    found.extend(parsed.items.iter().filter_map(item_finding));
    found.sort();
    found
}

fn entry_finding(entry: &Entry) -> Option<(usize, String)> {
    let key = entry.path.last()?.as_str();
    let parent = entry
        .path
        .len()
        .checked_sub(2)
        .and_then(|index| entry.path.get(index))
        .map(String::as_str);
    let value = entry.value.trim();
    let message = if key == "disable-all" && value == "true" {
        "`disable-all` switches every linter off; enable the shared set and fix the findings"
            .to_owned()
    } else if key == "default" && parent == Some("linters") && value == "none" {
        "`linters.default: none` switches every linter off; fix the findings instead".to_owned()
    } else if BASELINE_KEYS.contains(&key) && parent == Some("issues") && !is_off(value) {
        format!(
            "`issues.{key}` reports only new findings, a bulk baseline; fix the findings instead"
        )
    } else if HIDING_KEYS.contains(&key) && hides(&entry.path) && !is_empty(value) {
        format!("`{key}` hides findings in bulk; fix them or suppress one site with a reason")
    } else {
        return None;
    };
    Some((entry.line, message))
}

/// Exclusion keys live under `issues` (v1) or `linters.exclusions` (v2).
fn hides(path: &[String]) -> bool {
    path.iter()
        .any(|key| key == "issues" || key == "exclusions")
}

fn item_finding(item: &Item) -> Option<(usize, String)> {
    let key = item.path.last()?.as_str();
    let value = item.value.as_str();
    if DISABLING_KEYS.contains(&key) && item.path.iter().any(|key| key == "linters") {
        return Some((
            item.line,
            format!("linter `{value}` disabled; fix its findings instead"),
        ));
    }
    if DIRECTORY_KEYS.contains(&key) && hides(&item.path) && !is_generated_output(value) {
        return Some((
            item.line,
            format!(
                "`{key}` entry `{value}` hides source from the linters; only generated or build output may be excluded"
            ),
        ));
    }
    None
}

fn is_off(value: &str) -> bool {
    matches!(value, "false" | "\"\"" | "''" | "")
}

fn is_empty(value: &str) -> bool {
    matches!(value, "[]" | "{}" | "false" | "null" | "~")
}

fn parse_yaml(source: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut stack: Vec<(usize, String)> = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let content = strip_comment(line);
        let trimmed = content.trim();
        if trimmed.is_empty() || trimmed == "---" {
            continue;
        }
        let indent = content.len() - content.trim_start().len();
        if let Some(item) = trimmed.strip_prefix('-') {
            while stack.last().is_some_and(|(open, _)| *open > indent) {
                stack.pop();
            }
            parsed.items.push(Item {
                line: index + 1,
                path: keys(&stack),
                value: unquote(item.trim()).to_owned(),
            });
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        while stack.last().is_some_and(|(open, _)| *open >= indent) {
            stack.pop();
        }
        stack.push((indent, unquote(key.trim()).to_owned()));
        record(&mut parsed, index + 1, keys(&stack), value.trim());
    }
    parsed
}

fn parse_toml(source: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut table: Vec<String> = Vec::new();
    let mut open_list: Option<Vec<String>> = None;
    for (index, line) in source.lines().enumerate() {
        let trimmed = strip_comment(line).trim();
        if let Some(path) = &open_list {
            for value in list_values(trimmed.trim_end_matches(']')) {
                parsed.items.push(Item {
                    line: index + 1,
                    path: path.clone(),
                    value,
                });
            }
            if trimmed.contains(']') {
                open_list = None;
            }
            continue;
        }
        if let Some(header) = trimmed.strip_prefix('[') {
            table = header
                .trim_matches(['[', ']'])
                .split('.')
                .map(|key| unquote(key.trim()).to_owned())
                .collect();
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let mut path = table.clone();
        path.extend(key.split('.').map(|key| unquote(key.trim()).to_owned()));
        let value = value.trim();
        if value.starts_with('[') && !value.contains(']') {
            open_list = Some(path.clone());
        }
        record(&mut parsed, index + 1, path, value);
    }
    parsed
}

/// Records an entry, plus the elements of an inline `[a, b]` list.
fn record(parsed: &mut Parsed, line: usize, path: Vec<String>, value: &str) {
    if let Some(inner) = value.strip_prefix('[') {
        for element in list_values(inner.trim_end_matches(']')) {
            parsed.items.push(Item {
                line,
                path: path.clone(),
                value: element,
            });
        }
    }
    parsed.entries.push(Entry {
        line,
        path,
        value: value.to_owned(),
    });
}

fn list_values(text: &str) -> Vec<String> {
    text.split(',')
        .map(|value| unquote(value.trim()).to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

fn keys(stack: &[(usize, String)]) -> Vec<String> {
    stack.iter().map(|(_, key)| key.clone()).collect()
}

fn unquote(text: &str) -> &str {
    text.trim_matches(['"', '\''])
}

/// Drops a `#` comment that starts outside a quoted value.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (offset, character) in line.char_indices() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {}
            None if matches!(character, '"' | '\'') => quote = Some(character),
            None if character == '#' => return line.get(..offset).unwrap_or(line),
            None => {}
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(name: &str, source: &str) -> Vec<usize> {
        findings(name, source)
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    #[test]
    fn yaml_disabling_and_excluding_are_findings() {
        let source = [
            "linters:",
            "  disable-all: true",
            "  enable:",
            "    - errcheck",
            "  disable:",
            "    - gosec",
            "    - unused # too noisy",
            "issues:",
            "  exclude-rules:",
            "    - path: _test.go",
            "      linters: [errcheck]",
            "  exclude-dirs:",
            "    - vendor",
            "    - internal/legacy",
            "  skip-dirs: [dist, pkg/old]",
            "  new-from-rev: HEAD~1",
            "  exclude: []",
        ]
        .join("\n");
        assert_eq!(
            lines(".golangci.yml", &source),
            vec![2, 6, 7, 9, 14, 15, 16]
        );
    }

    #[test]
    fn yaml_v2_defaults_and_exclusions() {
        let source = [
            "version: \"2\"",
            "linters:",
            "  default: none",
            "  exclusions:",
            "    generated: lax",
            "    presets: [comments]",
            "    paths:",
            "      - third_party/generated",
            "      - cmd/tool",
            "formatters:",
            "  default: none",
        ]
        .join("\n");
        assert_eq!(lines(".golangci.yaml", &source), vec![3, 6, 9]);
    }

    #[test]
    fn clean_configs_pass() {
        let source = "linters:\n  enable:\n    - errcheck\n    - gosec\nissues:\n  new: false\n  exclude-dirs:\n    - node_modules\n";
        assert!(lines(".golangci.yml", source).is_empty());
    }

    #[test]
    fn toml_configs_are_checked() {
        let source = [
            "[linters]",
            "disable-all = true",
            "disable = [\"gosec\"]",
            "enable = [\"errcheck\"]",
            "[issues]",
            "exclude-dirs = [",
            "  \"vendor\",",
            "  \"internal/legacy\",",
            "]",
            "new = true",
        ]
        .join("\n");
        assert_eq!(lines(".golangci.toml", &source), vec![2, 3, 8, 10]);
    }
}

use std::path::Path;

use super::walk;

/// The suppression kinds that `baukit check suppressions` budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SuppressionKind {
    RustAllow,
    RustExpect,
    EslintDisable,
    AstGrepIgnore,
}

impl SuppressionKind {
    pub const ALL: [Self; 4] = [
        Self::RustAllow,
        Self::RustExpect,
        Self::EslintDisable,
        Self::AstGrepIgnore,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::RustAllow => "rust-allow",
            Self::RustExpect => "rust-expect",
            Self::EslintDisable => "eslint-disable",
            Self::AstGrepIgnore => "ast-grep-ignore",
        }
    }
}

/// One suppression found in a source file; `missing_reason` is set when it lacks the
/// required reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    pub kind: SuppressionKind,
    pub line: usize,
    pub missing_reason: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    Script,
}

const RUST_EXTENSIONS: &[&str] = &["rs"];
const SCRIPT_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
const TEMPLATE_SUFFIXES: &[&str] = &[".j2", ".jinja"];
const REASON_SEPARATOR: &str = " -- ";
const ATTRIBUTE_SCAN_LINES: usize = 20;

fn language(path: &Path) -> Option<Language> {
    let name = walk::file_name(path);
    let name = TEMPLATE_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(name);
    let extension = name.rsplit_once('.')?.1;
    if RUST_EXTENSIONS.contains(&extension) {
        Some(Language::Rust)
    } else if SCRIPT_EXTENSIONS.contains(&extension) {
        Some(Language::Script)
    } else {
        None
    }
}

pub fn is_scanned(path: &Path) -> bool {
    language(path).is_some()
}

/// Lists the suppressions in one file's source text.
pub fn scan(path: &Path, source: &str) -> Vec<Suppression> {
    let Some(language) = language(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if language == Language::Rust
            && let Some(suppression) = rust_attribute(&lines, index)
        {
            found.push(suppression);
        }
        if let Some(comment) = comment_text(line, language) {
            found.extend(comment_suppression(comment, index + 1));
        }
    }
    found
}

fn rust_attribute(lines: &[&str], index: usize) -> Option<Suppression> {
    let trimmed = lines.get(index)?.trim_start();
    let body = trimmed
        .strip_prefix("#![")
        .or_else(|| trimmed.strip_prefix("#["))?;
    let kind = if body.starts_with("allow(") {
        SuppressionKind::RustAllow
    } else if body.starts_with("expect(") {
        SuppressionKind::RustExpect
    } else {
        return None;
    };
    let attribute = attribute_text(lines, index);
    Some(Suppression {
        kind,
        line: index + 1,
        missing_reason: kind == SuppressionKind::RustAllow
            && !attribute
                .split_whitespace()
                .collect::<String>()
                .contains("reason="),
    })
}

fn attribute_text(lines: &[&str], start: usize) -> String {
    let mut text = String::new();
    let mut depth = 0_i32;
    for line in lines.iter().skip(start).take(ATTRIBUTE_SCAN_LINES) {
        text.push_str(line);
        text.push('\n');
        depth += bracket_balance(line);
        if depth <= 0 {
            break;
        }
    }
    text
}

fn bracket_balance(line: &str) -> i32 {
    line.chars().fold(0, |depth, character| match character {
        '[' => depth + 1,
        ']' => depth - 1,
        _ => depth,
    })
}

/// Returns the text after the first comment opener that sits outside a string literal.
fn comment_text(line: &str, language: Language) -> Option<&str> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (offset, character) in line.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open {
                quote = None;
            }
            continue;
        }
        if is_quote(character, language) {
            quote = Some(character);
            continue;
        }
        let rest = line.get(offset..)?;
        if let Some(comment) = rest.strip_prefix("//").or_else(|| rest.strip_prefix("/*")) {
            return Some(comment);
        }
    }
    None
}

fn is_quote(character: char, language: Language) -> bool {
    match language {
        Language::Rust => character == '"',
        Language::Script => matches!(character, '"' | '\'' | '`'),
    }
}

fn comment_suppression(comment: &str, line: usize) -> Option<Suppression> {
    let comment = comment.trim_start_matches(['/', '*', '!']).trim_start();
    let kind = if comment.starts_with("eslint-disable") {
        SuppressionKind::EslintDisable
    } else if comment.starts_with("ast-grep-ignore") {
        SuppressionKind::AstGrepIgnore
    } else {
        return None;
    };
    Some(Suppression {
        kind,
        line,
        missing_reason: !has_comment_reason(comment),
    })
}

fn has_comment_reason(comment: &str) -> bool {
    comment
        .split_once(REASON_SEPARATOR)
        .is_some_and(|(_, reason)| !reason.trim().trim_end_matches("*/").trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(file: &str, source: &str) -> Vec<(SuppressionKind, usize, bool)> {
        scan(Path::new(file), source)
            .into_iter()
            .map(|found| (found.kind, found.line, found.missing_reason))
            .collect()
    }

    #[test]
    fn rust_allow_needs_a_reason_even_across_lines() {
        let source = [
            "#[allow(dead_code)]",
            "#![allow(",
            "    clippy::x,",
            "    reason = \"r\"",
            ")]",
            "#[expect(clippy::y)]",
            "let text = \"#[allow(x)]\";",
        ]
        .join("\n");
        assert_eq!(
            kinds("a.rs", &source),
            vec![
                (SuppressionKind::RustAllow, 1, true),
                (SuppressionKind::RustAllow, 2, false),
                (SuppressionKind::RustExpect, 6, false),
            ]
        );
    }

    #[test]
    fn comment_suppressions_need_a_reason() {
        let source = [
            "// eslint-disable-next-line no-console -- debug output in a CLI",
            "foo(); // eslint-disable-line no-x",
            "{/* eslint-disable-line a11y -- decorative */}",
            "const s = '// eslint-disable-line';",
            "// ast-grep-ignore: ts-double-cast",
            "/* ast-grep-ignore: ts-double-cast --  */",
        ]
        .join("\n");
        assert_eq!(
            kinds("a.tsx", &source),
            vec![
                (SuppressionKind::EslintDisable, 1, false),
                (SuppressionKind::EslintDisable, 2, true),
                (SuppressionKind::EslintDisable, 3, false),
                (SuppressionKind::AstGrepIgnore, 5, true),
                (SuppressionKind::AstGrepIgnore, 6, true),
            ]
        );
    }

    #[test]
    fn template_suffixes_and_other_files() {
        assert!(is_scanned(Path::new("lib.rs.j2")));
        assert!(is_scanned(Path::new("a.mjs")));
        assert!(!is_scanned(Path::new("README.md")));
    }
}

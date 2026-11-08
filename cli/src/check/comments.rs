use std::path::Path;

use super::{hash_comments, markup_comments, walk};

/// The suppression kinds that `baukit check suppressions` budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SuppressionKind {
    RustAllow,
    RustExpect,
    EslintDisable,
    AstGrepIgnore,
    TsExpectError,
    TsIgnore,
    PrettierIgnore,
    Noqa,
    TypeIgnore,
    Nosec,
    ShellcheckDisable,
    Nolint,
}

impl SuppressionKind {
    pub const ALL: [Self; 12] = [
        Self::RustAllow,
        Self::RustExpect,
        Self::EslintDisable,
        Self::AstGrepIgnore,
        Self::TsExpectError,
        Self::TsIgnore,
        Self::PrettierIgnore,
        Self::Noqa,
        Self::TypeIgnore,
        Self::Nosec,
        Self::ShellcheckDisable,
        Self::Nolint,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::RustAllow => "rust-allow",
            Self::RustExpect => "rust-expect",
            Self::EslintDisable => "eslint-disable",
            Self::AstGrepIgnore => "ast-grep-ignore",
            Self::TsExpectError => "ts-expect-error",
            Self::TsIgnore => "ts-ignore",
            Self::PrettierIgnore => "prettier-ignore",
            Self::Noqa => "noqa",
            Self::TypeIgnore => "type-ignore",
            Self::Nosec => "nosec",
            Self::ShellcheckDisable => "shellcheck-disable",
            Self::Nolint => "nolint",
        }
    }

    /// What a finding says when this suppression lacks its reason.
    pub const fn missing_reason_message(self) -> &'static str {
        match self {
            Self::RustAllow => "`#[allow]` without `reason = \"...\"`",
            Self::RustExpect => "`#[expect]` without a reason",
            Self::EslintDisable => "`eslint-disable` without ` -- reason`",
            Self::AstGrepIgnore => "`ast-grep-ignore` without ` -- reason`",
            Self::TsExpectError => "`@ts-expect-error` without ` -- reason`",
            Self::TsIgnore => "`@ts-ignore` is forbidden; use `@ts-expect-error -- reason`",
            Self::PrettierIgnore => "`prettier-ignore` without a reason comment on the line above",
            Self::Noqa => "`# noqa` must name codes and give a reason: `# noqa: CODE -- reason`",
            Self::TypeIgnore => {
                "`# type: ignore` must name codes and give a reason: `# type: ignore[code]  # reason`"
            }
            Self::Nosec => "`nosec` without ` -- reason`",
            Self::Nolint => {
                "`//nolint` must name linters and give a reason: `//nolint:linter // reason`"
            }
            Self::ShellcheckDisable => {
                "`# shellcheck disable=` without a reason (`# reason` after it or a comment on the line above)"
            }
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
    Python,
    Shell,
    Go,
    Markdown,
    Html,
    Component,
}

const RUST_EXTENSIONS: &[&str] = &["rs"];
const SCRIPT_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
const PYTHON_EXTENSIONS: &[&str] = &["py", "pyi"];
const SHELL_EXTENSIONS: &[&str] = &["sh", "bash"];
const SHELL_INTERPRETERS: &[&str] = &["sh", "bash", "zsh", "dash"];
const TEMPLATE_SUFFIXES: &[&str] = &[".j2", ".jinja"];
const REASON_SEPARATOR: &str = " -- ";
const ATTRIBUTE_SCAN_LINES: usize = 20;

fn language(path: &Path) -> Option<Language> {
    let extension = base_name(path).rsplit_once('.')?.1;
    if RUST_EXTENSIONS.contains(&extension) {
        Some(Language::Rust)
    } else if SCRIPT_EXTENSIONS.contains(&extension) {
        Some(Language::Script)
    } else if PYTHON_EXTENSIONS.contains(&extension) {
        Some(Language::Python)
    } else if SHELL_EXTENSIONS.contains(&extension) {
        Some(Language::Shell)
    } else {
        match extension {
            "go" => Some(Language::Go),
            "md" | "mdx" => Some(Language::Markdown),
            "html" => Some(Language::Html),
            "vue" | "svelte" => Some(Language::Component),
            _ => None,
        }
    }
}

fn base_name(path: &Path) -> &str {
    let name = walk::file_name(path);
    TEMPLATE_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(name)
}

/// A file name without an extension, which may be a script that only a `#!` line identifies.
pub(super) fn has_no_extension(path: &Path) -> bool {
    !base_name(path).contains('.')
}

/// Whether the first line is a `#!` naming sh, bash, zsh or dash, directly or through `env`.
pub(super) fn has_shell_shebang(source: &str) -> bool {
    let Some(command) = source
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("#!"))
    else {
        return false;
    };
    let mut words = command.split_whitespace();
    let Some(program) = words
        .next()
        .map(|word| word.rsplit('/').next().unwrap_or(word))
    else {
        return false;
    };
    let interpreter = if program == "env" {
        words.find(|word| !word.starts_with('-'))
    } else {
        Some(program)
    };
    interpreter.is_some_and(|name| SHELL_INTERPRETERS.contains(&name))
}

pub fn is_scanned(path: &Path) -> bool {
    language(path).is_some() || has_no_extension(path)
}

/// Lists the suppressions in one file's source text.
pub fn scan(path: &Path, source: &str) -> Vec<Suppression> {
    let shebang =
        || (has_no_extension(path) && has_shell_shebang(source)).then_some(Language::Shell);
    let Some(language) = language(path).or_else(shebang) else {
        return Vec::new();
    };
    let lines: Vec<&str> = source.lines().collect();
    match language {
        Language::Python => return hash_comments::scan(&lines, hash_comments::Dialect::Python),
        Language::Shell => return hash_comments::scan(&lines, hash_comments::Dialect::Shell),
        Language::Markdown => {
            return markup_comments::scan(&lines, markup_comments::Markup::Markdown);
        }
        Language::Html => return markup_comments::scan(&lines, markup_comments::Markup::Html),
        Language::Component => {
            let mut found = markup_comments::scan(&lines, markup_comments::Markup::Html);
            found.extend(scan_line_comments(&lines, Language::Script));
            found.sort_by_key(|suppression| suppression.line);
            return found;
        }
        Language::Rust | Language::Script | Language::Go => {}
    }
    scan_line_comments(&lines, language)
}

fn scan_line_comments(lines: &[&str], language: Language) -> Vec<Suppression> {
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if language == Language::Rust
            && let Some(suppression) = rust_attribute(lines, index)
        {
            found.push(suppression);
        }
        if let Some(comment) = comment_text(line, language) {
            found.extend(comment_suppression(comment, lines, index, language));
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
        Language::Script
        | Language::Python
        | Language::Shell
        | Language::Go
        | Language::Markdown
        | Language::Html
        | Language::Component => matches!(character, '"' | '\'' | '`'),
    }
}

fn comment_suppression(
    comment: &str,
    lines: &[&str],
    index: usize,
    language: Language,
) -> Option<Suppression> {
    let raw = comment;
    let comment = comment.trim_start_matches(['/', '*', '!']).trim_start();
    let detected = if language == Language::Go {
        go_directive(raw, comment)
    } else {
        script_directive(comment, lines, index)
    };
    let (kind, missing_reason) = detected?;
    Some(Suppression {
        kind,
        line: index + 1,
        missing_reason,
    })
}

/// golangci-lint only reads `//nolint` with no space after the slashes, so `raw` is the text
/// right after the opener and `comment` the trimmed text.
fn go_directive(raw: &str, comment: &str) -> Option<(SuppressionKind, bool)> {
    if comment.starts_with("#nosec") {
        return Some((SuppressionKind::Nosec, !has_comment_reason(comment)));
    }
    let rest = raw.strip_prefix("nolint")?;
    let reasoned = rest.strip_prefix(':').is_some_and(|rest| {
        rest.split_once("//").is_some_and(|(linters, reason)| {
            !linters.trim().is_empty() && !reason.trim().is_empty()
        })
    });
    Some((SuppressionKind::Nolint, !reasoned))
}

fn script_directive(
    comment: &str,
    lines: &[&str],
    index: usize,
) -> Option<(SuppressionKind, bool)> {
    let detected = if comment.starts_with("eslint-disable") {
        (SuppressionKind::EslintDisable, !has_comment_reason(comment))
    } else if comment.starts_with("ast-grep-ignore") {
        (SuppressionKind::AstGrepIgnore, !has_comment_reason(comment))
    } else if comment.starts_with("@ts-expect-error") {
        (SuppressionKind::TsExpectError, !has_comment_reason(comment))
    } else if comment.starts_with("@ts-ignore") {
        (SuppressionKind::TsIgnore, true)
    } else if comment.starts_with("prettier-ignore") {
        let above = index.checked_sub(1).and_then(|above| lines.get(above));
        let reasoned = above.is_some_and(|line| is_reason_line(line, &["//", "/*", "*"]));
        (SuppressionKind::PrettierIgnore, !reasoned)
    } else {
        return None;
    };
    Some(detected)
}

pub(super) fn has_comment_reason(comment: &str) -> bool {
    comment
        .split_once(REASON_SEPARATOR)
        .is_some_and(|(_, reason)| !reason.trim().trim_end_matches("*/").trim().is_empty())
}

/// A comment-only line with text that is not itself a suppression directive.
pub(super) fn is_reason_line(line: &str, openers: &[&str]) -> bool {
    let trimmed = line.trim();
    let Some(text) = openers
        .iter()
        .find_map(|opener| trimmed.strip_prefix(opener))
    else {
        return false;
    };
    let text = text.trim_start_matches(['/', '*', '#', '!']).trim();
    !text.is_empty()
        && !DIRECTIVE_PREFIXES
            .iter()
            .any(|prefix| text.starts_with(prefix))
}

const DIRECTIVE_PREFIXES: &[&str] = &[
    "eslint-",
    "ast-grep-ignore",
    "@ts-",
    "prettier-ignore",
    "shellcheck ",
    "noqa",
    "type:",
    "nosec",
    "#nosec",
    "nolint",
];

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
    fn typescript_directives_need_a_reason() {
        let source = [
            "// @ts-expect-error -- a Date is not a JSON value",
            "// @ts-expect-error a Date is not a JSON value",
            "// @ts-ignore -- even with a reason",
            "const s = '// @ts-ignore';",
            "// keeps the table aligned",
            "// prettier-ignore",
            "const table = [1,0];",
            "",
            "// prettier-ignore",
            "// eslint-disable-next-line x -- reason",
            "// prettier-ignore",
        ]
        .join("\n");
        assert_eq!(
            kinds("a.ts", &source),
            vec![
                (SuppressionKind::TsExpectError, 1, false),
                (SuppressionKind::TsExpectError, 2, true),
                (SuppressionKind::TsIgnore, 3, true),
                (SuppressionKind::PrettierIgnore, 6, false),
                (SuppressionKind::PrettierIgnore, 9, true),
                (SuppressionKind::EslintDisable, 10, false),
                (SuppressionKind::PrettierIgnore, 11, true),
            ]
        );
    }

    #[test]
    fn python_and_shell_files_are_scanned() {
        assert!(is_scanned(Path::new("tool.py")));
        assert!(is_scanned(Path::new("gate.sh.j2")));
        assert_eq!(
            kinds("tool.py", "x()  # noqa\n"),
            vec![(SuppressionKind::Noqa, 1, true)]
        );
        assert_eq!(
            kinds("gate.sh", "# shellcheck disable=SC2086\n"),
            vec![(SuppressionKind::ShellcheckDisable, 1, true)]
        );
    }

    #[test]
    fn extensionless_scripts_are_shell_only_with_a_shell_shebang() {
        let directive = "# shellcheck disable=SC2086\n";
        for shebang in [
            "#!/bin/sh",
            "#!/usr/bin/env bash",
            "#!/bin/zsh -e",
            "#!/usr/bin/env -S dash",
        ] {
            let source = format!("{shebang}\n{directive}");
            assert_eq!(
                kinds("bin/gate", &source),
                vec![(SuppressionKind::ShellcheckDisable, 2, false)],
                "{shebang}"
            );
        }
        for other in ["#!/usr/bin/env python3\n", "#!/usr/bin/env fish\n", ""] {
            let source = format!("{other}{directive}");
            assert!(kinds("bin/tool", &source).is_empty(), "{other}");
        }
        assert!(kinds("gate.txt", &format!("#!/bin/sh\n{directive}")).is_empty());
        assert!(is_scanned(Path::new("bin/gate")));
        assert!(is_scanned(Path::new("bin/gate.j2")));
    }

    #[test]
    fn go_nosec_and_nolint_need_reasons() {
        let source = [
            "x := run() // #nosec G204 -- argv is a fixed list",
            "y := run() // #nosec G204",
            "z := f() //nolint:errcheck // close errors are logged by the caller",
            "w := f() //nolint:errcheck",
            "v := f() //nolint // no linter named",
            "s := \"//nolint:errcheck\"",
            "r := `//nolint`",
            "// nolint is a word in this sentence",
        ]
        .join("\n");
        assert_eq!(
            kinds("main.go", &source),
            vec![
                (SuppressionKind::Nosec, 1, false),
                (SuppressionKind::Nosec, 2, true),
                (SuppressionKind::Nolint, 3, false),
                (SuppressionKind::Nolint, 4, true),
                (SuppressionKind::Nolint, 5, true),
            ]
        );
    }

    #[test]
    fn markup_and_components_count_prettier_ignore() {
        assert_eq!(
            kinds("README.md", "<!-- prettier-ignore -->\n"),
            vec![(SuppressionKind::PrettierIgnore, 1, true)]
        );
        assert_eq!(
            kinds(
                "page.html",
                "<!-- layout is hand-aligned -->\n<!-- prettier-ignore -->\n"
            ),
            vec![(SuppressionKind::PrettierIgnore, 2, false)]
        );
        let component = "<!-- prettier-ignore -->\n<script>\n// @ts-ignore\n</script>\n";
        assert_eq!(
            kinds("App.vue", component),
            vec![
                (SuppressionKind::PrettierIgnore, 1, true),
                (SuppressionKind::TsIgnore, 3, true),
            ]
        );
        assert_eq!(kinds("App.svelte", component).len(), 2);
        assert!(kinds("notes.txt", "<!-- prettier-ignore -->\n").is_empty());
    }

    #[test]
    fn template_suffixes_and_other_files() {
        assert!(is_scanned(Path::new("lib.rs.j2")));
        assert!(is_scanned(Path::new("a.mjs")));
        assert!(!is_scanned(Path::new("notes.txt")));
    }
}

use super::comments::{Suppression, SuppressionKind, has_comment_reason, is_reason_line};

/// Languages whose comments start with `#`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Python,
    Shell,
}

const TRIPLE_QUOTES: &[&str] = &["\"\"\"", "'''"];

/// Lists the suppressions in a Python or shell file.
pub fn scan(lines: &[&str], dialect: Dialect) -> Vec<Suppression> {
    let mut found = Vec::new();
    let mut open_triple: Option<&str> = None;
    let mut continued: Option<char> = None;
    for (index, line) in lines.iter().enumerate() {
        if dialect == Dialect::Python {
            let was_open = open_triple.is_some();
            open_triple = triple_quote_state(line, open_triple);
            if was_open || open_triple.is_some() {
                continue;
            }
        }
        let scanned = comment_text(line, dialect, continued);
        continued = scanned.continued;
        let Some(comment) = scanned.comment else {
            continue;
        };
        found.extend(comment_suppressions(comment, lines, index, dialect));
    }
    found
}

/// Tracks whether a Python triple-quoted string is still open after `line`.
fn triple_quote_state<'a>(line: &str, open: Option<&'a str>) -> Option<&'a str> {
    let mut open = open;
    let mut rest = line;
    loop {
        match open {
            Some(quote) => match rest.split_once(quote) {
                Some((_, after)) => {
                    open = None;
                    rest = after;
                }
                None => return open,
            },
            None => {
                let next = TRIPLE_QUOTES
                    .iter()
                    .filter_map(|quote| rest.find(quote).map(|offset| (offset, *quote)))
                    .min_by_key(|(offset, _)| *offset);
                let (offset, quote) = next?;
                if rest
                    .get(..offset)
                    .is_some_and(|before| before.contains('#'))
                {
                    return None;
                }
                open = Some(quote);
                rest = rest.get(offset + quote.len()..).unwrap_or_default();
            }
        }
    }
}

/// Where a line's comment starts, and the quote a backslash continues onto the next line.
struct LineScan<'a> {
    comment: Option<&'a str>,
    continued: Option<char>,
}

/// Finds the first `#` that opens a comment outside a string literal. `carried` is a Python
/// string continued from the previous line. A quote that never closes and is not continued is
/// a stray character, so the scan resumes after it instead of hiding the rest of the line.
fn comment_text(line: &str, dialect: Dialect, carried: Option<char>) -> LineScan<'_> {
    let mut quote = carried;
    let mut opened_at: Option<usize> = None;
    let mut escaped = false;
    let mut previous: Option<char> = None;
    for (offset, character) in line.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' && (open != '\'' || dialect == Dialect::Python) {
                escaped = true;
            } else if character == open {
                quote = None;
                opened_at = None;
            }
            previous = Some(character);
            continue;
        }
        if matches!(character, '"' | '\'') {
            quote = Some(character);
            opened_at = Some(offset);
        } else if character == '#' && opens_comment(previous, dialect) {
            return LineScan {
                comment: line.get(offset + 1..),
                continued: None,
            };
        }
        previous = Some(character);
    }
    let continues = dialect == Dialect::Python && line.trim_end().ends_with('\\');
    if continues {
        return LineScan {
            comment: None,
            continued: quote,
        };
    }
    let resumed = opened_at
        .and_then(|offset| line.get(offset + 1..))
        .and_then(|rest| comment_text(rest, dialect, None).comment);
    LineScan {
        comment: resumed,
        continued: None,
    }
}

fn opens_comment(previous: Option<char>, dialect: Dialect) -> bool {
    match dialect {
        Dialect::Python => true,
        Dialect::Shell => previous.is_none_or(char::is_whitespace),
    }
}

fn comment_suppressions(
    comment: &str,
    lines: &[&str],
    index: usize,
    dialect: Dialect,
) -> Vec<Suppression> {
    let segments: Vec<&str> = comment.split('#').map(str::trim).collect();
    let mut found = Vec::new();
    for (position, segment) in segments.iter().enumerate() {
        let later_reason = segments
            .iter()
            .skip(position + 1)
            .any(|later| !later.is_empty());
        let detected = match dialect {
            Dialect::Python => python_suppression(segment, later_reason),
            Dialect::Shell => shell_suppression(segment, later_reason, lines, index),
        };
        if let Some((kind, missing_reason)) = detected {
            found.push(Suppression {
                kind,
                line: index + 1,
                missing_reason,
            });
        }
    }
    found
}

fn python_suppression(segment: &str, later_reason: bool) -> Option<(SuppressionKind, bool)> {
    let lower = segment.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("noqa") {
        return Some((SuppressionKind::Noqa, !noqa_is_reasoned(rest)));
    }
    if let Some(rest) = segment.strip_prefix("type:")
        && let Some(codes) = rest.trim_start().strip_prefix("ignore")
    {
        let named = codes.trim_start().starts_with('[');
        return Some((SuppressionKind::TypeIgnore, !(named && later_reason)));
    }
    if lower.starts_with("nosec") {
        return Some((SuppressionKind::Nosec, !has_comment_reason(segment)));
    }
    None
}

fn noqa_is_reasoned(rest: &str) -> bool {
    let Some(codes) = rest.strip_prefix(':') else {
        return false;
    };
    let named = codes
        .split_once(" -- ")
        .is_some_and(|(codes, _)| !codes.trim().is_empty());
    named && has_comment_reason(codes)
}

fn shell_suppression(
    segment: &str,
    later_reason: bool,
    lines: &[&str],
    index: usize,
) -> Option<(SuppressionKind, bool)> {
    let directive = segment.strip_prefix("shellcheck ")?;
    if !directive.contains("disable=") {
        return None;
    }
    let above = index.checked_sub(1).and_then(|above| lines.get(above));
    let reasoned = later_reason || above.is_some_and(|line| is_reason_line(line, &["#"]));
    Some((SuppressionKind::ShellcheckDisable, !reasoned))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str, dialect: Dialect) -> Vec<(SuppressionKind, usize, bool)> {
        let lines: Vec<&str> = source.lines().collect();
        scan(&lines, dialect)
            .into_iter()
            .map(|found| (found.kind, found.line, found.missing_reason))
            .collect()
    }

    #[test]
    fn noqa_needs_codes_and_a_reason() {
        let source = [
            "x = 1  # noqa: S311 -- not used for security",
            "y = 2  # noqa",
            "z = 3  # noqa: S311",
            "w = 4  # NOQA -- reason without codes",
            "s = '# noqa'",
            "\"\"\"",
            "# noqa inside a docstring",
            "\"\"\"",
        ]
        .join("\n");
        assert_eq!(
            kinds(&source, Dialect::Python),
            vec![
                (SuppressionKind::Noqa, 1, false),
                (SuppressionKind::Noqa, 2, true),
                (SuppressionKind::Noqa, 3, true),
                (SuppressionKind::Noqa, 4, true),
            ]
        );
    }

    #[test]
    fn type_ignore_needs_codes_and_a_trailing_reason() {
        let source = [
            "a = f()  # type: ignore[attr-defined]  # stub lacks the attribute",
            "b = f()  # type: ignore",
            "c = f()  # type: ignore[attr-defined]",
            "d = f()  # type: int",
            "e = f()  # type: ignore[x]  # noqa: E501 -- long generated line",
        ]
        .join("\n");
        assert_eq!(
            kinds(&source, Dialect::Python),
            vec![
                (SuppressionKind::TypeIgnore, 1, false),
                (SuppressionKind::TypeIgnore, 2, true),
                (SuppressionKind::TypeIgnore, 3, true),
                (SuppressionKind::TypeIgnore, 5, false),
                (SuppressionKind::Noqa, 5, false),
            ]
        );
    }

    #[test]
    fn quotes_continued_or_stray_classify_the_hash_correctly() {
        let source = [
            "message = \"first half \\",
            "    # noqa inside the continued string\"",
            "x = 1  # noqa",
            "label = 'it''s fine'  # noqa: E501 -- long label",
            "y = 2 ' # noqa",
        ]
        .join("\n");
        assert_eq!(
            kinds(&source, Dialect::Python),
            vec![
                (SuppressionKind::Noqa, 3, true),
                (SuppressionKind::Noqa, 4, false),
                (SuppressionKind::Noqa, 5, true),
            ]
        );
    }

    #[test]
    fn extensionless_shell_quotes_do_not_hide_directives() {
        let source = "echo don't  # shellcheck disable=SC2086\n";
        assert_eq!(
            kinds(source, Dialect::Shell),
            vec![(SuppressionKind::ShellcheckDisable, 1, true)]
        );
    }

    #[test]
    fn nosec_needs_a_reason() {
        let source = "a()  # nosec B603 -- fixed argv\nb()  # nosec B603\n";
        assert_eq!(
            kinds(source, Dialect::Python),
            vec![
                (SuppressionKind::Nosec, 1, false),
                (SuppressionKind::Nosec, 2, true),
            ]
        );
    }

    #[test]
    fn shellcheck_disable_takes_a_reason_on_the_line_or_above() {
        let source = [
            "# the list is space-separated words",
            "# shellcheck disable=SC2086",
            "echo $list",
            "# shellcheck disable=SC2086 # one argument per word",
            "echo $list",
            "",
            "# shellcheck disable=SC1090",
            "echo \"# shellcheck disable=SC1090\"",
            "echo ${#list}",
            "# shellcheck source=lib.sh",
        ]
        .join("\n");
        assert_eq!(
            kinds(&source, Dialect::Shell),
            vec![
                (SuppressionKind::ShellcheckDisable, 2, false),
                (SuppressionKind::ShellcheckDisable, 4, false),
                (SuppressionKind::ShellcheckDisable, 7, true),
            ]
        );
    }

    #[test]
    fn a_directive_above_is_not_a_reason() {
        let source = "# shellcheck disable=SC1090\n# shellcheck disable=SC2086\n";
        assert_eq!(
            kinds(source, Dialect::Shell),
            vec![
                (SuppressionKind::ShellcheckDisable, 1, true),
                (SuppressionKind::ShellcheckDisable, 2, true),
            ]
        );
    }
}

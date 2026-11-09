use super::comments::{Suppression, SuppressionKind};

/// Files whose comments are `<!-- ... -->`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Markup {
    /// Markdown and MDX: fenced code blocks hold examples, not directives.
    Markdown,
    /// HTML, and the template part of Vue and Svelte components.
    Html,
}

const OPENER: &str = "<!--";
const CLOSER: &str = "-->";
const FENCES: &[&str] = &["```", "~~~"];
const PRETTIER_IGNORE: &str = "prettier-ignore";
const PRETTIER_IGNORE_RANGE: &str = "prettier-ignore-start";

/// Lists the `<!-- prettier-ignore -->` and `<!-- prettier-ignore-start -->` directives in a
/// markup file.
pub fn scan(lines: &[&str], markup: Markup) -> Vec<Suppression> {
    let mut found = Vec::new();
    let mut in_fence = false;
    for (index, line) in lines.iter().enumerate() {
        if markup == Markup::Markdown && is_fence(line) {
            in_fence = !in_fence;
            continue;
        }
        let text = if markup == Markup::Markdown {
            without_code_spans(line)
        } else {
            (*line).to_owned()
        };
        if in_fence
            || !comment_bodies(&text)
                .any(|body| body == PRETTIER_IGNORE || body == PRETTIER_IGNORE_RANGE)
        {
            continue;
        }
        let above = index.checked_sub(1).and_then(|above| lines.get(above));
        found.push(Suppression {
            kind: SuppressionKind::PrettierIgnore,
            line: index + 1,
            missing_reason: !above.is_some_and(|line| is_reason_comment(line)),
        });
    }
    found
}

/// The line without its `` `code` `` spans, which hold documented examples.
fn without_code_spans(line: &str) -> String {
    line.split('`').step_by(2).collect()
}

fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    FENCES.iter().any(|fence| trimmed.starts_with(fence))
}

/// The trimmed text of every `<!-- ... -->` on the line.
fn comment_bodies(line: &str) -> impl Iterator<Item = &str> {
    line.split(OPENER)
        .skip(1)
        .filter_map(|rest| rest.split_once(CLOSER).map(|(body, _)| body.trim()))
}

/// A line holding only a comment whose text is not itself a directive.
fn is_reason_comment(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with(OPENER) || !trimmed.ends_with(CLOSER) {
        return false;
    }
    comment_bodies(trimmed)
        .next()
        .is_some_and(|body| !body.is_empty() && !body.starts_with(PRETTIER_IGNORE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str, markup: Markup) -> Vec<(usize, bool)> {
        let lines: Vec<&str> = source.lines().collect();
        scan(&lines, markup)
            .into_iter()
            .map(|found| (found.line, found.missing_reason))
            .collect()
    }

    #[test]
    fn prettier_ignore_needs_a_reason_comment_above() {
        let source = [
            "<!-- the table columns line up by hand -->",
            "<!-- prettier-ignore -->",
            "| a | b |",
            "",
            "<!-- prettier-ignore -->",
            "<!-- prettier-ignore -->",
            "<p>prettier-ignore in text</p>",
            "<!-- prettier-ignore-start -->",
        ]
        .join("\n");
        assert_eq!(
            kinds(&source, Markup::Html),
            vec![(2, false), (5, true), (6, true), (8, true)]
        );
    }

    #[test]
    fn markdown_skips_fenced_examples() {
        let source = [
            "```html",
            "<!-- prettier-ignore -->",
            "```",
            "<!-- prettier-ignore -->",
        ]
        .join("\n");
        assert_eq!(kinds(&source, Markup::Markdown), vec![(4, true)]);
        assert_eq!(kinds(&source, Markup::Html), vec![(2, true), (4, true)]);
        let documented = "| `prettier-ignore` | `<!-- prettier-ignore -->` |\n";
        assert!(kinds(documented, Markup::Markdown).is_empty());
        assert_eq!(kinds(documented, Markup::Html), vec![(1, true)]);
    }
}

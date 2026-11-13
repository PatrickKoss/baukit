//! eslint flat configs that switch rules off or keep source out of the lint.

use super::bulk::is_generated_output;

const CONFIG_STEM: &str = "eslint.config.";
const CONFIG_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "cts"];
const IGNORE_FILE: &str = ".eslintignore";
const IGNORE_KEYS: &[&str] = &["ignores", "ignorePatterns"];
const WEAK_LEVELS: &[&str] = &["off", "warn"];
const WEAK_NUMBERS: &[&str] = &["0", "1"];
const DIRECTIVE_OPTION: &str = "reportUnusedDisableDirectives";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum File {
    Config,
    Ignore,
}

pub fn file(name: &str) -> Option<File> {
    if name == IGNORE_FILE {
        return Some(File::Ignore);
    }
    let extension = name.strip_prefix(CONFIG_STEM)?;
    CONFIG_EXTENSIONS
        .contains(&extension)
        .then_some(File::Config)
}

/// `(line, message)` for every weakening setting in an eslint config or ignore file.
pub fn findings(file: File, source: &str) -> Vec<(usize, String)> {
    match file {
        File::Ignore => ignore_file(source),
        File::Config => config(source),
    }
}

fn ignore_file(source: &str) -> Vec<(usize, String)> {
    source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let entry = line.trim();
            let skipped = entry.is_empty() || entry.starts_with('#') || is_generated_output(entry);
            (!skipped).then(|| (index + 1, ignore_message(entry)))
        })
        .collect()
}

fn ignore_message(entry: &str) -> String {
    format!(
        "eslint ignore `{entry}` keeps source out of the lint; only generated or build output may be ignored"
    )
}

/// Which block of the config the scan is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    None,
    /// A `rules: {` object; `depth` counts the braces opened since.
    Rules {
        depth: i32,
    },
    /// An `ignores: [` array.
    Ignores,
}

fn config(source: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut block = Block::None;
    for (index, raw) in source.lines().enumerate() {
        let line = code(raw);
        let line_number = index + 1;
        if let Some(message) = directive_option(line) {
            found.push((line_number, message));
        }
        block = match block {
            Block::None => open_block(line, line_number, &mut found),
            Block::Rules { depth } => rules_line(line, line_number, depth, &mut found),
            Block::Ignores => ignores_line(line, line_number, &mut found),
        };
    }
    found
}

fn open_block(line: &str, line_number: usize, found: &mut Vec<(usize, String)>) -> Block {
    if let Some(rest) = after_key(line, &["rules"])
        && let Some(body) = rest.strip_prefix('{')
    {
        found.extend(rule_levels(body).map(|message| (line_number, message)));
        let depth = 1 + brace_balance(body);
        return if depth > 0 {
            Block::Rules { depth }
        } else {
            Block::None
        };
    }
    let Some(rest) = after_key(line, IGNORE_KEYS) else {
        return Block::None;
    };
    let Some(body) = rest.strip_prefix('[') else {
        return Block::None;
    };
    found.extend(ignore_entries(body).map(|message| (line_number, message)));
    if body.contains(']') {
        Block::None
    } else {
        Block::Ignores
    }
}

fn rules_line(
    line: &str,
    line_number: usize,
    depth: i32,
    found: &mut Vec<(usize, String)>,
) -> Block {
    if depth == 1 {
        found.extend(rule_levels(line).map(|message| (line_number, message)));
    }
    let depth = depth + brace_balance(line);
    if depth > 0 {
        Block::Rules { depth }
    } else {
        Block::None
    }
}

fn ignores_line(line: &str, line_number: usize, found: &mut Vec<(usize, String)>) -> Block {
    found.extend(ignore_entries(line).map(|message| (line_number, message)));
    if line.contains(']') {
        Block::None
    } else {
        Block::Ignores
    }
}

/// The text after `key:` when the line sets one of `keys`, anywhere on the line.
fn after_key<'a>(line: &'a str, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        [(*key).to_owned(), format!("'{key}'"), format!("\"{key}\"")]
            .iter()
            .find_map(|spelling| value_after(line, spelling))
    })
}

fn value_after<'a>(line: &'a str, spelling: &str) -> Option<&'a str> {
    line.match_indices(spelling).find_map(|(offset, _)| {
        let standalone = line
            .get(..offset)
            .and_then(|before| before.chars().next_back())
            .is_none_or(|character| !character.is_alphanumeric() && character != '_');
        let rest = line.get(offset + spelling.len()..)?;
        let value = rest.trim_start().strip_prefix(':')?.trim_start();
        standalone.then_some(value)
    })
}

/// Rules on this line whose level is `off`, `warn`, `0` or `1`.
fn rule_levels(line: &str) -> impl Iterator<Item = String> + '_ {
    top_level_parts(line).into_iter().filter_map(|part| {
        let (rule, value) = part.split_once(':')?;
        let level = value.trim().trim_start_matches('[');
        let level = level
            .split([',', ']', '}'])
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches(['\'', '"', '`']);
        let weak = WEAK_LEVELS.contains(&level) || WEAK_NUMBERS.contains(&level);
        let rule = rule.trim().trim_matches(['\'', '"', '{', ' ']);
        weak.then(|| {
            format!("eslint rule `{rule}` set to `{level}`; fix the findings or move the setting into @baukit/eslint-config")
        })
    })
}

/// Splits on the commas that are not nested inside brackets, braces or parentheses.
fn top_level_parts(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (offset, character) in text.char_indices() {
        match character {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.extend(text.get(start..offset));
                start = offset + 1;
            }
            _ => {}
        }
    }
    parts.extend(text.get(start..));
    parts
}

fn ignore_entries(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(',').filter_map(|part| {
        let entry = part.trim().trim_end_matches(']').trim();
        let entry = entry.trim_matches(['\'', '"', '`']);
        let skipped = entry.is_empty() || entry.starts_with("//") || is_generated_output(entry);
        (!skipped).then(|| ignore_message(entry))
    })
}

fn directive_option(line: &str) -> Option<String> {
    let rest = line.split_once(DIRECTIVE_OPTION)?.1;
    let value = rest.trim_start().strip_prefix(':')?.trim();
    let value = value
        .trim_end_matches([',', '}', ' '])
        .trim_matches(['\'', '"']);
    matches!(value, "off" | "warn" | "false" | "0" | "1").then(|| {
        format!("`{DIRECTIVE_OPTION}: {value}` lets stale eslint-disable comments pass; keep it at error")
    })
}

fn brace_balance(text: &str) -> i32 {
    text.chars().fold(0, |depth, character| match character {
        '{' => depth + 1,
        '}' => depth - 1,
        _ => depth,
    })
}

/// The line without a trailing `//` comment.
fn code(line: &str) -> &str {
    line.split_once("//")
        .filter(|(before, _)| !before.ends_with(':'))
        .map_or(line, |(before, _)| before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(file: File, source: &str) -> Vec<usize> {
        findings(file, source)
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    #[test]
    fn config_files_are_recognised() {
        assert_eq!(file("eslint.config.mjs"), Some(File::Config));
        assert_eq!(file(".eslintignore"), Some(File::Ignore));
        assert_eq!(file("eslint.config.json"), None);
        assert_eq!(file("eslintrc.js"), None);
    }

    #[test]
    fn weak_rule_levels_are_findings() {
        let source = [
            "export default [",
            "  { rules: { 'no-console': 'off' } },",
            "  {",
            "    rules: {",
            "      // Specs read the DOM: 'off' would be wrong here",
            "      'no-undef': 'off',",
            "      eqeqeq: ['warn', 'always'],",
            "      'max-params': ['error', { max: 1 }],",
            "      complexity: 0,",
            "      'no-shadow': \"error\",",
            "      '@baukit/x': [2, { allowed: 0, max: 1 }],",
            "    },",
            "    linterOptions: { reportUnusedDisableDirectives: 'warn' },",
            "  },",
            "  { files: testFiles, rules: testRelaxedRules },",
            "];",
        ]
        .join("\n");
        assert_eq!(lines(File::Config, &source), vec![2, 6, 7, 9, 13]);
    }

    #[test]
    fn ignores_may_only_name_generated_output() {
        let source = [
            "export default [",
            "  { ignores: ['dist/**', 'node_modules/**', 'coverage/**', '.expo/**', '.qa/**'] },",
            "  { ignores: ['**/.turbo/**', 'src/legacy/**'] },",
            "  {",
            "    ignores: [",
            "      'build/**',",
            "      'scripts/old.mjs',",
            "    ],",
            "  },",
            "];",
        ]
        .join("\n");
        assert_eq!(lines(File::Config, &source), vec![3, 7]);
    }

    #[test]
    fn eslintignore_entries_follow_the_same_rule() {
        let source = "# build output\ndist/\ncoverage\nsrc/generated-by-hand.ts\nmobile/.qa/\n";
        assert_eq!(lines(File::Ignore, source), vec![4]);
    }
}

use std::{fs, io, path::Path};

use anyhow::{Context, Result, bail};

use super::{CheckContext, Finding, Rule};

const NAME: &str = "agent-docs";
const CLAUDE_FILE: &str = "CLAUDE.md";
const AGENTS_FILE: &str = "AGENTS.md";
const START_MARKER: &str = "<!-- baukit:agent-rules:start -->";
const END_MARKER: &str = "<!-- baukit:agent-rules:end -->";
const AGENT_RULES: &str = include_str!("../../../lint/agent-rules.md");

pub struct AgentDocs;

impl Rule for AgentDocs {
    fn name(&self) -> &'static str {
        NAME
    }

    fn run(&self, context: &CheckContext<'_>) -> Result<Vec<Finding>> {
        if context.options.fix {
            fix(context.root, AGENT_RULES)?;
        }
        check(context.root, AGENT_RULES)
    }
}

fn managed_block(rules: &str) -> String {
    format!("{START_MARKER}\n{rules}{END_MARKER}\n")
}

fn check(root: &Path, rules: &str) -> Result<Vec<Finding>> {
    let claude = root.join(CLAUDE_FILE);
    let Some(text) = read_optional(&claude)? else {
        return Ok(vec![Finding::error(
            NAME,
            CLAUDE_FILE,
            "missing; run `baukit check agent-docs --fix`",
        )]);
    };
    let mut findings = Vec::new();
    if let Some(message) = block_problem(&text, rules) {
        findings.push(Finding::error(NAME, CLAUDE_FILE, message));
    }
    if let Some(message) = agents_problem(root)? {
        findings.push(Finding::error(NAME, AGENTS_FILE, message));
    }
    Ok(findings)
}

fn block_problem(text: &str, rules: &str) -> Option<String> {
    let starts = text.matches(START_MARKER).count();
    let ends = text.matches(END_MARKER).count();
    match (starts, ends) {
        (0, 0) => Some("managed agent-rules block is missing; run `baukit check agent-docs --fix`".to_owned()),
        (1, 1) => (block_body(text) != Some(rules)).then(|| {
            "managed agent-rules block differs from baukit's lint/agent-rules.md; run `baukit check agent-docs --fix`"
                .to_owned()
        }),
        _ => Some(format!(
            "expected exactly one managed agent-rules block, found {starts} start and {ends} end markers"
        )),
    }
}

fn block_body(text: &str) -> Option<&str> {
    let (_, after_start) = text.split_once(START_MARKER)?;
    let (body, _) = after_start.split_once(END_MARKER)?;
    body.strip_prefix('\n')
}

fn agents_problem(root: &Path) -> Result<Option<&'static str>> {
    let agents = root.join(AGENTS_FILE);
    let Ok(metadata) = fs::symlink_metadata(&agents) else {
        return Ok(Some("missing; it must be a symlink to CLAUDE.md"));
    };
    if !metadata.file_type().is_symlink() {
        return Ok(Some("must be a symlink to CLAUDE.md, not a regular file"));
    }
    let resolves = match (agents.canonicalize(), root.join(CLAUDE_FILE).canonicalize()) {
        (Ok(target), Ok(claude)) => target == claude,
        _ => false,
    };
    Ok((!resolves).then_some("symlink does not resolve to CLAUDE.md"))
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn fix(root: &Path, rules: &str) -> Result<()> {
    fix_agents_link(root)?;
    let claude = root.join(CLAUDE_FILE);
    let text = read_optional(&claude)?.unwrap_or_default();
    let updated = with_block(&text, rules)?;
    if updated != text {
        fs::write(&claude, updated)
            .with_context(|| format!("failed to write {}", claude.display()))?;
    }
    Ok(())
}

fn with_block(text: &str, rules: &str) -> Result<String> {
    let block = managed_block(rules);
    let starts = text.matches(START_MARKER).count();
    let ends = text.matches(END_MARKER).count();
    match (starts, ends) {
        (0, 0) if text.is_empty() => Ok(block),
        (0, 0) => {
            let separator = if text.ends_with("\n\n") {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            Ok(format!("{text}{separator}{block}"))
        }
        (1, 1) => {
            let (before, after_start) = text.split_once(START_MARKER).context("start marker")?;
            let (_, after_end) = after_start.split_once(END_MARKER).context("end marker")?;
            let after_end = after_end.strip_prefix('\n').unwrap_or(after_end);
            Ok(format!("{before}{block}{after_end}"))
        }
        _ => bail!(
            "{CLAUDE_FILE} has {starts} start and {ends} end markers; fix the managed block by hand"
        ),
    }
}

fn fix_agents_link(root: &Path) -> Result<()> {
    let agents = root.join(AGENTS_FILE);
    let claude = root.join(CLAUDE_FILE);
    match fs::symlink_metadata(&agents) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => {
            let claude_text = read_optional(&claude)?;
            if claude_text.as_deref().map(str::as_bytes) != Some(fs::read(&agents)?.as_slice()) {
                bail!(
                    "{AGENTS_FILE} is a regular file that differs from {CLAUDE_FILE}; merge it into {CLAUDE_FILE} and delete it first"
                );
            }
            fs::remove_file(&agents)?;
            link(&agents)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => link(&agents),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", agents.display())),
    }
}

#[cfg(unix)]
fn link(agents: &Path) -> Result<()> {
    std::os::unix::fs::symlink(CLAUDE_FILE, agents)
        .with_context(|| format!("failed to create the {AGENTS_FILE} symlink"))
}

#[cfg(not(unix))]
fn link(_agents: &Path) -> Result<()> {
    bail!("creating the {AGENTS_FILE} symlink needs a Unix host")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULES: &str = "- rule one\n- rule two\n";

    fn messages(root: &Path) -> Result<Vec<String>> {
        Ok(check(root, RULES)?
            .iter()
            .map(ToString::to_string)
            .collect())
    }

    #[test]
    fn missing_claude_md_fails() -> Result<()> {
        let root = tempfile::tempdir()?;
        assert_eq!(
            messages(root.path())?,
            vec!["agent-docs: CLAUDE.md: missing; run `baukit check agent-docs --fix`"]
        );
        Ok(())
    }

    #[test]
    fn fix_appends_the_block_and_links_agents_md() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join(CLAUDE_FILE), "# Project\n")?;
        assert_eq!(messages(root.path())?.len(), 2);
        fix(root.path(), RULES)?;
        assert!(messages(root.path())?.is_empty());
        assert_eq!(
            fs::read_to_string(root.path().join(CLAUDE_FILE))?,
            format!("# Project\n\n{START_MARKER}\n{RULES}{END_MARKER}\n")
        );
        Ok(())
    }

    #[test]
    fn fix_replaces_a_stale_block_in_place() -> Result<()> {
        let root = tempfile::tempdir()?;
        let stale = format!("# A\n\n{START_MARKER}\nold\n{END_MARKER}\n\n## B\n");
        fs::write(root.path().join(CLAUDE_FILE), stale)?;
        let found = messages(root.path())?;
        assert!(
            found.iter().any(|line| line.contains("differs from")),
            "{found:?}"
        );
        fix(root.path(), RULES)?;
        assert_eq!(
            fs::read_to_string(root.path().join(CLAUDE_FILE))?,
            format!("# A\n\n{START_MARKER}\n{RULES}{END_MARKER}\n\n## B\n")
        );
        Ok(())
    }

    #[test]
    fn duplicate_blocks_fail_and_fix_refuses() -> Result<()> {
        let root = tempfile::tempdir()?;
        let block = managed_block(RULES);
        fs::write(root.path().join(CLAUDE_FILE), format!("{block}{block}"))?;
        assert!(
            messages(root.path())?
                .iter()
                .any(|line| line.contains("exactly one"))
        );
        assert!(fix(root.path(), RULES).is_err());
        Ok(())
    }

    #[test]
    fn regular_agents_md_fails_and_fix_refuses_when_it_differs() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join(CLAUDE_FILE), managed_block(RULES))?;
        fs::write(root.path().join(AGENTS_FILE), "other\n")?;
        assert_eq!(
            messages(root.path())?,
            vec!["agent-docs: AGENTS.md: must be a symlink to CLAUDE.md, not a regular file"]
        );
        assert!(fix(root.path(), RULES).is_err());
        fs::write(root.path().join(AGENTS_FILE), managed_block(RULES))?;
        fix(root.path(), RULES)?;
        assert!(messages(root.path())?.is_empty());
        Ok(())
    }

    #[test]
    fn symlink_to_another_file_fails() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join(CLAUDE_FILE), managed_block(RULES))?;
        fs::write(root.path().join("OTHER.md"), "x")?;
        std::os::unix::fs::symlink("OTHER.md", root.path().join(AGENTS_FILE))?;
        assert_eq!(
            messages(root.path())?,
            vec!["agent-docs: AGENTS.md: symlink does not resolve to CLAUDE.md"]
        );
        Ok(())
    }
}

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, ensure};

const FINDINGS_EXIT: i32 = 1;
const USAGE_EXIT: i32 = 2;

fn baukit_check(root: &Path, args: &[&str]) -> Result<(i32, String)> {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .arg("check")
        .args(args)
        .arg("--root")
        .arg(root)
        .output()?;
    let code = status
        .code()
        .context("baukit check was killed by a signal")?;
    let mut text = String::from_utf8(stdout)?;
    text.push_str(&String::from_utf8(stderr)?);
    Ok((code, text))
}

fn write(root: &Path, path: &str, content: &str) -> Result<()> {
    let file = root.join(path);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, content)?;
    Ok(())
}

#[test]
fn clean_tree_exits_zero_with_a_summary_per_rule() -> Result<()> {
    let root = tempfile::tempdir()?;
    write(root.path(), "src/lib.rs", "pub fn ok() {}\n")?;
    let (code, output) = baukit_check(root.path(), &["file-size", "suppressions"])?;
    ensure!(code == 0, "{output}");
    assert_eq!(output, "file-size: ok\nsuppressions: ok\n");
    Ok(())
}

#[test]
fn findings_exit_one_with_one_line_each() -> Result<()> {
    let root = tempfile::tempdir()?;
    write(root.path(), "src/big.rs", &"x\n".repeat(1001))?;
    write(
        root.path(),
        "src/lib.rs",
        "#[allow(dead_code)]\nfn a() {}\n",
    )?;
    let (code, output) = baukit_check(root.path(), &["file-size", "suppressions"])?;
    ensure!(code == FINDINGS_EXIT, "{output}");
    assert_eq!(
        output,
        [
            "file-size: src/big.rs: 1001 lines exceeds the limit of 1000; split the file",
            "suppressions: src/lib.rs:1: `#[allow]` without `reason = \"...\"`",
            "suppressions: baukit-check.toml: rust-allow count 1 exceeds max 0; remove suppressions instead of raising the max",
            "file-size: 1 finding",
            "suppressions: 2 findings",
            "",
        ]
        .join("\n")
    );
    Ok(())
}

#[test]
fn unknown_rule_and_invalid_config_exit_two() -> Result<()> {
    let root = tempfile::tempdir()?;
    let (code, output) = baukit_check(root.path(), &["nope"])?;
    ensure!(code == USAGE_EXIT, "{output}");
    assert!(output.contains("unknown rule `nope`"), "{output}");

    write(
        root.path(),
        "baukit-check.toml",
        "[file_size]\nrust-max-lines = 2000\n",
    )?;
    let (code, output) = baukit_check(root.path(), &["file-size"])?;
    ensure!(code == USAGE_EXIT, "{output}");
    assert!(output.contains("may only be lowered"), "{output}");
    Ok(())
}

#[test]
fn agent_docs_fix_makes_the_tree_pass() -> Result<()> {
    let root = tempfile::tempdir()?;
    write(root.path(), "CLAUDE.md", "# Product\n")?;
    let (code, output) = baukit_check(root.path(), &["agent-docs"])?;
    ensure!(code == FINDINGS_EXIT, "{output}");
    let (code, output) = baukit_check(root.path(), &["agent-docs", "--fix"])?;
    ensure!(code == 0, "{output}");
    assert_eq!(output, "agent-docs: ok\n");
    let agents = fs::read_link(root.path().join("AGENTS.md"))?;
    assert_eq!(agents, PathBuf::from("CLAUDE.md"));
    Ok(())
}

#[test]
fn agent_docs_fix_refuses_a_differing_agents_md() -> Result<()> {
    let root = tempfile::tempdir()?;
    write(root.path(), "CLAUDE.md", "# Product\n")?;
    write(root.path(), "AGENTS.md", "# Other\n")?;
    let (code, output) = baukit_check(root.path(), &["agent-docs", "--fix"])?;
    ensure!(code == USAGE_EXIT, "{output}");
    assert!(output.contains("differs from CLAUDE.md"), "{output}");
    assert_eq!(
        fs::read_to_string(root.path().join("CLAUDE.md"))?,
        "# Product\n"
    );
    Ok(())
}

#[test]
fn patterns_fail_on_errors_and_show_hints_only_when_verbose() -> Result<()> {
    let root = tempfile::tempdir()?;
    write(
        root.path(),
        "web/src/cast.ts",
        "export const value = input as unknown as string;\n",
    )?;
    write(
        root.path(),
        "backend/src/repo.rs",
        "pub const SQL: &str = \"SELECT name FROM items WHERE id = $1\";\n",
    )?;
    write(
        root.path(),
        "vendor/x/cast.ts",
        "export const value = input as unknown as string;\n",
    )?;
    write(
        root.path(),
        "web/src/ignored.ts",
        "// ast-grep-ignore: ts-double-cast\nexport const other = input as unknown as number;\n",
    )?;
    let (code, output) = baukit_check(root.path(), &["patterns"])?;
    ensure!(code == FINDINGS_EXIT, "{output}");
    assert!(
        output.contains("patterns: web/src/cast.ts:1: ts-double-cast: "),
        "{output}"
    );
    assert!(
        output.contains("patterns: web/src/ignored.ts:1: `ast-grep-ignore` without ` -- reason`"),
        "{output}"
    );
    assert!(!output.contains("vendor/"), "{output}");
    assert!(!output.contains("rust-sql-unscoped-by-id"), "{output}");
    assert!(output.ends_with("patterns: 2 findings\n"), "{output}");

    let (_, verbose) = baukit_check(root.path(), &["patterns", "--verbose"])?;
    assert!(
        verbose.contains("patterns: backend/src/repo.rs:1: note: rust-sql-unscoped-by-id: "),
        "{verbose}"
    );
    assert!(verbose.ends_with("patterns: 2 findings\n"), "{verbose}");
    Ok(())
}

#[test]
fn suppressions_update_rewrites_the_max_downward() -> Result<()> {
    let root = tempfile::tempdir()?;
    write(
        root.path(),
        "baukit-check.toml",
        "[suppressions.max]\nrust-expect = 5\n",
    )?;
    write(
        root.path(),
        "src/lib.rs",
        "#[expect(dead_code, reason = \"kept for the doc example\")]\nfn a() {}\n",
    )?;
    let (code, output) = baukit_check(root.path(), &["suppressions"])?;
    ensure!(code == 0, "{output}");
    assert!(output.contains("lower max to 1"), "{output}");
    let (code, output) = baukit_check(root.path(), &["suppressions", "--update"])?;
    ensure!(code == 0, "{output}");
    assert_eq!(
        fs::read_to_string(root.path().join("baukit-check.toml"))?,
        "[suppressions.max]\nrust-expect = 1\n"
    );
    Ok(())
}

#[test]
fn baukit_repository_passes_its_own_checks() -> Result<()> {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let (code, output) = baukit_check(&repository, &["file-size", "suppressions", "agent-docs"])?;
    ensure!(code == 0, "{output}");
    assert!(
        output.ends_with("file-size: ok\nsuppressions: ok\nagent-docs: ok\n"),
        "{output}"
    );
    Ok(())
}

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, ensure};

const PYTHON_FINDINGS_EXIT: i32 = 1;

fn repository_root() -> Result<PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("cli has a parent directory")?
        .to_path_buf())
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let status = Command::new("cp")
        .arg("-R")
        .arg(format!("{}/.", source.display()))
        .arg(target)
        .status()?;
    ensure!(status.success(), "copying {} failed", source.display());
    Ok(())
}

fn python_problems(root: &Path) -> Result<(i32, Vec<String>)> {
    let script = repository_root()?.join("deploy/observability/lint/check-metric-names.py");
    let mut command = Command::new("python3");
    command
        .arg(script)
        .args(["--observability-root", "."])
        .current_dir(root);
    if root.join("product-metrics.txt").is_file() {
        command.args(["--allowlist", "product-metrics.txt"]);
    }
    let output = command.output()?;
    let code = output.status.code().context("python3 was killed")?;
    let mut problems: Vec<String> = String::from_utf8(output.stderr)?
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(str::to_owned)
        .collect();
    problems.sort();
    Ok((code, problems))
}

fn rust_problems(root: &Path) -> Result<(i32, Vec<String>)> {
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .args(["check", "observability", "--root"])
        .arg(root)
        .output()?;
    let code = output.status.code().context("baukit was killed")?;
    let mut problems: Vec<String> = String::from_utf8(output.stdout)?
        .lines()
        .filter_map(|line| line.strip_prefix("observability: "))
        .filter(|line| *line != "ok" && !line.ends_with(" findings") && !line.ends_with(" finding"))
        .map(str::to_owned)
        .collect();
    problems.sort();
    Ok((code, problems))
}

#[test]
fn rust_rule_and_check_metric_names_py_agree_on_the_shared_fixture() -> Result<()> {
    let root = tempfile::tempdir()?;
    copy_tree(
        &repository_root()?.join("cli/tests/fixtures/observability"),
        root.path(),
    )?;
    let (python_code, python) = python_problems(root.path())?;
    let (rust_code, rust) = rust_problems(root.path())?;
    ensure!(
        python_code == PYTHON_FINDINGS_EXIT,
        "python exited {python_code}"
    );
    ensure!(
        rust_code == PYTHON_FINDINGS_EXIT,
        "baukit exited {rust_code}"
    );
    ensure!(
        python.len() >= 10,
        "fixture lost its violations: {python:?}"
    );
    assert_eq!(rust, python);
    Ok(())
}

#[test]
fn rust_rule_and_check_metric_names_py_agree_on_baukits_observability() -> Result<()> {
    let root = tempfile::tempdir()?;
    copy_tree(
        &repository_root()?.join("deploy/observability"),
        root.path(),
    )?;
    std::fs::write(
        root.path().join("baukit-check.toml"),
        "[observability]\ndashboards = [\"dashboards\"]\nrules = [\"alerts\", \"recording-rules\"]\n",
    )?;
    let (python_code, python) = python_problems(root.path())?;
    let (rust_code, rust) = rust_problems(root.path())?;
    assert_eq!((rust_code, rust), (python_code, python));
    ensure!(python_code == 0, "baukit's observability must pass");
    Ok(())
}

fn write(root: &Path, path: &str, content: &str) -> Result<()> {
    let file = root.join(path);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, content)?;
    Ok(())
}

#[test]
fn without_configuration_the_rule_notes_and_passes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_baukit"))
        .args(["check", "observability", "--root"])
        .arg(root.path())
        .output()?;
    ensure!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "observability: baukit-check.toml: note: not configured; add an [observability] table to check dashboards and rules\nobservability: ok\n"
    );
    Ok(())
}

#[test]
fn product_checks_cover_emitted_metrics_naming_and_datasources() -> Result<()> {
    let root = tempfile::tempdir()?;
    write(
        root.path(),
        "baukit-check.toml",
        "[observability]\ndashboards = [\"ops/dashboards\"]\nrules = [\"ops/rules\", \"ops/missing.yml\"]\nsources = [\"backend\"]\ndatasource-uids = [\"prometheus\"]\nmetric-prefix = \"shop_\"\n",
    )?;
    write(
        root.path(),
        "backend/src/metrics.rs",
        "pub fn record() {\n    metrics::counter!(\"shop_orders_total\").increment(1);\n    metrics::gauge!(\"shop_backlog_total\").set(1.0);\n    metrics::histogram!(\"orders_latency\").record(1.0);\n    metrics::counter!(\"http_requests_total\").increment(1);\n}\n",
    )?;
    write(
        root.path(),
        "ops/dashboards/shop.json",
        r#"{"panels": [
  {"datasource": {"uid": "prometheus"}, "targets": [{"expr": "rate(shop_orders_total[5m]) + orders_latency_count"}]},
  {"datasource": {"uid": "${DS_PROMETHEUS}"}, "targets": [{"expr": "shop:orders:rate5m"}]},
  {"datasource": {"uid": "grafanacloud-prom"}, "targets": []}
]}"#,
    )?;
    write(
        root.path(),
        "ops/rules/shop.yml",
        "groups:\n  - name: shop\n    rules:\n      - record: shop:orders:rate5m\n        expr: sum(rate(shop_orders_total[5m]))\n      - record: shop_orders_total\n        expr: sum(shop_orders_total)\n",
    )?;
    let (code, problems) = rust_problems(root.path())?;
    ensure!(code == PYTHON_FINDINGS_EXIT, "baukit exited {code}");
    let mut expected = vec![
        "backend/src/metrics.rs:3: gauge `shop_backlog_total` must not end with `_total`",
        "backend/src/metrics.rs:3: emitted metric `shop_backlog_total` appears on no dashboard",
        "backend/src/metrics.rs:4: product metric `orders_latency` must start with `shop_`",
        "ops/dashboards/shop.json.panels[1].datasource: datasource variable `DS_PROMETHEUS` is not declared in templating or __inputs",
        "ops/dashboards/shop.json.panels[2].datasource: datasource uid `grafanacloud-prom` is neither a datasource variable nor in observability.datasource-uids",
        "ops/missing.yml: configured in [observability] but does not exist",
        "ops/rules/shop.yml:7: recording rule 'shop_orders_total' shadows an emitted metric",
    ];
    expected.sort_unstable();
    assert_eq!(problems, expected);
    Ok(())
}

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

import yaml
from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]


def load(name: str):
    spec = importlib.util.spec_from_file_location(
        name.replace("-", "_"), ROOT / "scripts" / f"{name}.py"
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


release_shape = load("check-release-shape")
list_proofs = load("list-proofs")
migrations = load("check-migrations")
vectors = load("regenerate-vectors")

UNGATED_RELEASE = """
on: push
permissions:
  contents: read
jobs:
  publish:
    runs-on: ubuntu-latest
    permissions:
      id-token: write
    steps:
      - run: pnpm install --frozen-lockfile
      - run: pnpm build
      - run: cargo build --release
      - run: npm install --global npm@12.2.2
"""

GATED_RELEASE = """
on: push
jobs:
  on-main:
    runs-on: ubuntu-latest
    steps:
      - run: git merge-base --is-ancestor "$GITHUB_SHA" origin/main
  build:
    needs: on-main
    runs-on: ubuntu-latest
    steps:
      - run: pnpm install --frozen-lockfile
  publish:
    needs: build
    environment: release
    runs-on: ubuntu-latest
    permissions:
      id-token: write
    steps:
      - run: npm install --global npm@12.2.2
      - run: scripts/publish-packages.sh
"""


class ReleaseShapeTest(unittest.TestCase):
    def test_flags_every_gap_in_an_ungated_publish_job(self) -> None:
        violations = release_shape.job_violations(yaml.safe_load(UNGATED_RELEASE), "release.yml")

        self.assertEqual(len(violations), 5)
        joined = "\n".join(violations)
        for expected in (
            "without an environment",
            "`pnpm install`",
            "`pnpm build`",
            "`cargo build`",
            "merge-base",
        ):
            self.assertIn(expected, joined)
        self.assertNotIn("npm install --global", joined)

    def test_accepts_a_gated_publish_only_job(self) -> None:
        self.assertEqual(
            release_shape.job_violations(yaml.safe_load(GATED_RELEASE), "release.yml"), []
        )

    def test_workflow_level_id_token_applies_to_jobs_without_permissions(self) -> None:
        workflow = {"permissions": {"id-token": "write"}, "jobs": {"any": {"steps": []}}}

        self.assertEqual(len(release_shape.job_violations(workflow, "ci.yml")), 2)

    def test_old_release_workflow_fails_and_current_one_passes(self) -> None:
        old = run_process(
            ["git", "show", "44eba6a4:.github/workflows/release.yml"],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        if old.returncode == 0:
            self.assertTrue(release_shape.job_violations(yaml.safe_load(old.stdout), "old"))
        self.assertEqual(release_shape.main([str(ROOT / ".github" / "workflows")]), 0)


HARNESSES = """
#[cfg(kani)]
mod proofs {
    /// K-11: admission never exceeds capacity.
    ///
    /// Bounds: unwind 4, cost <= 8.
    #[kani::proof]
    #[kani::unwind(4)]
    fn admit_bounded() {}

    /// Refill is monotone. No panic. Bounds: every `u64`, no loops.
    #[kani::proof]
    fn inline_bounds() {}

    #[kani::proof]
    fn missing_bounds() {}
}
"""


class ListProofsTest(unittest.TestCase):
    def test_reads_plan_ids_and_bounds_and_flags_missing_ones(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "crates" / "baukit-ratelimit" / "src" / "bucket.rs"
            source.parent.mkdir(parents=True)
            source.write_text(HARNESSES, encoding="utf-8")

            harnesses = list_proofs.collect(root)

            self.assertEqual(
                [(h.crate, h.name, h.plan_id, h.bounds) for h in harnesses],
                [
                    ("baukit-ratelimit", "admit_bounded", "K-11", "unwind 4, cost <= 8."),
                    ("baukit-ratelimit", "inline_bounds", None, "every `u64`, no loops."),
                    ("baukit-ratelimit", "missing_bounds", None, None),
                ],
            )
            table = list_proofs.table(harnesses)
            admit_row = "| K-11 | baukit-ratelimit | `admit_bounded` | unwind 4, cost <= 8. |"
            self.assertIn(admit_row, table)
            self.assertIn("| - | baukit-ratelimit | `missing_bounds` | missing |", table)
            self.assertEqual(list_proofs.main(["--check", "--root", str(root)]), 1)


class MigrationOrderTest(unittest.TestCase):
    def test_repository_order_is_consistent(self) -> None:
        plan = migrations.load_plan()

        self.assertEqual(migrations.order_errors(plan, ROOT / "rust" / "crates"), [])

    def test_rejects_a_crate_before_its_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            crates = Path(directory)
            for name in ("baukit-jobs", "baukit-erasure", "baukit-extra"):
                (crates / name / "migrations").mkdir(parents=True)
                (crates / name / "migrations" / "0001.sql").write_text(
                    "SELECT 1;\n", encoding="utf-8"
                )
            plan = migrations.Plan(
                [
                    {"name": "baukit-erasure", "after": ["baukit-jobs"]},
                    {"name": "baukit-jobs", "after": []},
                ],
                frozenset(),
                "postgres",
            )

            errors = migrations.order_errors(plan, crates)

        self.assertIn("baukit-erasure must come after baukit-jobs", errors)
        self.assertIn("baukit-extra has migrations but is not listed", errors)


class DoubleApplyTest(unittest.TestCase):
    def test_second_pass_reruns_only_files_newer_than_the_shipped_baseline(self) -> None:
        plan = migrations.load_plan()
        shipped = next(iter(plan.shipped))
        files = plan.files()
        newer = [path for path in files if not plan.is_shipped(path)]

        reruns = migrations.rerun_files(plan)

        self.assertEqual(reruns, newer)
        self.assertNotIn(migrations.ROOT / shipped, reruns)

    def test_schema_comparison_ignores_pg_dump_restrict_keys(self) -> None:
        first = "\\restrict abc\nCREATE TABLE t ();\n\\unrestrict abc\n"
        second = "\\restrict xyz\nCREATE TABLE t ();\n\\unrestrict xyz\n"

        self.assertEqual(migrations.normalized_schema(first), migrations.normalized_schema(second))
        self.assertNotEqual(
            migrations.normalized_schema(first), migrations.normalized_schema("CREATE TABLE u ();")
        )


class ModelsStepTest(unittest.TestCase):
    def test_models_step_hands_an_explicit_jar_to_models_check(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            jar = Path(directory) / "missing.jar"
            result = run_process(
                ["sh", str(ROOT / "scripts" / "ci-gate.sh"), "--step", "models"],
                env={"PATH": "/usr/bin:/bin", "TLA2TOOLS_JAR": str(jar), "TLC_WORKERS": "1"},
                capture_output=True,
                text=True,
                timeout=120,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("install-deploy-tools", result.stdout + result.stderr)
        self.assertIn("FAIL", result.stdout)


class VectorRegistryTest(unittest.TestCase):
    def test_every_generated_vector_needs_one_generator(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "fixtures" / "hlc").mkdir(parents=True)
            (root / "fixtures" / "hlc" / "generated-v1.json").write_text("{}", encoding="utf-8")

            self.assertEqual(
                vectors.registry_errors([], root),
                ["fixtures/hlc/generated-v1.json has no generator in vector-generators.toml"],
            )
            self.assertEqual(
                vectors.registry_errors([{"output": "fixtures/hlc/generated-v1.json"}], root), []
            )


if __name__ == "__main__":
    unittest.main()

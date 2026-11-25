import re
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "templates/common/__strict__/scripts/quality-gate.sh"
FAKE_GIT = """#!/bin/sh
printf '%s\\n' "$*" >> "$GIT_LOG"
[ -n "${GIT_FAILS:-}" ] && exit 128
for target; do :; done
mkdir -p "$target/cli"
"""
FAKE_UV = '#!/bin/sh\nprintf \'%s\\n\' "$*" > "$UV_ARGS"\n'


def gate_function(name):
    match = re.search(rf"^{name}\(\) \{{\n.*?^\}}\n", GATE.read_text(), re.M | re.S)
    assert match, f"{name} is missing from the strict gate"
    return match.group(0)


def deploy_values_python():
    return gate_function("deploy_values_python")


class DeployValuesPythonTests(unittest.TestCase):
    def uv_args(self, *product_files):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in product_files:
                (root / name).write_text("")
            bin_dir = root / "bin"
            bin_dir.mkdir()
            (bin_dir / "uv").write_text(FAKE_UV)
            (bin_dir / "uv").chmod(0o755)
            args_file = root / "args"
            result = run_process(
                ["sh", "-c", deploy_values_python() + "deploy_values_python check.py . chart"],
                capture_output=True,
                text=True,
                check=False,
                cwd=root,
                env={"PATH": f"{bin_dir}:/usr/bin:/bin", "UV_ARGS": str(args_file)},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            return args_file.read_text().split()

    def test_a_locked_uv_project_runs_its_own_environment(self):
        self.assertEqual(
            self.uv_args("pyproject.toml", "uv.lock"),
            ["run", "--locked", "--project", ".", "python3", "check.py", ".", "chart"],
        )

    def test_a_product_without_a_uv_lock_gets_a_pinned_pyyaml(self):
        for files in ((), ("pyproject.toml",)):
            self.assertEqual(
                self.uv_args(*files),
                [
                    "run",
                    "--no-project",
                    "--with",
                    "PyYAML==6.0.3",
                    "python3",
                    "check.py",
                    ".",
                    "chart",
                ],
                files,
            )


class RequireNodeModulesTests(unittest.TestCase):
    def check(self, installed):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "web").mkdir()
            if installed:
                (root / "web/node_modules").mkdir()
            return run_process(
                [
                    "sh",
                    "-c",
                    gate_function("require_node_modules") + "require_node_modules web web-install",
                ],
                capture_output=True,
                text=True,
                check=False,
                cwd=root,
            )

    def test_installed_modules_pass(self):
        self.assertEqual(self.check(installed=True).returncode, 0)

    def test_missing_modules_name_the_install_step(self):
        result = self.check(installed=False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("web/node_modules is missing", result.stderr)
        self.assertIn("--step web-install", result.stderr)


def cached_checkout_script():
    return (
        "baukit_remote=https://example.invalid/baukit.git\n"
        "baukit_clone_retry_seconds=0\n"
        + gate_function("clone_baukit_tag")
        + gate_function("move_into_place")
        + gate_function("cached_baukit_checkout")
        + "cached_baukit_checkout v1.2.3"
    )


def without_cache_lookup(script):
    lookup = '  if [ -d "$checkout_dir" ]; then\n    printf'
    assert lookup in script
    return script.replace(lookup, "  if false; then\n    printf", 1)


class CachedBaukitCheckoutTests(unittest.TestCase):
    def run_checkout(self, root, git_fails=False, script=None):
        bin_dir = root / "bin"
        bin_dir.mkdir()
        (bin_dir / "git").write_text(FAKE_GIT)
        (bin_dir / "git").chmod(0o755)
        env = {
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "XDG_CACHE_HOME": str(root / "cache"),
            "GIT_LOG": str(root / "git.log"),
        }
        if git_fails:
            env["GIT_FAILS"] = "1"
        return run_process(
            ["sh", "-c", script or cached_checkout_script()],
            capture_output=True,
            text=True,
            check=False,
            cwd=root,
            env=env,
        )

    def git_calls(self, root):
        log = root / "git.log"
        return log.read_text().splitlines() if log.exists() else []

    def test_an_existing_checkout_is_reused_without_git(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            checkout = root / "cache/baukit/checkouts/v1.2.3"
            checkout.mkdir(parents=True)
            result = self.run_checkout(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), str(checkout))
            self.assertEqual(self.git_calls(root), [])

    def test_a_missing_checkout_is_cloned_beside_the_target_and_moved_in(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            checkouts = root / "cache/baukit/checkouts"
            result = self.run_checkout(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), str(checkouts / "v1.2.3"))
            self.assertTrue((checkouts / "v1.2.3/cli").is_dir())
            self.assertEqual([path.name for path in checkouts.iterdir()], ["v1.2.3"])
            [call] = self.git_calls(root)
            self.assertIn("--branch v1.2.3 --depth 1 https://example.invalid/baukit.git", call)
            self.assertIn(str(checkouts / "v1.2.3.tmp."), call)

    def test_a_checkout_that_appeared_meanwhile_wins_over_the_staged_clone(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            checkouts = root / "cache/baukit/checkouts"
            (checkouts / "v1.2.3").mkdir(parents=True)
            (checkouts / "v1.2.3/marker").write_text("")
            result = self.run_checkout(root, script=without_cache_lookup(cached_checkout_script()))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((checkouts / "v1.2.3/marker").exists())
            self.assertFalse((checkouts / "v1.2.3/checkout").exists())
            self.assertEqual([path.name for path in checkouts.iterdir()], ["v1.2.3"])

    def test_an_unreachable_remote_is_retried_once_then_named(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.run_checkout(root, git_fails=True)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(len(self.git_calls(root)), 2)
            self.assertIn(
                "baukit remote https://example.invalid/baukit.git was unreachable", result.stderr
            )
            self.assertIn(str(root / "cache/baukit/checkouts/v1.2.3"), result.stderr)
            self.assertEqual(list((root / "cache/baukit/checkouts").iterdir()), [])


if __name__ == "__main__":
    unittest.main()

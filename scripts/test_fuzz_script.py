import re
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
FAKE_CARGO = """#!/bin/sh
case "$*" in
  *"fuzz list"*) echo parser ;;
  *"fuzz run"*) printf '%s\\n' "$*" > "$FUZZ_ARGS" ;;
esac
"""
FAKE_RUSTC = '#!/bin/sh\necho "host: x86_64-unknown-linux-gnu"\n'
RENDEZVOUS_SECONDS = 10
# Each run records its argv, waits for its partner when PARTNERS is set, then fails if listed.
FAKE_PARALLEL_CARGO = f"""#!/bin/sh
case "$*" in
  *"fuzz list"*) printf '%s\\n' $FUZZ_TARGETS; exit 0 ;;
  *"fuzz run"*) ;;
  *) exit 0 ;;
esac
name=$6
printf '%s\\n' "$*" > "$FUZZ_ARGS/$name"
: > "$FUZZ_ARGS/$name.started"
for partner in ${{PARTNERS:-}}; do
  [ "$partner" = "$name" ] && continue
  waited=0
  until [ -e "$FUZZ_ARGS/$partner.started" ]; do
    [ "$waited" -ge {RENDEZVOUS_SECONDS} ] && exit 3
    sleep 1
    waited=$((waited + 1))
  done
done
echo "log of $name"
case " $FUZZ_FAIL " in *" $name "*) exit 1 ;; esac
exit 0
"""
TEMPLATE_GATE = ROOT / "templates/common/__strict__/scripts/quality-gate.sh"
TEMPLATE_FUZZ_FUNCTIONS = ("fuzz_target", "print_fuzz_log", "fuzz_worker", "step_fuzz")


def template_function(name):
    match = re.search(rf"^{name}\(\) \{{\n.*?^\}}\n", TEMPLATE_GATE.read_text(), re.M | re.S)
    assert match, f"{name} is missing from the strict gate"
    return match.group(0)


def install_fakes(root, cargo):
    bin_dir = root / "bin"
    bin_dir.mkdir()
    for name, body in (("cargo", cargo), ("rustc", FAKE_RUSTC)):
        (bin_dir / name).write_text(body)
        (bin_dir / name).chmod(0o755)
    return bin_dir


class ParallelFuzzCase(unittest.TestCase):
    def assert_target_args(self, args_dir, name):
        args = (args_dir / name).read_text().split()
        corpora = args[args.index(name) + 1 : args.index("--")]
        self.assertEqual(corpora[1], f"corpus/{name}", args)
        self.assertNotEqual(corpora[0], f"corpus/{name}", args)
        self.assertIn(f"-artifact_prefix=artifacts/{name}/", args)
        return args

    def fake_env(self, root, jobs, partners="", fail=""):
        args_dir = root / "args"
        args_dir.mkdir()
        bin_dir = install_fakes(root, FAKE_PARALLEL_CARGO)
        env = {
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "FUZZ_ARGS": str(args_dir),
            "FUZZ_TARGETS": "alpha beta",
            "PARTNERS": partners,
            "FUZZ_FAIL": fail,
            "BAUKIT_FUZZ_JOBS": str(jobs),
        }
        return env, args_dir


class ParallelFuzzScriptTests(ParallelFuzzCase):
    def run_all(self, jobs, partners="", fail=""):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "scripts").mkdir()
        (root / "rust/fuzz").mkdir(parents=True)
        (root / "scripts/fuzz.sh").write_text((ROOT / "scripts/fuzz.sh").read_text())
        env, args_dir = self.fake_env(root, jobs, partners, fail)
        result = run_process(
            ["sh", str(root / "scripts/fuzz.sh"), "all", "7"],
            capture_output=True,
            text=True,
            check=False,
            env=env,
        )
        return result, args_dir

    def test_two_jobs_run_both_targets_at_once_with_their_own_arguments(self):
        result, args_dir = self.run_all(2, partners="alpha beta")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in ("alpha", "beta"):
            args = self.assert_target_args(args_dir, name)
            self.assertIn("-max_total_time=7", args)
            self.assertIn("-print_final_stats=1", args)
            self.assertIn(f"fuzz.sh: {name} for 7s\nlog of {name}\n", result.stdout)
        self.assertIn("fuzz.sh: no crashes", result.stdout)

    def test_a_failing_target_is_named_and_does_not_stop_the_other(self):
        result, args_dir = self.run_all(2, partners="alpha beta", fail="alpha")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("fuzz.sh: crashes in: alpha (", result.stderr)
        self.assertIn("log of beta", result.stdout)
        self.assertTrue((args_dir / "beta").exists())

    def test_one_job_runs_the_targets_in_order(self):
        result, args_dir = self.run_all(1, fail="beta")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("fuzz.sh: crashes in: beta (", result.stderr)
        self.assertLess(result.stdout.index("log of alpha"), result.stdout.index("fuzz.sh: beta"))
        for name in ("alpha", "beta"):
            self.assert_target_args(args_dir, name)

    def test_jobs_must_be_a_positive_integer(self):
        for jobs in ("0", "two"):
            result, _ = self.run_all(jobs)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("BAUKIT_FUZZ_JOBS must be a positive integer", result.stderr)


class TemplateParallelFuzzTests(ParallelFuzzCase):
    def run_step(self, jobs, partners="", fail=""):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "backend/fuzz").mkdir(parents=True)
        (root / "backend/Cargo.lock").write_text("")
        env, args_dir = self.fake_env(root, jobs, partners, fail)
        env["BAUKIT_FUZZ_SECONDS"] = "7"
        script = "set -eu\nworkers=1\n" + "".join(map(template_function, TEMPLATE_FUZZ_FUNCTIONS))
        result = run_process(
            ["sh", "-c", script + "step_fuzz"],
            capture_output=True,
            text=True,
            check=False,
            cwd=root,
            env=env,
        )
        return result, args_dir

    def test_two_jobs_run_both_targets_at_once(self):
        result, args_dir = self.run_step(2, partners="alpha beta")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in ("alpha", "beta"):
            self.assertIn("-max_total_time=7", self.assert_target_args(args_dir, name))
            self.assertIn(f"fuzz {name} for 7s\nlog of {name}\n", result.stdout)

    def test_a_failing_target_is_named_and_does_not_stop_the_other(self):
        result, args_dir = self.run_step(2, partners="alpha beta", fail="alpha")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("fuzz crashes in: alpha (", result.stderr)
        self.assertTrue((args_dir / "beta").exists())

    def test_one_job_keeps_running_after_a_crash(self):
        result, args_dir = self.run_step(1, fail="alpha")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("fuzz crashes in: alpha (", result.stderr)
        self.assert_target_args(args_dir, "beta")


class FuzzScriptTests(unittest.TestCase):
    def run_script(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            (root / "rust/fuzz").mkdir(parents=True)
            (root / "scripts/fuzz.sh").write_text((ROOT / "scripts/fuzz.sh").read_text())
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for name, body in (("cargo", FAKE_CARGO), ("rustc", FAKE_RUSTC)):
                (bin_dir / name).write_text(body)
                (bin_dir / name).chmod(0o755)
            args_file = root / "args"
            env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "FUZZ_ARGS": str(args_file)}
            result = run_process(
                ["sh", str(root / "scripts/fuzz.sh"), "parser", "1"],
                capture_output=True,
                text=True,
                check=False,
                env=env,
            )
            args = args_file.read_text().split() if args_file.exists() else []
            return result, args

    def test_new_inputs_go_to_a_scratch_directory_before_the_seed_directory(self):
        result, args = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        corpora = args[args.index("parser") + 1 : args.index("--")]
        self.assertEqual(len(corpora), 2, args)
        self.assertNotEqual(corpora[0], "corpus/parser")
        self.assertEqual(corpora[1], "corpus/parser")

    def test_template_fuzz_step_lists_the_seed_directory_last(self):
        script = (ROOT / "templates/common/__strict__/scripts/quality-gate.sh").read_text()
        self.assertIn('"$fuzz_scratch/$target" "corpus/$target"', script)

    def resolve_template_fuzz_scratch(self, log_dir):
        script = (ROOT / "templates/common/__strict__/scripts/quality-gate.sh").read_text()
        assignment = next(
            line.strip() for line in script.splitlines() if line.strip().startswith("fuzz_scratch=")
        )
        env = {"PATH": "/usr/bin:/bin"}
        if log_dir is not None:
            env["BAUKIT_GATE_LOG_DIR"] = log_dir
        with tempfile.TemporaryDirectory() as directory:
            result = run_process(
                ["sh", "-c", f'{assignment}; printf %s "$fuzz_scratch"'],
                capture_output=True,
                text=True,
                check=True,
                cwd=directory,
                env=env,
            )
            return Path(directory).resolve(), result.stdout

    def test_template_fuzz_scratch_keeps_an_absolute_gate_log_dir(self):
        _, scratch = self.resolve_template_fuzz_scratch("/var/gate-logs")
        self.assertEqual(scratch, "/var/gate-logs/fuzz-corpus")

    def test_template_fuzz_scratch_defaults_under_the_working_directory(self):
        cwd, scratch = self.resolve_template_fuzz_scratch(None)
        self.assertEqual(Path(scratch).resolve(), cwd / ".quality-gate/fuzz-corpus")


if __name__ == "__main__":
    unittest.main()

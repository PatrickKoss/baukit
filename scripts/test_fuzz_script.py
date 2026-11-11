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

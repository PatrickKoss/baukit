#!/usr/bin/env python3

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).with_name("process_runner.py")
SPEC = importlib.util.spec_from_file_location("process_runner", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
run_process = MODULE.run_process
start_process = MODULE.start_process

SLEEP_SECONDS = 60
TIMEOUT_SECONDS = 0.5
FAILURE_CODE = 3


def python(code: str) -> list[str]:
    return [sys.executable, "-c", code]


class RunProcessTest(unittest.TestCase):
    def test_returns_the_completed_process(self) -> None:
        completed = run_process(python("pass"))

        self.assertEqual(completed.returncode, 0)
        self.assertIsNone(completed.stdout)

    def test_check_raises_on_a_failing_exit(self) -> None:
        code = f"raise SystemExit({FAILURE_CODE})"

        with self.assertRaises(subprocess.CalledProcessError) as raised:
            run_process(python(code), check=True)

        self.assertEqual(raised.exception.returncode, FAILURE_CODE)

    def test_a_failing_exit_is_returned_without_check(self) -> None:
        completed = run_process(python(f"raise SystemExit({FAILURE_CODE})"))

        self.assertEqual(completed.returncode, FAILURE_CODE)

    def test_capture_output_with_text_returns_strings(self) -> None:
        code = "import sys; print('out'); print('err', file=sys.stderr)"

        completed = run_process(python(code), capture_output=True, text=True)

        self.assertEqual(completed.stdout, "out\n")
        self.assertEqual(completed.stderr, "err\n")

    def test_check_carries_the_captured_output(self) -> None:
        code = f"print('boom'); raise SystemExit({FAILURE_CODE})"

        with self.assertRaises(subprocess.CalledProcessError) as raised:
            run_process(python(code), check=True, capture_output=True, text=True)

        self.assertEqual(raised.exception.output, "boom\n")

    def test_input_reaches_stdin(self) -> None:
        code = "import sys; print(sys.stdin.read().upper(), end='')"

        completed = run_process(python(code), input="hello", capture_output=True, text=True)

        self.assertEqual(completed.stdout, "HELLO")

    def test_timeout_kills_the_child_and_raises(self) -> None:
        code = f"import time; time.sleep({SLEEP_SECONDS})"

        with self.assertRaises(subprocess.TimeoutExpired):
            run_process(python(code), timeout=TIMEOUT_SECONDS)

    def test_a_missing_executable_exits_with_a_message(self) -> None:
        with self.assertRaises(SystemExit) as raised:
            run_process(["baukit-no-such-executable"])

        self.assertEqual(str(raised.exception), "baukit-no-such-executable is not on PATH")


class StartProcessTest(unittest.TestCase):
    def test_returns_a_live_popen(self) -> None:
        code = f"import time; time.sleep({SLEEP_SECONDS})"

        process = start_process(python(code))
        try:
            self.assertIsInstance(process, subprocess.Popen)
            self.assertIsNone(process.poll())
        finally:
            process.kill()
            process.wait()

    def test_passes_options_to_popen(self) -> None:
        process = start_process(python("print('piped')"), stdout=subprocess.PIPE, text=True)
        with process:
            self.assertEqual(process.communicate()[0], "piped\n")

    def test_a_missing_executable_exits_with_a_message(self) -> None:
        with self.assertRaises(SystemExit):
            start_process(["baukit-no-such-executable"])


if __name__ == "__main__":
    unittest.main()

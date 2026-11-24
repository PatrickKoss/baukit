import unittest

import test_android_qa as qa

FAKE_MAESTRO = r"""
n=$(($(cat "$COUNT" 2>/dev/null || echo 0) + 1))
echo "$n" > "$COUNT"
echo "maestro $*" >> "$EVENTS"
offline='Command failed (host:transport:emulator-5800): device offline'
case "$MODE:$n" in
  retry:1 | twice:*) printf 'Picked up JAVA_TOOL_OPTIONS: x\n\n%s\n' "$offline"; exit 1 ;;
  started:*) printf '> Flow a-first\n%s\n' "$offline"; exit 1 ;;
esac
"""
OFFLINE = "qa: foreign emulator-5800 went offline on the shared adb server"


class MaestroForeignRetryTests(unittest.TestCase):
    setUp = qa.AndroidQaTests.setUp
    executable = qa.AndroidQaTests.executable
    run_script = qa.AndroidQaTests.run_script
    sharded = qa.AndroidQaTests.sharded

    def run_mode(self, mode):
        result = self.sharded(FAKE_MAESTRO, MODE=mode, COUNT=str(self.root / "count"))
        runs = [line for line in self.events.read_text().splitlines() if line.startswith("maestro")]
        return result, len(runs)

    def test_a_foreign_device_offline_at_session_start_is_retried_once(self):
        result, runs = self.run_mode("retry")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(runs, 2)
        self.assertIn(f"{OFFLINE} while Maestro 2.8.0 listed every device", result.stderr)
        self.assertIn("retrying it once", result.stderr)

    def test_a_second_foreign_offline_failure_fails_the_run(self):
        result, runs = self.run_mode("twice")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(runs, 2)
        self.assertIn("failed again", result.stderr)

    def test_a_failure_after_a_flow_started_is_not_retried(self):
        result, runs = self.run_mode("started")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(runs, 1)
        self.assertIn("a flow had started", result.stderr)
        self.assertNotIn("retrying", result.stderr)


if __name__ == "__main__":
    unittest.main()

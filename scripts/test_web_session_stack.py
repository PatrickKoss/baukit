from __future__ import annotations

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("web-session-stack.py")
SPEC = importlib.util.spec_from_file_location("web_session_stack", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
STACK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STACK)


class WebSessionStackTest(unittest.TestCase):
    def test_override_publishes_no_host_ports_and_shares_the_keycloak_network(self) -> None:
        override = STACK.compose_override(
            Path("/repo"), Path("/repo/fixture"), Path("/cargo/target/debug")
        )
        self.assertEqual(override.count("ports: !reset []"), 3)
        self.assertIn("network_mode: service:stack", override)
        self.assertIn('"/repo:/repo:ro"', override)
        self.assertIn('"/repo/fixture:/repo/fixture"', override)
        self.assertIn('"/cargo/target/debug:/cargo/target/debug:ro"', override)

    def test_binaries_follow_the_cargo_target_directory(self) -> None:
        fixture = Path("/repo/fixture")
        with patch.dict(STACK.os.environ, {"CARGO_TARGET_DIR": "/cargo/target"}):
            self.assertEqual(STACK.binary_directory(fixture), Path("/cargo/target/debug"))
        with patch.dict(STACK.os.environ, {"CARGO_TARGET_DIR": ""}):
            self.assertEqual(
                STACK.binary_directory(fixture), Path("/repo/fixture/backend/target/debug")
            )

    def test_stack_is_torn_down_even_when_startup_fails(self) -> None:
        for fail_startup in (False, True):
            with self.subTest(fail_startup=fail_startup):

                def run(command: list[str], fail: bool = fail_startup, **_kwargs: object) -> None:
                    if fail and "up" in command:
                        raise subprocess.CalledProcessError(1, command)

                with patch.object(STACK.subprocess, "run", side_effect=run) as mocked:
                    if fail_startup:
                        with self.assertRaises(subprocess.CalledProcessError):
                            STACK.run(Path("/repo/fixture"), Path("/repo"))
                    else:
                        STACK.run(Path("/repo/fixture"), Path("/repo"))
                commands = [call.args[0] for call in mocked.call_args_list]
                self.assertEqual(commands[-1][-2:], ["down", "--volumes"])
                self.assertTrue(all("baukit-web-session-" in command[3] for command in commands))


if __name__ == "__main__":
    unittest.main()

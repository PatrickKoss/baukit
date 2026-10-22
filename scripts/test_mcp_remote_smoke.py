from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("mcp-remote-smoke.py")
SPEC = importlib.util.spec_from_file_location("mcp_remote_smoke", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


class McpSmokeProjectTest(unittest.TestCase):
    def test_cleanup_uses_its_own_project_even_when_startup_fails(self) -> None:
        repository = SCRIPT.resolve().parents[1]
        for fail_startup in (False, True):
            with self.subTest(fail_startup=fail_startup):
                def run(command: list[str], **_kwargs: object) -> None:
                    if fail_startup and "up" in command:
                        raise subprocess.CalledProcessError(1, command)

                with patch.dict(os.environ, {"COMPOSE_PROJECT_NAME": "unrelated-live-stack"}):
                    with patch.object(SMOKE.subprocess, "run", side_effect=run) as mocked:
                        if fail_startup:
                            with self.assertRaises(subprocess.CalledProcessError):
                                SMOKE.run(repository / "fixture", repository)
                        else:
                            SMOKE.run(repository / "fixture", repository)
                commands = [call.args[0] for call in mocked.call_args_list]
                compose = [command for command in commands if command[:2] == ["docker", "compose"]]
                self.assertEqual(len(compose), 2 if fail_startup else 3)
                for command in compose:
                    project = command[command.index("--project-name") + 1]
                    self.assertEqual(project, f"baukit-mcp-smoke-{os.getpid()}")
                self.assertEqual(compose[-1][-2:], ["down", "--volumes"])


if __name__ == "__main__":
    unittest.main()

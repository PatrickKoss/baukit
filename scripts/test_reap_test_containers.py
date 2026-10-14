from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest


SCRIPT = Path(__file__).resolve().parent / "reap-test-containers.sh"
FAKE_DOCKER = r'''#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

state = json.loads(Path(os.environ["FAKE_DOCKER_STATE"]).read_text())
args = sys.argv[1:]
with Path(os.environ["FAKE_DOCKER_CALLS"]).open("a") as log:
    log.write(json.dumps(args) + "\n")
if args[0] == "ps":
    assert "--all" in args and "--quiet" in args
    assert "label=dev.baukit.test=true" in args
    identity = next((arg.removeprefix("id=") for arg in args if arg.startswith("id=")), None)
    for container in state:
        if identity and (container["id"] != identity or
                         os.environ.get("FAKE_DOCKER_DISAPPEAR_INSPECT") or
                         os.environ.get("FAKE_DOCKER_DISAPPEAR_RM")):
            continue
        print(container["id"])
elif args[0] == "inspect":
    if os.environ.get("FAKE_DOCKER_INSPECT_FAILURE") or os.environ.get("FAKE_DOCKER_DISAPPEAR_INSPECT"):
        sys.exit(1)
    container = next(item for item in state if item["id"] == args[-1])
    print("\x1f".join(container[key] for key in
          ("marker", "pid", "binary", "manifest", "started", "image")))
elif args[0] == "rm":
    assert args[1:3] == ["--force", "--volumes"]
    if os.environ.get("FAKE_DOCKER_DISAPPEAR_RM") or os.environ.get("FAKE_DOCKER_RM_FAILURE"):
        sys.exit(1)
else:
    raise AssertionError(args)
'''


class ReapTestContainersTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        docker = root / "docker"
        docker.write_text(FAKE_DOCKER)
        docker.chmod(0o755)
        self.state = root / "state.json"
        self.calls = root / "calls.jsonl"
        self.environment = {
            **os.environ,
            "PATH": f"{root}:{os.environ['PATH']}",
            "FAKE_DOCKER_STATE": str(self.state),
            "FAKE_DOCKER_CALLS": str(self.calls),
        }
        self.binary = Path(f"/proc/{os.getpid()}/exe").resolve().name
        child = subprocess.Popen(["true"])
        self.dead_pid = str(child.pid)
        child.wait()

    def container(self, identity, **overrides):
        return {
            "id": identity,
            "marker": "true",
            "pid": self.dead_pid,
            "binary": "test-binary",
            "manifest": "/repo/worktree with spaces/backend",
            "started": str(int(time.time()) - 3600),
            "image": "postgres:18.6-alpine",
            **overrides,
        }

    def run_script(self, containers, *arguments):
        self.state.write_text(json.dumps(containers))
        result = subprocess.run(
            ["bash", str(SCRIPT), *arguments], env=self.environment,
            capture_output=True, text=True, timeout=10,
        )
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()] \
            if self.calls.exists() else []
        return result, calls

    def test_removes_dead_and_reused_pids_and_preserves_live_and_unlabelled(self):
        result, calls = self.run_script([
            self.container("dead"),
            self.container("reused", pid=str(os.getpid()), binary="different-binary"),
            self.container("live", pid=str(os.getpid()), binary=self.binary),
            self.container("unlabelled", marker=""),
            self.container("false-label", marker="false"),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[-1] for call in calls if call[0] == "rm"], ["dead", "reused"])

    def test_dry_run_lists_details_and_removes_nothing(self):
        result, calls = self.run_script([self.container("dead")], "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))
        for detail in ["dead", "postgres:18.6-alpine", "/repo/worktree with spaces/backend", "test-binary"]:
            self.assertIn(detail, result.stdout)
        self.assertRegex(result.stdout, r"age=36\d\ds")

    def test_missing_binary_preserves_live_pid_but_allows_dead_pid_cleanup(self):
        result, calls = self.run_script([
            self.container("unknown-live", pid=str(os.getpid()), binary=""),
            self.container("unknown-dead", binary="", manifest="", started=""),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[-1] for call in calls if call[0] == "rm"], ["unknown-dead"])
        self.assertIn("age=unknown", result.stdout)

    def test_invalid_pids_are_preserved(self):
        result, calls = self.run_script([
            self.container(f"invalid-{index}", pid=pid)
            for index, pid in enumerate(["", "0", "-1", "abc", "../1"])
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))

    def test_unreadable_live_executable_is_preserved(self):
        readlink = Path(self.directory.name) / "readlink"
        readlink.write_text("#!/bin/sh\nexit 1\n")
        readlink.chmod(0o755)
        result, calls = self.run_script([
            self.container("unreadable", pid=str(os.getpid()), binary="different-binary"),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))

    def test_deleted_executable_with_same_name_is_preserved(self):
        readlink = Path(self.directory.name) / "readlink"
        readlink.write_text(f"#!/bin/sh\nprintf '/usr/bin/{self.binary} (deleted)\\n'\n")
        readlink.chmod(0o755)
        result, calls = self.run_script([
            self.container("deleted-executable", pid=str(os.getpid()), binary=self.binary),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))

    def test_binary_name_ending_in_deleted_is_preserved(self):
        readlink = Path(self.directory.name) / "readlink"
        readlink.write_text("#!/bin/sh\nprintf '/target/test (deleted)\\n'\n")
        readlink.chmod(0o755)
        result, calls = self.run_script([
            self.container("suffix-in-name", pid=str(os.getpid()), binary="test (deleted)"),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))

    def test_invalid_and_future_start_times_do_not_prevent_cleanup(self):
        result, calls = self.run_script([
            self.container(f"unknown-age-{index}", started=started)
            for index, started in enumerate(["bad", "18446744073709551615", str(int(time.time()) + 3600)])
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len([call for call in calls if call[0] == "rm"]), 3)
        self.assertEqual(result.stdout.count("age=unknown"), 3)

    def test_docker_failure_is_reported_without_removal(self):
        self.environment["FAKE_DOCKER_INSPECT_FAILURE"] = "1"
        result, calls = self.run_script([self.container("dead")])
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(call[0] == "rm" for call in calls))

    def test_container_removed_between_listing_and_inspection_is_skipped(self):
        self.environment["FAKE_DOCKER_DISAPPEAR_INSPECT"] = "1"
        result, calls = self.run_script([self.container("gone")])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))

    def test_container_removed_before_removal_succeeds(self):
        self.environment["FAKE_DOCKER_DISAPPEAR_RM"] = "1"
        result, calls = self.run_script([self.container("gone")])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len([call for call in calls if call[0] == "rm"]), 1)

    def test_removal_failure_for_existing_container_fails(self):
        self.environment["FAKE_DOCKER_RM_FAILURE"] = "1"
        result, calls = self.run_script([self.container("dead")])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len([call for call in calls if call[0] == "rm"]), 1)

    def test_empty_list_succeeds(self):
        result, calls = self.run_script([])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 1)
        self.assertEqual(result.stdout, "")

    def test_unknown_option_fails_before_docker(self):
        result, calls = self.run_script([], "--unknown")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, [])

from __future__ import annotations

import json
import os
from pathlib import Path
import shlex
import shutil
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
          ("marker", "pid", "binary", "manifest", "started", "image", "pid_ns", "host")))
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
        self.environment.pop("DOCKER_HOST", None)
        self.pid_namespace = os.readlink("/proc/self/ns/pid")
        self.hostname = Path("/proc/sys/kernel/hostname").read_text().rstrip("\n")
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
            "pid_ns": self.pid_namespace,
            "host": self.hostname,
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

    def test_mismatched_pid_namespace_is_preserved(self):
        result, calls = self.run_script([
            self.container("foreign-namespace", pid_ns="pid:[different]"),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))
        self.assertEqual(result.stdout, "")

    def test_mismatched_host_is_preserved(self):
        result, calls = self.run_script([
            self.container("foreign-host", host=f"{self.hostname}-different"),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))
        self.assertEqual(result.stdout, "")

    def test_missing_scope_labels_are_preserved(self):
        result, calls = self.run_script([
            self.container("missing-namespace", pid_ns=""),
            self.container("missing-host", host=""),
            self.container("missing-both", pid_ns="", host=""),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))
        self.assertEqual(result.stdout, "")

    def test_non_unix_docker_hosts_are_skipped_before_docker(self):
        for endpoint in ["tcp://docker.example:2376", "ssh://docker.example", ""]:
            with self.subTest(endpoint=endpoint):
                self.environment["DOCKER_HOST"] = endpoint
                result, calls = self.run_script([self.container("remote-orphan")])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, [])
                self.assertEqual(result.stdout, "")
                self.assertIn("DOCKER_HOST", result.stderr)
                self.assertIn("unix socket", result.stderr)

    def test_unix_docker_host_allows_matching_orphan_cleanup(self):
        self.environment["DOCKER_HOST"] = "unix:///var/run/docker.sock"
        result, calls = self.run_script([self.container("local-orphan")])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[-1] for call in calls if call[0] == "rm"], ["local-orphan"])

    def mock_executable_readlink(self, body):
        real_readlink = shutil.which("readlink")
        self.assertIsNotNone(real_readlink)
        readlink = Path(self.directory.name) / "readlink"
        readlink.write_text(
            "#!/bin/sh\n"
            'if [ "$1" = /proc/self/ns/pid ]; then\n'
            f'    exec {shlex.quote(real_readlink)} "$@"\n'
            "fi\n" + body
        )
        readlink.chmod(0o755)

    def test_unreadable_pid_namespace_preserves_all_containers(self):
        readlink = Path(self.directory.name) / "readlink"
        readlink.write_text("#!/bin/sh\nexit 1\n")
        readlink.chmod(0o755)
        result, calls = self.run_script([self.container("unknown-namespace")])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [])
        self.assertEqual(result.stdout, "")

    def mock_hostname_files(self, primary, fallback):
        real_cat = shutil.which("cat")
        self.assertIsNotNone(real_cat)
        lines = ["#!/bin/sh"]
        for path, value in [("/proc/sys/kernel/hostname", primary), ("/etc/hostname", fallback)]:
            lines.append(f'if [ "$1" = {shlex.quote(path)} ]; then')
            if value is None:
                lines.append("    exit 1")
            else:
                lines.append(f"    printf '%s\\n' {shlex.quote(value)}")
                lines.append("    exit 0")
            lines.append("fi")
        lines.append(f'exec {shlex.quote(real_cat)} "$@"')
        cat = Path(self.directory.name) / "cat"
        cat.write_text("\n".join(lines) + "\n")
        cat.chmod(0o755)

    def test_hostname_fallback_allows_matching_orphan_cleanup(self):
        self.mock_hostname_files(None, "fallback-host")
        result, calls = self.run_script([self.container("fallback-orphan", host="fallback-host")])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[-1] for call in calls if call[0] == "rm"], ["fallback-orphan"])

    def test_unreadable_hostname_preserves_all_containers(self):
        self.mock_hostname_files(None, None)
        result, calls = self.run_script([self.container("unknown-host")])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [])
        self.assertEqual(result.stdout, "")

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
        self.mock_executable_readlink("exit 1\n")
        result, calls = self.run_script([
            self.container("unreadable", pid=str(os.getpid()), binary="different-binary"),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))

    def test_deleted_executable_with_same_name_is_preserved(self):
        self.mock_executable_readlink(f"printf '/usr/bin/{self.binary} (deleted)\\n'\n")
        result, calls = self.run_script([
            self.container("deleted-executable", pid=str(os.getpid()), binary=self.binary),
        ])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[0] == "rm" for call in calls))

    def test_binary_name_ending_in_deleted_is_preserved(self):
        self.mock_executable_readlink("printf '/target/test (deleted)\\n'\n")
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

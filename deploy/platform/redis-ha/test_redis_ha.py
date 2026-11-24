import hashlib
import importlib.util
import os
import re
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("redis-ha.sh")
STATEFULSET = Path(__file__).with_name("statefulset.yaml")
PROCESS_RUNNER = Path(__file__).resolve().parents[3] / "scripts" / "process_runner.py"


def load_process_runner():
    spec = importlib.util.spec_from_file_location("process_runner", PROCESS_RUNNER)
    if spec is None or spec.loader is None:
        raise ImportError(PROCESS_RUNNER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


process_runner = load_process_runner()
SENTINEL_TILT_MS = 30000
SUFFIX = ".redis-headless.platform-redis.svc.cluster.local"
STUB = textwrap.dedent(
    """\
    #!/bin/sh
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "-h" ]; then host="$2"; fi
      shift
    done
    answer="$STUB_DIR/$host"
    refused="$answer.refused"
    count=$(($(cat "$refused" 2>/dev/null || echo 0) + 1))
    echo "$count" >"$refused"
    [ "$count" -gt "${STUB_REFUSALS:-0}" ] || exit 1
    [ -f "$answer" ] || exit 1
    cat "$answer"
    """
)


def fqdn(ordinal: int) -> str:
    return f"redis-{ordinal}{SUFFIX}"


def sentinel_setting(conf: str, name: str) -> int:
    match = re.search(rf"^sentinel {name} mymaster (\d+)$", conf, re.MULTILINE)
    if match is None:
        raise AssertionError(f"sentinel {name} missing from:\n{conf}")
    return int(match.group(1))


class RedisHaScript:
    def decide(
        self,
        pod: str,
        answers: dict[int, str],
        mode: str = "decide",
        extra_env: dict[str, str] | None = None,
        local_info: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as tmp:
            stub_dir = Path(tmp)
            cli = stub_dir / "redis-cli"
            cli.write_text(STUB)
            cli.chmod(0o755)
            sentinel = stub_dir / "redis-sentinel"
            sentinel.write_text('#!/bin/sh\ncat "$1"\n')
            sentinel.chmod(0o755)
            server = stub_dir / "redis-server"
            server.write_text('#!/bin/sh\nprintf \'arg %s\\n\' "$@"\n[ ! -f "$1" ] || cat "$1"\n')
            server.chmod(0o755)
            for ordinal, answer in answers.items():
                (stub_dir / fqdn(ordinal)).write_text(answer)
            if local_info is not None:
                (stub_dir / "127.0.0.1").write_text(local_info)
            env = {
                **os.environ,
                "PATH": f"{stub_dir}:{os.environ['PATH']}",
                "STUB_DIR": str(stub_dir),
                "POD_NAME": pod,
                "SELF_WAIT_SECONDS": "2",
                "RETRY_SECONDS": "0",
                "NO_ANSWER_ATTEMPTS": "2",
                "LOCAL_WAIT_ATTEMPTS": "1",
                "SENTINEL_CONF": str(stub_dir / "sentinel.conf"),
                "REDIS_ACL_CONF": str(stub_dir / "redis-acl.conf"),
                "REDIS_ACL_FILE": str(stub_dir / "redis-users.acl"),
                "REDIS_ACL_RELOAD_TOKEN": str(stub_dir / "redis-acl-reload.token"),
                **(extra_env or {}),
            }
            return process_runner.run_process(
                ["sh", str(SCRIPT), mode],
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

    def sentinel_conf(self, pod: str) -> str:
        return self.decide(pod, {0: f"{fqdn(0)}\n6379\n"}, mode="sentinel").stdout


class DecideMasterTest(RedisHaScript, unittest.TestCase):
    def assert_decision(self, pod: str, answers: dict[int, str], expected: str) -> None:
        result = self.decide(pod, answers)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), expected)

    def test_redis_0_bootstraps_when_no_sentinel_answers(self) -> None:
        self.assert_decision("redis-0", {}, "self")

    def test_other_pod_follows_redis_0_when_no_sentinel_answers(self) -> None:
        self.assert_decision("redis-2", {}, fqdn(0))

    def test_restarted_redis_0_waits_for_peers_its_network_still_rejects(self) -> None:
        answers = {1: f"{fqdn(1)}\n6379\n", 2: f"{fqdn(1)}\n6379\n"}
        late_network = {"NO_ANSWER_ATTEMPTS": "", "STUB_REFUSALS": "5"}
        result = self.decide("redis-0", answers, extra_env=late_network)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), fqdn(1))

    def test_restarted_old_master_follows_promoted_replica(self) -> None:
        answers = {1: f"{fqdn(1)}\n6379\n", 2: f"{fqdn(1)}\n6379\n"}
        self.assert_decision("redis-0", answers, fqdn(1))

    def test_one_answering_sentinel_is_enough(self) -> None:
        self.assert_decision("redis-0", {2: f"{fqdn(2)}\n6379\n"}, fqdn(2))

    def test_unknown_master_counts_as_no_answer(self) -> None:
        self.assert_decision("redis-0", {1: "\n", 2: "IDONTKNOW\n"}, "self")

    def test_self_reported_master_starts_as_master_only_after_waiting(self) -> None:
        answers = {1: f"{fqdn(0)}\n6379\n", 2: f"{fqdn(0)}\n6379\n"}
        result = self.decide("redis-0", answers)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "self")
        self.assertIn("waiting for Sentinels", result.stderr)

    def default_self_wait_seconds(self) -> int:
        answers = {1: f"{fqdn(0)}\n6379\n", 2: f"{fqdn(0)}\n6379\n"}
        result = self.decide("redis-0", answers, extra_env={"SELF_WAIT_SECONDS": ""})
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stderr.count("waiting for Sentinels")

    def slowest_detection_and_failover_timeout_ms(self) -> tuple[int, int]:
        confs = [self.sentinel_conf(f"redis-{ordinal}") for ordinal in range(3)]
        down_after_ms = max(sentinel_setting(conf, "down-after-milliseconds") for conf in confs)
        return down_after_ms, sentinel_setting(confs[0], "failover-timeout")

    def test_restarted_master_waits_for_three_failover_retries_after_split_votes(self) -> None:
        waited_seconds = self.default_self_wait_seconds()
        down_after_ms, failover_ms = self.slowest_detection_and_failover_timeout_ms()
        retries_after_split_votes_ms = down_after_ms + 3 * 2 * failover_ms
        self.assertGreaterEqual(waited_seconds * 1000, retries_after_split_votes_ms + failover_ms)

    def test_restarted_master_waits_out_a_sentinel_tilt_before_the_split_vote_retry(self) -> None:
        waited_seconds = self.default_self_wait_seconds()
        down_after_ms, failover_ms = self.slowest_detection_and_failover_timeout_ms()
        worst_case_promotion_ms = down_after_ms + SENTINEL_TILT_MS + 3 * failover_ms
        self.assertGreaterEqual(waited_seconds * 1000, worst_case_promotion_ms)

    def test_disagreeing_sentinels_fail_the_start(self) -> None:
        answers = {1: f"{fqdn(1)}\n6379\n", 2: f"{fqdn(2)}\n6379\n"}
        result = self.decide("redis-0", answers)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("disagree", result.stderr)


class SentinelConfigTest(RedisHaScript, unittest.TestCase):
    def test_sentinels_detect_a_dead_master_at_different_times(self) -> None:
        delays = set()
        for ordinal in range(3):
            conf = self.sentinel_conf(f"redis-{ordinal}")
            delays.add(sentinel_setting(conf, "down-after-milliseconds"))
        self.assertEqual(len(delays), 3)
        self.assertGreaterEqual(max(delays) - min(delays), 2 * 1000)

    def test_sentinel_monitors_reported_master_and_announces_hostname(self) -> None:
        result = self.decide("redis-0", {1: f"{fqdn(2)}\n6379\n"}, mode="sentinel")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"sentinel monitor mymaster {fqdn(2)} 6379 2", result.stdout)
        self.assertIn("sentinel announce-hostnames yes", result.stdout)
        self.assertIn("sentinel resolve-hostnames yes", result.stdout)
        self.assertIn(f"sentinel announce-ip {fqdn(0)}", result.stdout)

    def test_sentinel_does_not_wait_when_peers_report_this_pod(self) -> None:
        answers = {1: f"{fqdn(0)}\n6379\n"}
        extra = {"SELF_WAIT_SECONDS": "5000", "RETRY_SECONDS": "5"}
        result = self.decide("redis-0", answers, mode="sentinel", extra_env=extra)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"sentinel monitor mymaster {fqdn(0)} 6379 2", result.stdout)

    def test_sentinel_falls_back_to_redis_0_when_peers_disagree(self) -> None:
        answers = {0: f"{fqdn(0)}\n6379\n", 2: f"{fqdn(2)}\n6379\n"}
        result = self.decide("redis-1", answers, mode="sentinel")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"sentinel monitor mymaster {fqdn(0)} 6379 2", result.stdout)

    def test_sentinel_monitors_the_master_local_redis_replicates(self) -> None:
        info = f"# Replication\r\nrole:slave\r\nmaster_host:{fqdn(1)}\r\nmaster_port:6379\r\n"
        answers = {2: f"{fqdn(2)}\n6379\n"}
        result = self.decide("redis-0", answers, mode="sentinel", local_info=info)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"sentinel monitor mymaster {fqdn(1)} 6379 2", result.stdout)

    def test_sentinel_monitors_this_pod_when_local_redis_is_master(self) -> None:
        info = "# Replication\r\nrole:master\r\nconnected_slaves:0\r\n"
        result = self.decide("redis-0", {}, mode="sentinel", local_info=info)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"sentinel monitor mymaster {fqdn(0)} 6379 2", result.stdout)


class AclConfigTest(RedisHaScript, unittest.TestCase):
    def acl_env(self, acl_dir: Path) -> dict[str, str]:
        (acl_dir / "replication-password").write_text("replication-secret")
        (acl_dir / "sentinel-password").write_text("sentinel-secret")
        return {"REDIS_ACL_DIR": str(acl_dir)}

    def test_server_without_acl_dir_starts_without_config_file(self) -> None:
        result = self.decide("redis-0", {}, mode="server")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("aclfile", result.stdout)
        self.assertIn("arg --save", result.stdout.splitlines()[0])

    def test_server_with_acl_dir_loads_acl_and_replication_credentials(self) -> None:
        with tempfile.TemporaryDirectory() as acl:
            (Path(acl) / "users.acl").write_text("user default on nopass -@all +ping\n")
            result = self.decide("redis-2", {}, mode="server", extra_env=self.acl_env(Path(acl)))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertRegex(result.stdout, r"(?m)^aclfile \S+/redis-users\.acl$")
            self.assertIn("masteruser replication", result.stdout)
            self.assertIn("masterauth replication-secret", result.stdout)
            self.assertNotIn("arg replication-secret", result.stdout)
            self.assertIn(f"arg {fqdn(0)}", result.stdout)

    def test_sentinel_without_acl_dir_has_no_auth(self) -> None:
        result = self.decide("redis-0", {1: f"{fqdn(1)}\n6379\n"}, mode="sentinel")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("auth-", result.stdout)

    def test_sentinel_with_acl_dir_authenticates_as_sentinel_user(self) -> None:
        with tempfile.TemporaryDirectory() as acl:
            env = self.acl_env(Path(acl))
            answers = {1: f"{fqdn(1)}\n6379\n"}
            result = self.decide("redis-0", answers, mode="sentinel", extra_env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = result.stdout.splitlines()
            monitor = lines.index(f"sentinel monitor mymaster {fqdn(1)} 6379 2")
            self.assertGreater(lines.index("sentinel auth-user mymaster sentinel"), monitor)
            self.assertIn("sentinel auth-pass mymaster sentinel-secret", lines)


RELOAD_STUB = textwrap.dedent(
    """\
    #!/bin/sh
    call=$(($(cat "$STUB_DIR/calls" 2>/dev/null || echo 0) + 1))
    echo "$call" >"$STUB_DIR/calls"
    printf 'auth=%s args=%s\\n' "${REDISCLI_AUTH:-}" "$*" >>"$STUB_DIR/cli.log"
    cp "$REDIS_ACL_FILE" "$STUB_DIR/loaded.$call"
    [ ! -f "$STUB_DIR/change.$call" ] || cat "$STUB_DIR/change.$call" >>"$REDIS_ACL_DIR/users.acl"
    reply="$STUB_DIR/reply.$call"
    [ -f "$reply" ] || reply="$STUB_DIR/reply"
    cat "$reply"
    """
)
TOKEN = "a" * 64
REJECTED = (
    "ERR ACL_FILE:2: Error in applying operation 'tenant-secret': Syntax error. "
    "WARNING: ACL errors detected, no change to the previously active ACL rules was performed\n"
)


class AclReloadTest(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.stub_dir = Path(tmp.name)
        self.acl_dir = self.stub_dir / "acl"
        self.acl_dir.mkdir()
        (self.acl_dir / "users.acl").write_text("user default on nopass -@all +ping")
        cli = self.stub_dir / "redis-cli"
        cli.write_text(RELOAD_STUB)
        cli.chmod(0o755)
        self.acl_file = self.stub_dir / "redis-users.acl"
        self.token = self.stub_dir / "redis-acl-reload.token"
        self.token.write_text(TOKEN)
        (self.stub_dir / "reply").write_text("OK\n")
        self.rejected = REJECTED.replace("ACL_FILE", str(self.acl_file))

    def reload(self, polls: int, retry_seconds: int = 3600) -> subprocess.CompletedProcess[str]:
        env = {
            **os.environ,
            "PATH": f"{self.stub_dir}:{os.environ['PATH']}",
            "STUB_DIR": str(self.stub_dir),
            "POD_NAME": "redis-1",
            "REDIS_ACL_DIR": str(self.acl_dir),
            "REDIS_ACL_FILE": str(self.acl_file),
            "REDIS_ACL_RELOAD_TOKEN": str(self.token),
            "ACL_POLL_SECONDS": "0",
            "ACL_RETRY_SECONDS": str(retry_seconds),
            "ACL_POLL_LIMIT": str(polls),
        }
        return process_runner.run_process(
            ["sh", str(SCRIPT), "acl-reload"],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    def calls(self) -> list[str]:
        log = self.stub_dir / "cli.log"
        return log.read_text().splitlines() if log.exists() else []

    def test_unchanged_file_is_loaded_once_as_the_reload_user(self) -> None:
        result = self.reload(polls=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual(len(calls), 1, calls)
        self.assertTrue(calls[0].startswith(f"auth={TOKEN} "), calls[0])
        self.assertIn("--user baukit-acl-reload ACL LOAD", calls[0])
        self.assertNotIn(TOKEN, calls[0].split(" args=", 1)[1])
        self.assertIn("loaded users.acl sha256", result.stderr)

    def test_loaded_file_is_the_secret_plus_a_reload_user_limited_to_acl_load(self) -> None:
        self.reload(polls=1)
        digest = hashlib.sha256(TOKEN.encode()).hexdigest()
        self.assertEqual(
            (self.stub_dir / "loaded.1").read_text().splitlines(),
            [
                "user default on nopass -@all +ping",
                f"user baukit-acl-reload on #{digest} resetkeys resetchannels -@all +acl|load",
            ],
        )
        self.assertEqual(self.acl_file.stat().st_mode & 0o777, 0o600)

    def test_rejected_file_is_logged_without_the_quoted_token_and_retried_later(self) -> None:
        (self.stub_dir / "reply").write_text(self.rejected)
        result = self.reload(polls=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertIn("Redis keeps the previous users: ERR users.acl:2:", result.stderr)
        self.assertNotIn("tenant-secret", result.stderr)
        self.assertNotIn("WARNING", result.stderr)

    def test_rejected_file_is_retried_after_the_retry_interval(self) -> None:
        (self.stub_dir / "reply").write_text(self.rejected)
        self.reload(polls=3, retry_seconds=0)
        self.assertEqual(len(self.calls()), 3)

    def test_changed_file_after_a_rejection_is_loaded_at_once(self) -> None:
        (self.stub_dir / "reply.1").write_text(self.rejected)
        (self.stub_dir / "change.1").write_text("\nuser tenant-b on >b -@all +get\n")
        result = self.reload(polls=3)
        self.assertEqual(len(self.calls()), 2, result.stderr)
        self.assertIn("user tenant-b on >b -@all +get", (self.stub_dir / "loaded.2").read_text())

    def test_waits_quietly_until_redis_creates_the_token(self) -> None:
        self.token.unlink()
        result = self.reload(polls=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertEqual(result.stderr.count("retrying"), 1, result.stderr)

    def test_unreachable_redis_is_retried_on_every_poll(self) -> None:
        (self.stub_dir / "reply").write_text("Could not connect to Redis at 127.0.0.1:6379\n")
        result = self.reload(polls=3)
        self.assertEqual(len(self.calls()), 3)
        self.assertEqual(result.stderr.count("retrying"), 1, result.stderr)


class AclComponentTests(unittest.TestCase):
    def test_secret_is_mounted_as_a_directory_so_the_kubelet_updates_it(self) -> None:
        component = STATEFULSET.parents[1] / "components" / "redis-ha-acl" / "statefulset-acl.yaml"
        text = component.read_text()
        self.assertNotIn("subPath", text)
        self.assertIn("- acl-reload\n", text)


class SentinelConfigVolumeTests(unittest.TestCase):
    def test_sentinel_config_lives_in_memory(self) -> None:
        # Sentinel fsyncs its config on every vote before the vote request leaves the event loop.
        volume = "- name: sentinel-config\n          emptyDir:\n            medium: Memory\n"
        self.assertIn(volume, STATEFULSET.read_text())


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import os
import re
import subprocess
import tempfile
import textwrap
import unittest
from fnmatch import fnmatch
from pathlib import Path
from typing import Any

import yaml
from process_runner import run_process

PLATFORM = Path(__file__).resolve().parent.parent / "deploy/platform"
HELPER = PLATFORM / "local-git-http.sh"
WILDCARD_ADDRESS = re.compile(r"(?<![\d.])0\.0\.0\.0(?![\d.])")
LIFECYCLE_SCRIPTS = (PLATFORM / "platform-up.sh", PLATFORM / "platform-lifecycle.sh")


def bash(
    script: str, *, cwd: Path | None = None, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return run_process(
        ["bash", "-euo", "pipefail", "-c", script],
        env={**os.environ, **(env or {})},
        cwd=cwd,
        check=False,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
    )


def write_stub(directory: Path, name: str, body: str) -> None:
    path = directory / name
    path.write_text("#!/usr/bin/env bash\nset -eu\n" + body)
    path.chmod(0o755)


KERNEL_DEFAULTS = {
    "vm/overcommit_memory": "1",
    "vm/panic_on_oom": "0",
    "kernel/panic": "10",
    "kernel/panic_on_oops": "1",
}


def write_sysctls(root: Path, values: dict[str, str]) -> None:
    for name, value in values.items():
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_text(value + "\n")


class SnapshotTreeTest(unittest.TestCase):
    def test_snapshot_holds_what_git_would_commit(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            target = Path(directory) / "target"
            target.mkdir()
            git = ["git", "-C", str(source), "-c", "user.name=t", "-c", "user.email=t@invalid"]
            run_process(["git", "init", "-q", str(source)], check=True, capture_output=True)
            (source / ".gitignore").write_text("secret.env\nbuild/\n")
            (source / "tracked.txt").write_text("tracked\n")
            (source / "removed.txt").write_text("removed\n")
            (source / "dir with space").mkdir()
            (source / "dir with space/file.txt").write_text("nested\n")
            run_process([*git, "add", "-A"], check=True, capture_output=True)
            run_process([*git, "commit", "-qm", "init"], check=True, capture_output=True)
            (source / "removed.txt").unlink()
            (source / "untracked.txt").write_text("new\n")
            (source / "secret.env").write_text("TOKEN=1\n")
            (source / "build").mkdir()
            (source / "build/artifact").write_text("binary\n")

            result = bash(f'source "{HELPER}"\nsnapshot_tree "{source}" "{target}"')

            self.assertEqual(result.returncode, 0, result.stderr)
            files = sorted(str(p.relative_to(target)) for p in target.rglob("*") if p.is_file())
            self.assertEqual(
                files, [".gitignore", "dir with space/file.txt", "tracked.txt", "untracked.txt"]
            )


class BindAddressTest(unittest.TestCase):
    def run_with_stubs(
        self, script: str, docker_output: str = "2", sysctls: dict[str, str] | None = None
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            calls = bin_dir / "calls"
            sysctl_root = bin_dir / "sys"
            write_sysctls(sysctl_root, KERNEL_DEFAULTS if sysctls is None else sysctls)
            for tool in ("k3d", "docker"):
                write_stub(
                    bin_dir, tool, f'printf "%s\\n" "$*" >> "{calls}"\nprintf "%s" "$STUB_OUTPUT"\n'
                )
            result = bash(
                f'source "{HELPER}"\nplatform_sysctl_root="{sysctl_root}"\n{script}',
                env={
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                    "STUB_OUTPUT": docker_output,
                },
            )
            return result, calls.read_text() if calls.exists() else ""

    def test_cluster_api_and_ports_bind_loopback(self) -> None:
        result, calls = self.run_with_stubs("k3d_create_local_cluster ph img random 8080 8443")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--api-port 127.0.0.1:random", calls)
        self.assertIn("--port 127.0.0.1:8080:80@loadbalancer", calls)
        self.assertIn("--port 127.0.0.1:8443:443@loadbalancer", calls)
        self.assertIsNone(WILDCARD_ADDRESS.search(calls))

    def test_cluster_forwards_arguments_after_the_fifth_to_k3d(self) -> None:
        result, calls = self.run_with_stubs(
            "k3d_create_local_cluster ph img random 8080 8443 --subnet 10.89.0.0/24"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--wait --subnet 10.89.0.0/24", calls)

    def test_cgroup_v1_host_fails_before_k3d(self) -> None:
        result, calls = self.run_with_stubs(
            "k3d_create_local_cluster ph img random 8080 8443", docker_output="1"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cgroup_no_v1=all", result.stderr)
        self.assertNotIn("cluster create", calls)

    def test_cgroup_v2_host_creates_the_cluster(self) -> None:
        result, calls = self.run_with_stubs(
            "k3d_create_local_cluster ph img random 8080 8443", docker_output="2"
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("info --format {{.CgroupVersion}}", calls)
        self.assertIn("cluster create ph", calls)

    def test_unknown_cgroup_version_fails_before_k3d(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            calls = bin_dir / "calls"
            write_stub(bin_dir, "docker", "exit 1\n")
            write_stub(bin_dir, "k3d", f'printf "%s\\n" "$*" >> "{calls}"\n')
            sysctl_root = bin_dir / "sys"
            write_sysctls(sysctl_root, KERNEL_DEFAULTS)
            result = bash(
                f'source "{HELPER}"\nplatform_sysctl_root="{sysctl_root}"\n'
                "k3d_create_local_cluster ph img random 8080 8443",
                env={"PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}"},
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(calls.exists())

    def test_cluster_gets_every_hardening_arg_on_the_server(self) -> None:
        result, calls = self.run_with_stubs("k3d_create_local_cluster ph img random 8080 8443")
        self.assertEqual(result.returncode, 0, result.stderr)
        for arg in (
            "--disable=traefik",
            "--secrets-encryption",
            "--protect-kernel-defaults",
            "--kube-apiserver-arg=admission-control-config-file=/etc/baukit/k3s/pod-security.yaml",
            "--kube-apiserver-arg=audit-policy-file=/etc/baukit/k3s/audit-policy.yaml",
            "--kube-apiserver-arg=audit-log-path=/var/log/kubernetes/audit/audit.log",
        ):
            self.assertIn(f"--k3s-arg {arg}@server:0 ", calls)
        self.assertIn(f"--volume {PLATFORM / 'k3s'}:/etc/baukit/k3s:ro@server:0 ", calls)

    def test_wrong_kernel_defaults_fail_before_k3d(self) -> None:
        result, calls = self.run_with_stubs(
            "k3d_create_local_cluster ph img random 8080 8443",
            sysctls={**KERNEL_DEFAULTS, "vm/overcommit_memory": "0"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("vm.overcommit_memory=1 (now 0)", result.stderr)
        self.assertNotIn("cluster create", calls)

    def test_missing_kernel_setting_fails_before_k3d(self) -> None:
        sysctls = {k: v for k, v in KERNEL_DEFAULTS.items() if k != "kernel/panic"}
        result, calls = self.run_with_stubs(
            "k3d_create_local_cluster ph img random 8080 8443", sysctls=sysctls
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("kernel.panic=10 (now unreadable)", result.stderr)
        self.assertNotIn("cluster create", calls)

    def test_every_server_flag_exists_in_the_pinned_k3s(self) -> None:
        result = bash(f'source "{HELPER}"\nplatform_k3s_image\nplatform_k3s_server_args')
        self.assertEqual(result.returncode, 0, result.stderr)
        image, *args = result.stdout.splitlines()
        tag = image.split(":", 1)[1].split("@", 1)[0]
        help_file = PLATFORM / "k3s" / f"server-help-{tag}.txt"
        self.assertTrue(
            help_file.is_file(), f"run: docker run --rm {image} server --help > {help_file}"
        )
        known = set(re.findall(r"^\s+(--[a-z0-9-]+)", help_file.read_text(), re.M))
        for arg in args:
            with self.subTest(arg=arg):
                self.assertIn(arg.split("=", 1)[0], known)

    def test_create_times_out_and_prints_the_server_log(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            write_stub(bin_dir, "k3d", 'printf "%s\\n" "$*" >> "$CALLS"\nexit 1\n')
            write_stub(
                bin_dir,
                "docker",
                '[ "$1" = info ] && { echo 2; exit 0; }\necho "level=fatal msg=boom"\n',
            )
            sysctl_root = bin_dir / "sys"
            write_sysctls(sysctl_root, KERNEL_DEFAULTS)
            result = bash(
                f'source "{HELPER}"\nplatform_sysctl_root="{sysctl_root}"\n'
                "k3d_create_local_cluster ph img random 8080 8443",
                env={
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                    "CALLS": str(bin_dir / "calls"),
                },
            )
            calls = (bin_dir / "calls").read_text()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--timeout 5m", calls)
        self.assertIn("level=fatal msg=boom", result.stderr)

    def test_config_files_named_by_the_args_exist(self) -> None:
        result = bash(f'source "{HELPER}"\nplatform_k3s_server_args')
        self.assertEqual(result.returncode, 0, result.stderr)
        named = re.findall(r"/etc/baukit/k3s/(\S+)", result.stdout)
        self.assertEqual(sorted(named), ["audit-policy.yaml", "pod-security.yaml"])
        for name in named:
            self.assertTrue((PLATFORM / "k3s" / name).is_file(), name)

    def test_snapshot_container_publishes_on_the_gateway_only(self) -> None:
        result, calls = self.run_with_stubs("run_snapshot_container c img /srv 9418 172.30.0.1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("-p 172.30.0.1:9418:9418", calls)
        self.assertIn("-v /srv:/srv/git:ro", calls)
        self.assertIn("--cap-drop ALL", calls)
        self.assertIsNone(re.search(r"-p (0\.0\.0\.0:)?9418", calls))

    def test_gateway_is_the_ipv4_entry(self) -> None:
        result, _ = self.run_with_stubs("k3d_gateway ph", docker_output="fd00::1 172.30.0.1 ")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "172.30.0.1")

    def test_missing_gateway_fails(self) -> None:
        result, _ = self.run_with_stubs("k3d_gateway ph", docker_output="fd00::1 ")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot determine", result.stderr)

    def test_lifecycle_scripts_use_the_helpers(self) -> None:
        for script in LIFECYCLE_SCRIPTS:
            text = script.read_text()
            with self.subTest(script=script.name):
                self.assertIn('source "$platform_dir/local-git-http.sh"', text)
                self.assertIsNone(WILDCARD_ADDRESS.search(text))
                self.assertNotIn("k3d cluster create", text)
                self.assertNotIn("docker run", text)
                self.assertNotIn("tar \\", text)

    def test_snapshot_image_runs_as_non_root(self) -> None:
        lines = (PLATFORM / "local-git-http.Dockerfile").read_text().splitlines()
        users = [line.split()[1] for line in lines if line.startswith("USER ")]
        self.assertTrue(users)
        self.assertNotIn(users[-1].split(":")[0], {"0", "root"})


class StatusSecretsTest(unittest.TestCase):
    def test_status_prints_locations_not_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            state.joinpath("secrets.env").write_text(
                "GRAFANA_ADMIN_USER=admin\nGRAFANA_ADMIN_PASSWORD=grafana-value\n"
                "RUSTFS_ACCESS_KEY=local-access\nRUSTFS_SECRET_KEY=rustfs-value\n"
            )
            result = bash(
                f'source "{PLATFORM / "platform-up.sh"}"\n'
                "require_tools() { :; }\ncluster_exists() { return 1; }\nstatus",
                env={"BAUKIT_PLATFORM_STATE_DIR": str(state)},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            for value in ("grafana-value", "rustfs-value", "local-access"):
                self.assertNotIn(value, result.stdout)
            self.assertIn("observability/grafana-admin", result.stdout)
            self.assertIn(str(state / "secrets.env"), result.stdout)


class RestoreTestScriptTest(unittest.TestCase):
    PRIMARY = "postgres-cluster-restore-test-1"

    def setUp(self) -> None:
        manifest = yaml.safe_load_all((PLATFORM / "postgres-cluster/restore-test.yaml").read_text())
        cronjob = next(doc for doc in manifest if doc["kind"] == "CronJob")
        pod = cronjob["spec"]["jobTemplate"]["spec"]["template"]["spec"]
        self.writer = pod["initContainers"][0]
        self.restorer = pod["containers"][0]
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.work = Path(directory.name)
        self.applied = self.work / "applied.yaml"
        self.pushed = self.work / "pushed.txt"
        self.exec_log = self.work / "exec.txt"
        self.sql = self.work / "sentinel.sql"
        self.sentinel_file = self.work / "sentinel"

    def container_env(self, container: dict[str, Any], **values: str) -> dict[str, str]:
        names = {item["name"] for item in container["env"]}
        for name in values:
            self.assertIn(name, names)
        return {
            **os.environ,
            "PATH": f"{self.work}{os.pathsep}{os.environ['PATH']}",
            "POD_NAMESPACE": "postgres",
            "SENTINEL_FILE": str(self.sentinel_file),
            **values,
        }

    def run_script(
        self, container: dict[str, Any], env: dict[str, str]
    ) -> subprocess.CompletedProcess[str]:
        return run_process(
            ["sh", "-ec", textwrap.dedent(container["args"][0])],
            env=env,
            check=False,
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
        )

    def writer_env(self) -> dict[str, str]:
        return self.container_env(
            self.writer,
            POD_NAME="postgres-restore-test-abc",
            PGHOST="postgres-cluster-rw",
            PGDATABASE="product",
            PGUSER="product_owner",
            PGPASSWORD="secret",
        )

    def stub_pg_isready(self, failures: int) -> Path:
        calls = self.work / "pg_isready.calls"
        write_stub(
            self.work,
            "pg_isready",
            f'echo x >> "{calls}"\n[ "$(wc -l < "{calls}")" -gt {failures} ]\n',
        )
        write_stub(self.work, "sleep", "")
        return calls

    def stub_psql(self) -> None:
        write_stub(
            self.work, "psql", f'printf "%s\\n" "$*" > "{self.sql}.args"\ncat > "{self.sql}"\n'
        )

    def write_sentinel(self) -> str:
        self.stub_pg_isready(failures=0)
        self.stub_psql()
        result = self.run_script(self.writer, self.writer_env())
        self.assertEqual(result.returncode, 0, result.stderr)
        return self.sentinel_file.read_text().strip()

    def test_writer_waits_until_postgres_accepts_connections(self):
        calls = self.stub_pg_isready(failures=2)
        self.stub_psql()
        result = self.run_script(self.writer, self.writer_env())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls.read_text().splitlines()), 3)
        self.assertTrue(self.sql.exists())

    def test_writer_gives_up_when_postgres_stays_unreachable(self):
        self.stub_pg_isready(failures=1000)
        self.stub_psql()
        result = self.run_script(self.writer, self.writer_env())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("postgres unreachable after 30 tries", result.stderr)
        self.assertFalse(self.sql.exists())

    def run_restore(
        self, storage_class: str, restored_token: str
    ) -> subprocess.CompletedProcess[str]:
        write_stub(
            self.work,
            "kubectl",
            f"""case "$1 $2" in
  "apply -f") cat >> "{self.applied}"; echo --- >> "{self.applied}" ;;
  "-n postgres")
    case "$3 $4" in
      "get backup") printf completed ;;
      "get cluster") printf {self.PRIMARY} ;;
      "exec -i")
        printf '%s ' "$@" > "{self.exec_log}"
        cat >> "{self.exec_log}"
        printf '%s' {restored_token!r} ;;
    esac ;;
esac
""",
        )
        write_stub(self.work, "wget", f'cat > "{self.pushed}"\n')
        env = self.container_env(
            self.restorer,
            RESTORE_STORAGE_CLASS=storage_class,
            JOB_NAME="postgres-restore-test-29000000",
            JOB_UID="0b5c9a8e-1111-4222-8333-444455556666",
            BACKUP_TIMEOUT_SECONDS="5",
            POLL_SECONDS="0",
            PGDATABASE="product",
            PGUSER="product_owner",
            PGPASSWORD="secret",
        )
        return self.run_script(self.restorer, env)

    def applied_docs(self) -> list[dict[str, Any]]:
        return [doc for doc in yaml.safe_load_all(self.applied.read_text()) if doc]

    def restored_cluster(self) -> dict[str, Any]:
        return next(doc for doc in self.applied_docs() if doc["kind"] == "Cluster")

    def test_scheduled_backup_runs_once_at_creation(self) -> None:
        schedule = yaml.safe_load((PLATFORM / "postgres-cluster/scheduledbackup.yaml").read_text())
        self.assertIs(schedule["spec"]["immediate"], True)

    def test_writer_upserts_a_fresh_sentinel_row_with_product_credentials(self) -> None:
        token = self.write_sentinel()
        self.assertRegex(token, r"^\d{8}T\d{6}Z-postgres-restore-test-abc$")
        self.assertIn(f"token={token}", (self.sql.parent / "sentinel.sql.args").read_text())
        sql = self.sql.read_text()
        self.assertIn("CREATE TABLE IF NOT EXISTS baukit_restore_sentinel", sql)
        self.assertIn(
            "ON CONFLICT (id) DO UPDATE SET token = EXCLUDED.token, written_at = now()", sql
        )
        env = {item["name"]: item for item in self.writer["env"]}
        self.assertEqual(
            env["PGUSER"]["valueFrom"]["secretKeyRef"]["name"], "product-db-credentials"
        )
        self.assertEqual(
            env["PGPASSWORD"]["valueFrom"]["secretKeyRef"]["name"], "product-db-credentials"
        )

    def test_restored_sentinel_round_trip_pushes_the_metric(self) -> None:
        token = self.write_sentinel()
        result = self.run_restore("", token)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"restored sentinel {token} matches", result.stdout)
        backup = next(doc for doc in self.applied_docs() if doc["kind"] == "Backup")
        self.assertEqual(
            backup["spec"],
            {
                "method": "plugin",
                "cluster": {"name": "postgres-cluster"},
                "pluginConfiguration": {"name": "barman-cloud.cloudnative-pg.io"},
            },
        )
        exec_call = self.exec_log.read_text()
        self.assertIn(f"exec -i {self.PRIMARY} -c postgres", exec_call)
        self.assertTrue(exec_call.endswith("secret\n"), "the password goes over stdin")
        self.assertIn("baukit_restore_test_last_success_timestamp_seconds", self.pushed.read_text())

    def test_backup_is_owned_by_the_job_from_the_downward_api(self) -> None:
        token = self.write_sentinel()
        result = self.run_restore("", token)
        self.assertEqual(result.returncode, 0, result.stderr)
        backup = next(doc for doc in self.applied_docs() if doc["kind"] == "Backup")
        self.assertEqual(backup["metadata"]["labels"], {"baukit.dev/restore-test": "true"})
        self.assertEqual(
            backup["metadata"]["ownerReferences"],
            [
                {
                    "apiVersion": "batch/v1",
                    "kind": "Job",
                    "name": "postgres-restore-test-29000000",
                    "uid": "0b5c9a8e-1111-4222-8333-444455556666",
                    "controller": False,
                    "blockOwnerDeletion": False,
                }
            ],
        )
        env = {item["name"]: item for item in self.restorer["env"]}
        self.assertEqual(
            env["JOB_NAME"]["valueFrom"]["fieldRef"]["fieldPath"],
            "metadata.labels['batch.kubernetes.io/job-name']",
        )
        self.assertEqual(
            env["JOB_UID"]["valueFrom"]["fieldRef"]["fieldPath"],
            "metadata.labels['batch.kubernetes.io/controller-uid']",
        )
        cronjob = next(
            doc
            for doc in yaml.safe_load_all(
                (PLATFORM / "postgres-cluster/restore-test.yaml").read_text()
            )
            if doc["kind"] == "CronJob"
        )
        self.assertEqual(cronjob["spec"]["successfulJobsHistoryLimit"], 1)
        self.assertEqual(cronjob["spec"]["failedJobsHistoryLimit"], 2)

    def test_missing_sentinel_fails_without_pushing(self) -> None:
        self.write_sentinel()
        result = self.run_restore("", "")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("sentinel missing in the restored cluster", result.stderr)
        self.assertFalse(self.pushed.exists())

    def test_old_sentinel_fails_without_pushing(self) -> None:
        self.write_sentinel()
        result = self.run_restore("", "20260101T000000Z-postgres-restore-test-old")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match", result.stderr)
        self.assertFalse(self.pushed.exists())

    def test_unwritten_sentinel_fails_before_the_backup(self) -> None:
        self.sentinel_file.write_text("")
        result = self.run_restore("", "")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no sentinel was written", result.stderr)
        self.assertFalse(self.applied.exists())

    def test_restore_cluster_uses_the_configured_storage_class(self) -> None:
        token = self.write_sentinel()
        self.assertEqual(self.run_restore("hcloud-volumes-delete", token).returncode, 0)
        cluster = self.restored_cluster()
        self.assertEqual(cluster["metadata"]["name"], "postgres-cluster-restore-test")
        self.assertIs(cluster["spec"]["enablePDB"], False)
        self.assertEqual(
            cluster["spec"]["storage"],
            {"size": "10Gi", "storageClass": "hcloud-volumes-delete"},
        )

    def test_restore_cluster_defaults_to_the_default_storage_class(self) -> None:
        token = self.write_sentinel()
        self.assertEqual(self.run_restore("", token).returncode, 0)
        self.assertEqual(self.restored_cluster()["spec"]["storage"], {"size": "10Gi"})


class K3sPolicyFilesTest(unittest.TestCase):
    def load(self, name: str) -> Any:
        return yaml.safe_load((PLATFORM / "k3s" / name).read_text())

    def test_unlabeled_namespaces_enforce_baseline_and_only_kube_system_is_exempt(self) -> None:
        config = self.load("pod-security.yaml")
        self.assertEqual(config["kind"], "AdmissionConfiguration")
        (plugin,) = config["plugins"]
        self.assertEqual(plugin["name"], "PodSecurity")
        pod_security = plugin["configuration"]
        self.assertEqual(
            {mode: pod_security["defaults"][mode] for mode in ("enforce", "audit", "warn")},
            {"enforce": "baseline", "audit": "restricted", "warn": "restricted"},
        )
        exemptions = pod_security["exemptions"]
        self.assertEqual(exemptions["namespaces"], ["kube-system"])
        self.assertEqual(exemptions["usernames"], [])
        self.assertEqual(exemptions["runtimeClasses"], [])

    @staticmethod
    def matches(rule: dict[str, Any], request: dict[str, Any]) -> bool:
        if "path" in request:
            return any(fnmatch(request["path"], url) for url in rule.get("nonResourceURLs", []))
        if "nonResourceURLs" in rule:
            return False
        subjects = set(rule.get("users", [])) | set(rule.get("userGroups", []))
        return all(
            (
                request["verb"] in rule.get("verbs", [request["verb"]]),
                not subjects or bool(subjects & {request["user"], *request["groups"]}),
                any(
                    entry["group"] == request["group"]
                    and request["resource"] in entry.get("resources", [request["resource"]])
                    for entry in rule.get("resources", [request])
                ),
            )
        )

    def first_level(self, request: dict[str, Any]) -> str:
        rules = self.load("audit-policy.yaml")["rules"]
        return next((rule["level"] for rule in rules if self.matches(rule, request)), "None")

    def request(self, **overrides: Any) -> dict[str, Any]:
        base = {
            "verb": "create",
            "user": "alice",
            "groups": ["system:authenticated"],
            "group": "",
            "resource": "pods",
        }
        return {**base, **overrides}

    def test_secret_bodies_never_reach_the_log(self) -> None:
        for verb in ("create", "get", "patch"):
            with self.subTest(verb=verb):
                level = self.first_level(self.request(verb=verb, resource="secrets"))
                self.assertEqual(level, "Metadata")

    def test_rbac_writes_keep_the_request_body(self) -> None:
        request = self.request(group="rbac.authorization.k8s.io", resource="clusterrolebindings")
        self.assertEqual(self.first_level(request), "Request")
        self.assertEqual(self.first_level({**request, "verb": "get"}), "Metadata")

    def test_control_plane_reads_are_dropped_but_its_writes_are_kept(self) -> None:
        node = self.request(verb="watch", user="system:node:server-0", groups=["system:nodes"])
        self.assertEqual(self.first_level(node), "None")
        self.assertEqual(self.first_level({**node, "verb": "patch"}), "Metadata")
        user_read = self.request(verb="list")
        self.assertEqual(self.first_level(user_read), "Metadata")

    def test_health_probes_and_leases_are_dropped(self) -> None:
        self.assertEqual(self.first_level({"path": "/readyz"}), "None")
        self.assertEqual(self.first_level({"path": "/livez/etcd"}), "None")
        lease = self.request(verb="update", group="coordination.k8s.io", resource="leases")
        self.assertEqual(self.first_level(lease), "None")

    def test_policy_skips_the_request_received_stage(self) -> None:
        self.assertEqual(self.load("audit-policy.yaml")["omitStages"], ["RequestReceived"])


if __name__ == "__main__":
    unittest.main()

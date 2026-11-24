from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from fnmatch import fnmatch
from pathlib import Path
from typing import Any

import yaml
from process_runner import run_process, start_process

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
    "fs/inotify/max_user_instances": "512",
    "fs/inotify/max_user_watches": "524288",
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

    def test_inotify_limits_are_minimums(self) -> None:
        result, calls = self.run_with_stubs(
            "k3d_create_local_cluster ph img random 8080 8443",
            sysctls={
                **KERNEL_DEFAULTS,
                "fs/inotify/max_user_instances": "1024",
                "fs/inotify/max_user_watches": "1048576",
            },
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cluster create", calls)

    def test_low_inotify_limits_fail_before_k3d(self) -> None:
        result, calls = self.run_with_stubs(
            "k3d_create_local_cluster ph img random 8080 8443",
            sysctls={**KERNEL_DEFAULTS, "fs/inotify/max_user_instances": "128"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("fs.inotify.max_user_instances>=512 (now 128)", result.stderr)
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
        self.assertEqual(
            sorted(named), ["audit-policy.yaml", "audit-webhook.yaml", "pod-security.yaml"]
        )
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


ETCD_ARGS = [
    "--k3s-arg",
    "--cluster-init@server:0",
    "--k3s-arg",
    "--kube-controller-manager-arg=leader-elect=false@server:0",
    "--k3s-arg",
    "--kube-cloud-controller-manager-arg=leader-elect=false@server:0",
    "--k3s-arg",
    "--kube-scheduler-arg=leader-elect=false@server:0",
    "--env",
    "CATTLE_ELECTION_LEASE_DURATION=180s@server:0",
    "--env",
    "CATTLE_ELECTION_RENEW_DEADLINE=150s@server:0",
    "--env",
    "CATTLE_ELECTION_RETRY_PERIOD=10s@server:0",
]
KUBELET_ARGS = [
    "--k3s-arg",
    "--kubelet-arg=eviction-hard=memory.available<100Mi,nodefs.available<20Gi,imagefs.available<20Gi@server:0",
    "--k3s-arg",
    "--kubelet-arg=eviction-minimum-reclaim=nodefs.available=2Gi,imagefs.available=2Gi@server:0",
]
ETCD_CALLERS = (
    PLATFORM / "platform-up.sh",
    PLATFORM / "platform-lifecycle.sh",
    PLATFORM / "e2e" / "run.sh",
)


class LocalDatastoreTest(unittest.TestCase):
    def test_helper_prints_etcd_without_lease_loss_exits(self) -> None:
        result = bash(f'source "{HELPER}"\nplatform_local_etcd_args')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ETCD_ARGS)

    def test_platform_up_creates_the_cluster_with_the_helper_args(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = bash(
                f'source "{PLATFORM / "platform-up.sh"}"\n'
                "baukit_use_tools() { :; }\nrequire_tools() { :; }\nensure_secret_state() { :; }\n"
                "registry_mirrors_running() { return 1; }\n"
                "cluster_exists() { return 1; }\n"
                'k3d_create_local_cluster() { printf "%s\\n" "$@"; exit 0; }\nup',
                env={"BAUKIT_PLATFORM_STATE_DIR": directory},
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("\n".join(ETCD_ARGS) + "\n", result.stdout)
        self.assertIn("\n".join(KUBELET_ARGS) + "\n", result.stdout)

    def test_every_caller_uses_the_helper(self) -> None:
        for caller in ETCD_CALLERS:
            with self.subTest(caller=caller.name):
                source = caller.read_text()
                self.assertIn("mapfile -t etcd_args < <(platform_local_etcd_args)", source)
                self.assertIn('"${etcd_args[@]}"', source)
                self.assertNotIn("cluster-init", source)

    def test_kubelet_helper_prints_absolute_eviction_thresholds(self) -> None:
        result = bash(f'source "{HELPER}"\nplatform_local_kubelet_args')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), KUBELET_ARGS)
        for value in KUBELET_ARGS[1::2]:
            argument, node_filter = value.rsplit("@", 1)
            self.assertEqual(node_filter, "server:0")
            self.assertNotIn("@", argument)

    def test_every_caller_passes_the_kubelet_args(self) -> None:
        for caller in ETCD_CALLERS:
            with self.subTest(caller=caller.name):
                source = caller.read_text()
                self.assertIn("mapfile -t kubelet_args < <(platform_local_kubelet_args)", source)
                self.assertIn('"${kubelet_args[@]}"', source)
                self.assertNotIn("eviction-", source)

    def test_shared_and_etcd_args_keep_the_stock_eviction_policy(self) -> None:
        for helper in ("platform_k3s_server_args", "platform_local_etcd_args"):
            with self.subTest(helper=helper):
                result = bash(f'source "{HELPER}"\n{helper}')
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertNotIn("eviction", result.stdout)

    def test_shared_server_args_leave_the_datastore_alone(self) -> None:
        result = bash(f'source "{HELPER}"\nplatform_k3s_server_args')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("cluster-init", result.stdout)
        self.assertNotIn("leader-elect", result.stdout)


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


class RegistryMirrorClusterTest(unittest.TestCase):
    CASES = (
        ("platform-up.sh", "up", "down"),
        ("platform-lifecycle.sh", "ensure_cluster", "nuke"),
    )
    STUBS = textwrap.dedent(
        """
        source "{mirrors}"
        calls=$CALLS
        docker() {{
          printf 'docker %s\\n' "$*" >>"$calls"
          if [[ $1 == container ]]; then
            [[ $MIRRORS == running ]] && printf 'true\\n' && return 0
            return 1
          fi
        }}
        k3d() {{ printf 'k3d %s\\n' "$*" >>"$calls"; }}
        k3d_create_local_cluster() {{
          printf 'create %s\\n' "$*" >>"$calls"
          local previous=""
          for argument in "$@"; do
            [[ $previous == --registry-config ]] && cat "$argument" >>"$calls.registries"
            previous=$argument
          done
          return 0
        }}
        cluster_exists() {{ [[ $EXISTS == yes ]]; }}
        require_tools() {{ :; }}
        baukit_use_tools() {{ :; }}
        platform_local_etcd_args() {{ printf -- "--k3s-arg\\n--cluster-init@server:0\\n"; }}
        ensure_secret_state() {{ :; }}
        platform_check_free_subnet() {{ :; }}
        platform_check_cluster_network() {{ :; }}
        platform_check_network_in_node_cidr() {{ :; }}
        validate_common_config() {{ :; }}
        stop_git_daemon() {{ :; }}
        remove_snapshot() {{ :; }}
        use_context() {{ exit 0; }}
        use_cluster() {{ exit 0; }}
        cluster_name=demo k3s_image=k3s api_port=1 http_port=2 https_port=3
        k3d_subnet=10.0.0.0/24 state_dir=/nonexistent snapshot_image=snap repo_root=/nonexistent
        read -r -a extra_registry_mirrors <<<"$EXTRA"
        """
    )
    LAB = "registry.lab.example=http://10.43.200.10:5000"

    def run_function(
        self, script: str, function: str, *, mirrors: str, exists: str, extra: str = ""
    ) -> str:
        text = (PLATFORM / script).read_text()
        match = re.search(rf"^{function}\(\) {{\n.*?^}}\n", text, re.MULTILINE | re.DOTALL)
        assert match is not None
        with tempfile.TemporaryDirectory() as directory:
            calls = Path(directory) / "calls"
            calls.touch()
            stubs = self.STUBS.format(mirrors=PLATFORM / "registry-mirrors.sh")
            result = bash(
                f"{stubs}\n{match.group(0)}\n{function}",
                env={"CALLS": str(calls), "MIRRORS": mirrors, "EXISTS": exists, "EXTRA": extra},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.registries = (
                yaml.safe_load(Path(f"{calls}.registries").read_text())
                if Path(f"{calls}.registries").exists()
                else None
            )
            return calls.read_text()

    def test_extra_mirrors_join_the_running_mirrors(self) -> None:
        self.run_function(
            "platform-lifecycle.sh",
            "ensure_cluster",
            mirrors="running",
            exists="no",
            extra=self.LAB,
        )
        assert self.registries is not None
        mirrors = self.registries["mirrors"]
        self.assertEqual(
            mirrors["registry.lab.example"], {"endpoint": ["http://10.43.200.10:5000"]}
        )
        self.assertEqual(
            mirrors["ghcr.io"], {"endpoint": ["http://k3d-baukit-mirror-ghcr-io:5000"]}
        )

    def test_extra_mirrors_alone_when_the_baukit_mirrors_are_absent(self) -> None:
        line = self.created_with(
            self.run_function(
                "platform-lifecycle.sh",
                "ensure_cluster",
                mirrors="absent",
                exists="no",
                extra=self.LAB,
            )
        )
        self.assertIn("--registry-config", line)
        self.assertNotIn("--registry-use", line)
        self.assertEqual(
            self.registries,
            {"mirrors": {"registry.lab.example": {"endpoint": ["http://10.43.200.10:5000"]}}},
        )

    def test_existing_cluster_starts_without_registry_config(self) -> None:
        calls = self.run_function(
            "platform-lifecycle.sh",
            "ensure_cluster",
            mirrors="absent",
            exists="yes",
            extra=self.LAB,
        )
        self.assertIn("k3d cluster start demo", calls)
        self.assertIsNone(self.registries)

    def created_with(self, calls: str) -> str:
        return next(line for line in calls.splitlines() if line.startswith("create "))

    def test_running_mirrors_are_passed_to_the_new_cluster(self) -> None:
        for script, create, _ in self.CASES:
            with self.subTest(script=script):
                line = self.created_with(
                    self.run_function(script, create, mirrors="running", exists="no")
                )
                self.assertIn("--registry-config", line)
                self.assertIn("--registry-use k3d-baukit-mirror-ghcr-io:5000", line)

    def test_cluster_is_created_without_mirrors_when_they_are_absent(self) -> None:
        for script, create, _ in self.CASES:
            with self.subTest(script=script):
                line = self.created_with(
                    self.run_function(script, create, mirrors="absent", exists="no")
                )
                self.assertNotIn("--registry-config", line)
                self.assertNotIn("--registry-use", line)

    def test_mirrors_are_detached_before_the_cluster_is_deleted(self) -> None:
        for script, _, delete in self.CASES:
            with self.subTest(script=script):
                calls = self.run_function(
                    script, delete, mirrors="running", exists="yes"
                ).splitlines()
                disconnect = next(
                    i
                    for i, c in enumerate(calls)
                    if c.startswith("docker network disconnect k3d-demo ")
                )
                self.assertLess(disconnect, calls.index("k3d cluster delete demo"))


class ExtraRegistryMirrorCheckTest(unittest.TestCase):
    def check(self, *entries: str) -> subprocess.CompletedProcess[str]:
        quoted = " ".join(f"'{entry}'" for entry in entries)
        return bash(
            f'source "{PLATFORM / "registry-mirrors.sh"}"\nregistry_mirrors_check_extra {quoted}'
        )

    def test_valid_entries_pass(self) -> None:
        result = self.check(
            "registry.lab.example=http://10.43.200.10:5000",
            "cache.example:5000=https://cache.example",
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_no_entries_pass(self) -> None:
        self.assertEqual(self.check().returncode, 0)

    def test_malformed_entries_fail(self) -> None:
        for entry in (
            "registry.lab.example",
            "registry.lab.example=10.43.200.10:5000",
            "registry.lab.example=http://10.43.200.10:5000/v2",
            "Registry_Lab=http://10.43.200.10:5000",
        ):
            with self.subTest(entry=entry):
                result = self.check(entry)
                self.assertEqual(result.returncode, 2)
                self.assertIn("must be host=http(s)://address[:port]", result.stderr)

    def test_baukit_mirror_hosts_cannot_be_overridden(self) -> None:
        for host in ("docker.io", "reg.kyverno.io"):
            with self.subTest(host=host):
                result = self.check(f"{host}=http://10.0.0.1:5000")
                self.assertEqual(result.returncode, 2)
                self.assertIn("overrides a baukit mirror", result.stderr)

    def test_duplicate_hosts_fail(self) -> None:
        result = self.check("a.example=http://10.0.0.1:5000", "a.example=http://10.0.0.2:5000")
        self.assertEqual(result.returncode, 2)
        self.assertIn("listed twice", result.stderr)

    def test_lifecycle_checks_the_setting_before_any_command(self) -> None:
        text = (PLATFORM / "platform-lifecycle.sh").read_text()
        body = re.search(
            r"^validate_common_config\(\) \{\n.*?^\}\n", text, re.MULTILINE | re.DOTALL
        )
        assert body is not None
        self.assertIn('registry_mirrors_check_extra "${extra_registry_mirrors[@]}"', body.group(0))
        self.assertIn(
            'read -r -a extra_registry_mirrors <<<"${PLATFORM_EXTRA_REGISTRY_MIRRORS:-}"', text
        )


class LifecycleSubnetTest(unittest.TestCase):
    """platform-lifecycle.sh keeps platform-local inside the nodeCIDR; docker and k3d are stubs."""

    DOCKER = textwrap.dedent(
        """
        case "$1 $2" in
          "network ls")
            if [[ ${3:-} == -q && ${5:-} == "name=^k3d-demo\\$" ]]; then
              [[ -z ${LEFTOVER:-} ]] || printf 'leftover-id\\n'
            else
              printf 'n1\\nn2\\n'
            fi ;;
          "network inspect")
            if [[ $3 == k3d-* ]]; then
              printf '%s\\n' "$OWN_SUBNETS"
            else
              printf '%b' "$NETWORKS"
              if [[ -n ${VANISHED:-} ]]; then
                printf 'Error response from daemon: network %s not found\\n' "$VANISHED" >&2
                exit 1
              fi
            fi ;;
        esac
        """
    )
    STUBS = textwrap.dedent(
        """
        source "{helper}"
        cluster_exists() {{ [[ $EXISTS == yes ]]; }}
        registry_mirrors_running() {{ return 1; }}
        extra_registry_mirrors=()
        k3d_create_local_cluster() {{ printf 'create %s\\n' "$*" >>"$CALLS"; exit 0; }}
        k3d() {{ printf 'k3d %s\\n' "$*" >>"$CALLS"; exit 0; }}
        cluster_name=demo k3s_image=k3s api_port=1 http_port=2 https_port=3
        """
    )

    def run_ensure_cluster(
        self, *, exists: bool, networks: str = "", own: str = "", env: dict[str, str] | None = None
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        text = (PLATFORM / "platform-lifecycle.sh").read_text()
        subnet = re.search(r"^k3d_subnet=.*$", text, re.MULTILINE)
        function = re.search(r"^ensure_cluster\(\) {\n.*?^}\n", text, re.MULTILINE | re.DOTALL)
        assert subnet is not None and function is not None
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            calls = bin_dir / "calls"
            write_stub(bin_dir, "docker", self.DOCKER)
            result = bash(
                self.STUBS.format(helper=HELPER)
                + f"{subnet.group(0)}\n{function.group(0)}\nensure_cluster",
                env={
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                    "CALLS": str(calls),
                    "EXISTS": "yes" if exists else "no",
                    "NETWORKS": networks,
                    "OWN_SUBNETS": own,
                    **(env or {}),
                },
            )
            return result, calls.read_text() if calls.exists() else ""

    def test_new_cluster_uses_the_default_subnet_inside_the_node_cidr(self) -> None:
        result, calls = self.run_ensure_cluster(exists=False, networks="bridge 172.17.0.0/16\\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--subnet 10.89.1.0/24", calls)

    def test_subnet_override_is_passed_to_k3d(self) -> None:
        result, calls = self.run_ensure_cluster(
            exists=False, env={"PLATFORM_SUBNET": "10.89.7.0/24"}
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--subnet 10.89.7.0/24", calls)

    def test_overlapping_docker_network_fails_before_k3d(self) -> None:
        result, calls = self.run_ensure_cluster(
            exists=False, networks="k3d-baukit-local 10.89.0.0/24\\nother 10.89.0.0/20\\n"
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("overlaps the Docker network(s) other (10.89.0.0/20)", result.stderr)
        self.assertNotIn("create ", calls)

    def test_a_network_removed_during_the_check_does_not_block_create(self) -> None:
        result, calls = self.run_ensure_cluster(
            exists=False, networks="bridge 172.17.0.0/16\\n", env={"VANISHED": "n2"}
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--subnet 10.89.1.0/24", calls)

    def test_overlap_is_found_while_another_network_vanishes(self) -> None:
        result, calls = self.run_ensure_cluster(
            exists=False, networks="other 10.89.1.0/24\\n", env={"VANISHED": "n2"}
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("overlaps the Docker network(s) other (10.89.1.0/24)", result.stderr)
        self.assertNotIn("create ", calls)

    def test_subnet_outside_the_node_cidr_fails(self) -> None:
        result, calls = self.run_ensure_cluster(
            exists=False, env={"PLATFORM_SUBNET": "172.30.0.0/24"}
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not inside the nodeCIDR 10.89.0.0/16", result.stderr)
        self.assertNotIn("create ", calls)

    def test_leftover_network_on_another_subnet_fails_before_k3d(self) -> None:
        result, calls = self.run_ensure_cluster(
            exists=False,
            networks="k3d-demo 172.18.0.0/16\\n",
            own="172.18.0.0/16",
            env={"LEFTOVER": "yes"},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("leftover Docker network k3d-demo uses 172.18.0.0/16", result.stderr)
        self.assertIn("docker network rm k3d-demo", result.stderr)
        self.assertNotIn("create ", calls)
        self.assertNotIn("network rm", calls)

    def test_leftover_network_without_ipv4_fails(self) -> None:
        result, calls = self.run_ensure_cluster(
            exists=False, own="fd00::/64", env={"LEFTOVER": "yes"}
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("uses no IPv4 subnet", result.stderr)
        self.assertNotIn("create ", calls)

    def test_leftover_network_on_the_platform_subnet_is_reused(self) -> None:
        result, calls = self.run_ensure_cluster(
            exists=False,
            networks="k3d-demo 10.89.1.0/24\\n",
            own="10.89.1.0/24 fd00::/64",
            env={"LEFTOVER": "yes"},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--subnet 10.89.1.0/24", calls)

    def test_existing_cluster_inside_the_node_cidr_starts(self) -> None:
        result, calls = self.run_ensure_cluster(exists=True, own="10.89.1.0/24 fd00::/64")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("k3d cluster start demo", calls)

    def test_existing_cluster_outside_the_node_cidr_fails_with_guidance(self) -> None:
        result, calls = self.run_ensure_cluster(exists=True, own="172.18.0.0/16")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("k3d-demo uses 172.18.0.0/16", result.stderr)
        self.assertIn("run recreate", result.stderr)
        self.assertNotIn("cluster start", calls)


class SystemNamespaceLabelTest(unittest.TestCase):
    CALLERS = (*LIFECYCLE_SCRIPTS, PLATFORM / "e2e/run.sh")

    def test_labels_all_system_namespaces(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            calls = Path(directory) / "calls"
            result = bash(
                f'source "{HELPER}"\n'
                f'kubectl() {{ printf "%s\\n" "$*" >>"{calls}"; '
                f'[[ $1 != apply ]] || cat >>"{calls}"; }}\n'
                "platform_label_system_namespaces"
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = calls.read_text().splitlines()
        levels = {line.split()[2]: line for line in lines if line.startswith("label ")}
        self.assertEqual(lines.count("apply -f -"), 3)
        self.assertEqual(lines.count("automountServiceAccountToken: false"), 3)
        accounts = {line.split()[1] for line in lines if line.startswith("  namespace: ")}
        self.assertEqual(accounts, {"default", "kube-public", "kube-node-lease"})
        self.assertEqual(set(levels), {"kube-system", "default", "kube-public", "kube-node-lease"})
        for namespace, line in levels.items():
            level = "privileged" if namespace == "kube-system" else "restricted"
            with self.subTest(namespace=namespace):
                self.assertIn("--overwrite", line)
                self.assertIn("enforce-version=v1.37", line)
                for mode in ("enforce", "audit", "warn"):
                    self.assertIn(f"pod-security.kubernetes.io/{mode}={level}", line)

    def test_every_cluster_entry_point_labels_them(self) -> None:
        for script in self.CALLERS:
            with self.subTest(script=script.name):
                self.assertRegex(script.read_text(), r"(?m)^\s+platform_label_system_namespaces$")


class RestoreTestScriptTest(unittest.TestCase):
    RESTORE_HOST = "postgres-cluster-restore-test-rw"

    def setUp(self) -> None:
        self.manifest = list(
            yaml.safe_load_all((PLATFORM / "postgres-cluster/restore-test.yaml").read_text())
        )
        cronjob = next(doc for doc in self.manifest if doc["kind"] == "CronJob")
        pod = cronjob["spec"]["jobTemplate"]["spec"]["template"]["spec"]
        self.writer = pod["initContainers"][0]
        self.restorer, self.reader = pod["containers"]
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.work = Path(directory.name)
        self.applied = self.work / "applied.yaml"
        self.pushed = self.work / "pushed.txt"
        self.kubectl_log = self.work / "kubectl.txt"
        self.read_log = self.work / "read.txt"
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
            "WORK_DIR": str(self.work),
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
            WAIT_SECONDS="300",
        )

    def stub_psql(self, probe_failures: int = 0) -> Path:
        probes = self.work / "psql-probe.calls"
        write_stub(
            self.work,
            "psql",
            f"""case "$*" in
  *"select 1"*)
    echo x >> "{probes}"
    [ "$(wc -l < "{probes}")" -gt {probe_failures} ] ;;
  *)
    printf "%s\\n" "$*" > "{self.sql}.args"
    cat > "{self.sql}" ;;
esac
""",
        )
        write_stub(self.work, "sleep", "")
        return probes

    def write_sentinel(self) -> str:
        self.stub_psql()
        result = self.run_script(self.writer, self.writer_env())
        self.assertEqual(result.returncode, 0, result.stderr)
        return self.sentinel_file.read_text().strip()

    def test_writer_waits_until_the_database_accepts_queries(self):
        probes = self.stub_psql(probe_failures=2)
        result = self.run_script(self.writer, self.writer_env())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(probes.read_text().splitlines()), 3)
        self.assertTrue(self.sql.exists())

    def test_writer_gives_up_when_the_database_stays_unreachable(self):
        self.stub_psql(probe_failures=1000)
        env = {**self.writer_env(), "WAIT_SECONDS": "0"}
        result = self.run_script(self.writer, env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("database product not reachable as product_owner after 0s", result.stderr)
        self.assertFalse(self.sql.exists())

    def stub_restored_psql(self, restored_token: str | None) -> None:
        query = (
            f'printf "%s\\n" "$PGHOST $PGUSER $PGDATABASE $PGSSLMODE $*" >> "{self.read_log}"\n'
            f"printf %s {restored_token!r}"
            if restored_token is not None
            else "echo 'relation \"baukit_restore_sentinel\" does not exist' >&2; exit 1"
        )
        write_stub(
            self.work,
            "psql",
            f"""case "$*" in
  *"select 1"*) ;;
  *) {query} ;;
esac
""",
        )

    def reader_env(self, **values: str) -> dict[str, str]:
        return self.container_env(
            self.reader,
            PGDATABASE="product",
            PGSSLMODE="require",
            PGUSER="product_owner",
            PGPASSWORD="secret",
            **{"WAIT_SECONDS": "300", "POLL_SECONDS": "0", **values},
        )

    def run_restore(
        self, storage_class: str, restored_token: str | None, *, reader: bool = True, **values: str
    ) -> subprocess.CompletedProcess[str]:
        write_stub(
            self.work,
            "kubectl",
            f"""printf "%s\\n" "$*" >> "{self.kubectl_log}"
case "$1 $2" in
  "apply -f") cat >> "{self.applied}"; echo --- >> "{self.applied}" ;;
  "-n postgres")
    case "$3 $4" in
      "get backup") printf completed ;;
    esac ;;
esac
""",
        )
        write_stub(self.work, "wget", f'cat > "{self.pushed}"\n')
        write_stub(self.work, "sleep", f"exec {shutil.which('sleep')} 0.01\n")
        self.stub_restored_psql(restored_token)
        env = self.container_env(
            self.restorer,
            RESTORE_STORAGE_CLASS=storage_class,
            JOB_NAME="postgres-restore-test-29000000",
            JOB_UID="0b5c9a8e-1111-4222-8333-444455556666",
            **{
                "BACKUP_TIMEOUT_SECONDS": "5",
                "READ_TIMEOUT_SECONDS": "30",
                "POLL_SECONDS": "0",
                **values,
            },
        )
        if not reader:
            return self.run_script(self.restorer, env)
        with start_process(
            ["sh", "-ec", textwrap.dedent(self.reader["args"][0])],
            env=self.reader_env(),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        ) as reading:
            result = self.run_script(self.restorer, env)
            _, self.reader_stderr = reading.communicate(timeout=30)
        self.reader_returncode = reading.returncode
        return result

    def kubectl_calls(self) -> list[str]:
        return self.kubectl_log.read_text().splitlines()

    def assert_restore_cluster_deleted(self) -> None:
        self.assertIn(
            "-n postgres delete cluster postgres-cluster-restore-test "
            "--ignore-not-found=true --wait=false",
            self.kubectl_calls(),
        )

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
        self.assertEqual(self.reader_returncode, 0, self.reader_stderr)
        self.assertIn("baukit_restore_test_last_success_timestamp_seconds", self.pushed.read_text())
        self.assert_restore_cluster_deleted()

    def test_sentinel_is_read_over_the_network_without_exec(self) -> None:
        token = self.write_sentinel()
        self.assertEqual(self.run_restore("", token).returncode, 0)
        self.assertFalse(any(" exec " in f" {call} " for call in self.kubectl_calls()))
        (read,) = self.read_log.read_text().splitlines()
        self.assertTrue(
            read.startswith(f"{self.RESTORE_HOST} product_owner product require "), read
        )
        self.assertIn("SELECT token FROM baukit_restore_sentinel WHERE id = 1", read)
        self.assertNotIn("secret", read)
        env = {item["name"]: item for item in self.reader["env"]}
        for name in ("PGUSER", "PGPASSWORD"):
            self.assertEqual(
                env[name]["valueFrom"]["secretKeyRef"]["name"], "product-db-credentials"
            )
        restorer_env = {item["name"] for item in self.restorer["env"]}
        self.assertFalse(restorer_env & {"PGUSER", "PGPASSWORD"})

    def test_role_cannot_exec_or_read_pods(self) -> None:
        role = next(doc for doc in self.manifest if doc["kind"] == "Role")
        resources = {resource for rule in role["rules"] for resource in rule["resources"]}
        self.assertEqual(resources, {"clusters", "backups"})

    def test_failed_read_fails_and_deletes_the_restore_cluster(self) -> None:
        self.write_sentinel()
        result = self.run_restore("", None)
        self.assertNotEqual(result.returncode, 0)
        reason = f"reading baukit_restore_sentinel from {self.RESTORE_HOST} failed"
        self.assertIn(f"read-sentinel failed: {reason}", result.stderr)
        self.assertNotEqual(self.reader_returncode, 0)
        self.assertFalse(self.pushed.exists())
        self.assert_restore_cluster_deleted()

    def test_silent_reader_times_out_and_deletes_the_restore_cluster(self) -> None:
        token = self.write_sentinel()
        result = self.run_restore("", token, reader=False, READ_TIMEOUT_SECONDS="0")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("read-sentinel did not report within 0s", result.stderr)
        self.assertFalse(self.pushed.exists())
        self.assert_restore_cluster_deleted()

    def test_reader_gives_up_when_the_restored_database_stays_unreachable(self) -> None:
        write_stub(self.work, "psql", "exit 2\n")
        write_stub(self.work, "sleep", "")
        (self.work / "restored-host").write_text(f"{self.RESTORE_HOST}\n")
        result = self.run_script(self.reader, self.reader_env(WAIT_SECONDS="0"))
        self.assertNotEqual(result.returncode, 0)
        message = f"database product on {self.RESTORE_HOST} not reachable as product_owner after 0s"
        self.assertIn(message, result.stderr)
        self.assertEqual((self.work / "read-failed").read_text().strip(), message)

    def test_reader_exits_when_the_restore_ends_before_the_cluster_is_ready(self) -> None:
        write_stub(self.work, "psql", "exit 2\n")
        (self.work / "restore-done").touch()
        result = self.run_script(self.reader, self.reader_env())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.work / "read-failed").exists())

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
        self.assertEqual(self.reader_returncode, 0, self.reader_stderr)

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
        users, groups = rule.get("users"), rule.get("userGroups")
        return all(
            (
                request["verb"] in rule.get("verbs", [request["verb"]]),
                not users or request["user"] in users,
                not groups or bool(set(groups) & set(request["groups"])),
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

    def test_service_account_reads_keep_only_sensitive_resources(self) -> None:
        reader = self.request(
            verb="get",
            user="system:serviceaccount:e2e:reader",
            groups=["system:serviceaccounts", "system:serviceaccounts:e2e"],
        )
        self.assertEqual(self.first_level({**reader, "resource": "secrets"}), "Metadata")
        self.assertEqual(self.first_level(reader), "None")
        self.assertEqual(self.first_level({**reader, "verb": "patch"}), "Metadata")

    def test_exec_and_attach_are_logged_without_bodies(self) -> None:
        for resource in ("pods/exec", "pods/attach", "pods/portforward"):
            with self.subTest(resource=resource):
                self.assertEqual(self.first_level(self.request(resource=resource)), "Metadata")

    def test_webhook_targets_the_falco_k8saudit_service(self) -> None:
        server = self.load("audit-webhook.yaml")["clusters"][0]["cluster"]["server"]
        release = (PLATFORM / "falco/k8saudit-helm-release.yaml").read_text()
        cluster_ip = re.search(r"clusterIP: (\S+)", release)
        port = re.search(r"open_params: \"http://:(\d+)(/\S+)\"", release)
        assert cluster_ip is not None and port is not None
        self.assertEqual(server, f"http://{cluster_ip.group(1)}:{port.group(1)}{port.group(2)}")

    def test_access_review_self_checks_are_dropped(self) -> None:
        review = self.request(group="authorization.k8s.io", resource="selfsubjectaccessreviews")
        self.assertEqual(self.first_level(review), "None")

    def test_health_probes_and_leases_are_dropped(self) -> None:
        self.assertEqual(self.first_level({"path": "/readyz"}), "None")
        self.assertEqual(self.first_level({"path": "/livez/etcd"}), "None")
        lease = self.request(verb="update", group="coordination.k8s.io", resource="leases")
        self.assertEqual(self.first_level(lease), "None")

    def test_policy_skips_the_request_received_stage(self) -> None:
        self.assertEqual(
            self.load("audit-policy.yaml")["omitStages"], ["RequestReceived", "ResponseStarted"]
        )


if __name__ == "__main__":
    unittest.main()

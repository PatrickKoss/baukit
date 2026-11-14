from __future__ import annotations

import os
import re
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

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
        self, script: str, docker_output: str = ""
    ) -> tuple[subprocess.CompletedProcess[str], str]:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            calls = bin_dir / "calls"
            for tool in ("k3d", "docker"):
                write_stub(
                    bin_dir, tool, f'printf "%s\\n" "$*" >> "{calls}"\nprintf "%s" "$STUB_OUTPUT"\n'
                )
            result = bash(
                f'source "{HELPER}"\n{script}',
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
    def run_restore(self, storage_class: str) -> str:
        manifest = yaml.safe_load_all((PLATFORM / "postgres-cluster/restore-test.yaml").read_text())
        cronjob = next(doc for doc in manifest if doc["kind"] == "CronJob")
        container = cronjob["spec"]["jobTemplate"]["spec"]["template"]["spec"]["containers"][0]
        env_names = {item["name"] for item in container["env"]}
        self.assertIn("RESTORE_STORAGE_CLASS", env_names)
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            applied = bin_dir / "applied.yaml"
            write_stub(bin_dir, "kubectl", f'if [ "$1" = apply ]; then cat > "{applied}"; fi\n')
            write_stub(bin_dir, "wget", "cat >/dev/null\n")
            result = run_process(
                ["sh", "-ec", textwrap.dedent(container["args"][0])],
                env={
                    **os.environ,
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                    "POD_NAMESPACE": "postgres",
                    "RESTORE_STORAGE_CLASS": storage_class,
                },
                check=False,
                capture_output=True,
                text=True,
                stdin=subprocess.DEVNULL,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            return applied.read_text()

    def test_restore_cluster_uses_the_configured_storage_class(self) -> None:
        cluster = yaml.safe_load(self.run_restore("hcloud-volumes-delete"))
        self.assertEqual(cluster["metadata"]["name"], "postgres-cluster-restore-test")
        self.assertIs(cluster["spec"]["enablePDB"], False)
        self.assertEqual(
            cluster["spec"]["storage"], {"size": "10Gi", "storageClass": "hcloud-volumes-delete"}
        )

    def test_restore_cluster_defaults_to_the_default_storage_class(self) -> None:
        cluster = yaml.safe_load(self.run_restore(""))
        self.assertEqual(cluster["spec"]["storage"], {"size": "10Gi"})


if __name__ == "__main__":
    unittest.main()

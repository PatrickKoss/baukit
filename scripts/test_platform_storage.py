from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

PLATFORM = Path(__file__).resolve().parent.parent / "deploy/platform"


class RuntimeStorageCredentialsTest(unittest.TestCase):
    def run_shell(self, state: Path, script: str) -> str:
        result = subprocess.run(
            ["bash", "-ec", 'source "$PLATFORM_SCRIPT"\n' + script],
            env={
                **os.environ,
                "PLATFORM_SCRIPT": str(PLATFORM / "platform-up.sh"),
                "BAUKIT_PLATFORM_STATE_DIR": str(state),
            },
            check=True,
            capture_output=True,
            text=True,
        )
        return result.stdout

    def test_new_credentials_are_private_and_survive_convergence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "state"
            credentials = self.run_shell(state, """
                ensure_secret_state
                printf '%s %s\\n' "$RUSTFS_ACCESS_KEY" "$RUSTFS_SECRET_KEY"
                ensure_secret_state
                printf '%s %s\\n' "$RUSTFS_ACCESS_KEY" "$RUSTFS_SECRET_KEY"
            """).splitlines()
            self.assertEqual(len(credentials), 2)
            self.assertEqual(credentials[0], credentials[1])
            access_key, secret_key = credentials[0].split()
            self.assertRegex(access_key, r"^local-[0-9a-f]{12}$")
            self.assertRegex(secret_key, r"^[0-9a-f]{36}$")
            self.assertEqual(stat.S_IMODE(state.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE((state / "secrets.env").stat().st_mode), 0o600)

    def test_existing_state_gains_storage_credentials_without_rotating_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            state.joinpath("secrets.env").write_text("KEYCLOAK_ADMIN_PASSWORD=retained-password\n")
            result = self.run_shell(state, """
                ensure_secret_state
                printf '%s\\n' "$KEYCLOAK_ADMIN_PASSWORD" "$RUSTFS_ACCESS_KEY" "$RUSTFS_SECRET_KEY"
                ensure_secret_state
                printf '%s\\n' "$RUSTFS_ACCESS_KEY" "$RUSTFS_SECRET_KEY"
            """).splitlines()
            self.assertEqual(result[0], "retained-password")
            self.assertEqual(result[1:3], result[3:5])
            self.assertTrue(all(result[1:3]))

    def test_partial_storage_credentials_are_replaced_as_a_pair(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            state.joinpath("secrets.env").write_text("RUSTFS_ACCESS_KEY=partial-key\n")
            result = self.run_shell(state, """
                ensure_secret_state
                printf '%s\\n' "$RUSTFS_ACCESS_KEY" "$RUSTFS_SECRET_KEY"
            """).splitlines()
            self.assertEqual(len(result), 2)
            self.assertNotEqual(result[0], "partial-key")
            self.assertRegex(result[0], r"^local-[0-9a-f]{12}$")
            self.assertRegex(result[1], r"^[0-9a-f]{36}$")

    def test_consumers_receive_the_same_keys_in_their_required_secret_shapes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_shell(Path(directory), """
                ensure_secret_state
                RUSTFS_ACCESS_KEY=storage-access
                RUSTFS_SECRET_KEY=storage-secret
                kubectl() { cat >/dev/null; }
                apply_secret() { printf '%s\\n' "$*"; }
                apply_runtime_identity
            """).splitlines()
            expected = [
                "postgres postgres-backup-credentials --from-literal=ACCESS_KEY_ID=storage-access --from-literal=ACCESS_SECRET_KEY=storage-secret",
                "postgres rustfs-root --from-literal=username=storage-access --from-literal=password=storage-secret",
                "observability rustfs-root --from-literal=username=storage-access --from-literal=password=storage-secret",
                "posthog posthog-object-storage --from-literal=root-user=storage-access --from-literal=root-password=storage-secret",
            ]
            for secret in expected:
                self.assertIn(secret, result)


class BucketBootstrapTest(unittest.TestCase):
    def run_bootstrap(self, directory: Path, existing: str, create_status: int) -> subprocess.CompletedProcess[str]:
        manifest = (PLATFORM / "overlays/local/components/foundation/rustfs.yaml").read_text()
        body = manifest.split("            - |\n", 1)[1].split("          env:\n", 1)[0]
        commands = textwrap.dedent(body)
        directory.joinpath("aws").write_text("""#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$AWS_CALLS"
case "$*" in
  *head-bucket*) [ "$4" = "$EXISTING_BUCKET" ] ;;
  *create-bucket*) exit "$CREATE_STATUS" ;;
esac
""")
        directory.joinpath("curl").write_text("#!/bin/sh\nexit 0\n")
        for tool in ("aws", "curl"):
            directory.joinpath(tool).chmod(0o755)
        return subprocess.run(
            ["sh", "-ec", commands],
            env={
                **os.environ,
                "PATH": str(directory) + os.pathsep + os.environ["PATH"],
                "AWS_CALLS": str(directory / "calls"),
                "AWS_ENDPOINT_URL": "http://rustfs:9000",
                "EXISTING_BUCKET": existing,
                "CREATE_STATUS": str(create_status),
            },
            capture_output=True,
            text=True,
        )

    def test_existing_bucket_is_kept_and_missing_bucket_is_created(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            result = self.run_bootstrap(path, "baukit-local", 0)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = path.joinpath("calls").read_text().splitlines()
            self.assertIn("configure set default.s3.addressing_style path", calls)
            self.assertNotIn("s3api create-bucket --bucket baukit-local", calls)
            self.assertIn("s3api create-bucket --bucket posthog", calls)

    def test_bucket_creation_failure_stops_bootstrap(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            result = self.run_bootstrap(path, "", 42)
            self.assertEqual(result.returncode, 42)
            calls = path.joinpath("calls").read_text().splitlines()
            self.assertIn("s3api create-bucket --bucket baukit-local", calls)
            self.assertNotIn("s3api head-bucket --bucket posthog", calls)


if __name__ == "__main__":
    unittest.main()

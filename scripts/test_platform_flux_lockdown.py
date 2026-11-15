from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

PLATFORM = Path(__file__).resolve().parent.parent / "deploy/platform"
HELPER = PLATFORM / "flux-lockdown.sh"
COMPLETE = {
    "source-controller": [],
    "kustomize-controller": [
        "--no-cross-namespace-refs=true",
        "--no-remote-bases=true",
        "--default-service-account=default",
    ],
    "helm-controller": ["--no-cross-namespace-refs=true"],
    "notification-controller": [],
}
KUBECTL_STUB = """#!/usr/bin/env bash
set -eu
case "$*" in
  *"get deployment "*)
    name=$(printf '%s\\n' "$@" | sed -n '/^deployment$/{n;p}')
    file="$STUB_DIR/$name.args"
    [[ -f $file ]] || exit 1
    cat "$file"
    ;;
  *"get kustomizations"*)
    [[ -f $STUB_DIR/sync.sa ]] || exit 1
    cat "$STUB_DIR/sync.sa"
    ;;
  *) exit 3 ;;
esac
"""


class FluxLockdownVerifyTest(unittest.TestCase):
    def run_verify(
        self, args: dict[str, list[str]], sync_sa: str | None
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as directory:
            stub_dir = Path(directory)
            kubectl = stub_dir / "kubectl"
            kubectl.write_text(KUBECTL_STUB)
            kubectl.chmod(0o755)
            for name, flags in args.items():
                (stub_dir / f"{name}.args").write_text(
                    "".join(f"{flag}\n" for flag in ["--log-level=info", *flags])
                )
            if sync_sa is not None:
                (stub_dir / "sync.sa").write_text(sync_sa)
            return run_process(
                [
                    "bash",
                    "-euo",
                    "pipefail",
                    "-c",
                    f'source "{HELPER}"\nflux_lockdown_verify "{PLATFORM}"',
                ],
                env={
                    **os.environ,
                    "PATH": f"{stub_dir}:{os.environ['PATH']}",
                    "STUB_DIR": str(stub_dir),
                },
                check=False,
                capture_output=True,
                text=True,
                stdin=subprocess.DEVNULL,
            )

    def test_complete_lockdown_passes(self) -> None:
        result = self.run_verify(COMPLETE, "kustomize-controller")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("lockdown is active", result.stdout)

    def test_install_without_bootstrap_kustomization_passes(self) -> None:
        result = self.run_verify(COMPLETE, None)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_each_missing_flag_fails(self) -> None:
        for controller, flags in COMPLETE.items():
            for flag in flags:
                with self.subTest(controller=controller, flag=flag):
                    args = {**COMPLETE, controller: [f for f in flags if f != flag]}
                    result = self.run_verify(args, "kustomize-controller")
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"{controller} {flag}", result.stderr)
                    self.assertIn("components/flux-lockdown", result.stderr)

    def test_missing_controller_fails(self) -> None:
        args = {k: v for k, v in COMPLETE.items() if k != "helm-controller"}
        result = self.run_verify(args, "kustomize-controller")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("helm-controller --no-cross-namespace-refs=true", result.stderr)

    def test_bootstrap_kustomization_without_service_account_fails(self) -> None:
        for sync_sa in ("", "default"):
            with self.subTest(sync_sa=sync_sa):
                result = self.run_verify(COMPLETE, sync_sa)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("flux-system serviceAccountName", result.stderr)

    def test_lifecycle_scripts_install_and_verify_the_lockdown(self) -> None:
        up = (PLATFORM / "platform-up.sh").read_text()
        lifecycle = (PLATFORM / "platform-lifecycle.sh").read_text()
        for text in (up, lifecycle):
            self.assertIn('source "$platform_dir/flux-lockdown.sh"', text)
            self.assertIn('flux_lockdown_verify "$platform_dir"', text)
        self.assertIn("flux_lockdown_render", up)
        self.assertNotIn("flux install", up)
        self.assertEqual(lifecycle.count('--components="$flux_lockdown_components"'), 2)


if __name__ == "__main__":
    unittest.main()

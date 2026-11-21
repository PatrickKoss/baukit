from __future__ import annotations

import os
import re
import subprocess
import unittest
from pathlib import Path
from typing import Any

import yaml
from process_runner import run_process

PLATFORM = Path(__file__).resolve().parent.parent / "deploy/platform"
K3S = PLATFORM / "k3s"
BAREMETAL = K3S / "baremetal"
HELPER = PLATFORM / "local-git-http.sh"
CONFIG_MOUNT = "/etc/baukit/k3s/"
PLACEHOLDER = re.compile(r"\$\{([A-Z0-9_]+)\}")
HOST_PARAMETERS = {
    "K3S_INIT_SERVER",
    "K3S_NODE_NAME",
    "K3S_NODE_IP",
    "K3S_API_ADDRESS",
    "K3S_SYSTEM_RESERVED_MEMORY",
}


def local_helper(*functions: str) -> list[str]:
    script = f'source "{HELPER}"\n' + "\n".join(functions)
    result: subprocess.CompletedProcess[str] = run_process(
        ["bash", "-euo", "pipefail", "-c", script],
        env=dict(os.environ),
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.splitlines()


def load(name: str) -> dict[str, Any]:
    return yaml.safe_load((BAREMETAL / name).read_text())


class BaremetalK3sConfigTest(unittest.TestCase):
    files = sorted(path.name for path in BAREMETAL.glob("*.yaml"))

    def test_every_key_is_a_flag_of_the_pinned_k3s(self) -> None:
        (image,) = local_helper("platform_k3s_image")
        tag = image.split(":", 1)[1].split("@", 1)[0]
        known = set(
            re.findall(r"^\s+--([a-z0-9-]+)", (K3S / f"server-help-{tag}.txt").read_text(), re.M)
        )
        for name in self.files:
            for key in load(name):
                with self.subTest(file=name, key=key):
                    self.assertIn(key.removesuffix("+"), known)

    def test_shared_config_matches_the_local_server_flags(self) -> None:
        config = load("config.yaml")
        local = local_helper("platform_k3s_server_args")
        apiserver = [
            arg.split("=", 1)[1] for arg in local if arg.startswith("--kube-apiserver-arg=")
        ]
        self.assertEqual(config["kube-apiserver-arg"], apiserver)
        self.assertIn("--disable=traefik", local)
        self.assertEqual(config["disable"], ["traefik"])
        for flag in ("secrets-encryption", "protect-kernel-defaults"):
            with self.subTest(flag=flag):
                self.assertIn(f"--{flag}", local)
                self.assertIs(config[flag], True)

    def test_referenced_policy_files_ship_in_k3s(self) -> None:
        text = (BAREMETAL / "config.yaml").read_text()
        for path in re.findall(re.escape(CONFIG_MOUNT) + r"(\S+\.yaml)", text):
            with self.subTest(path=path):
                self.assertTrue((K3S / path).is_file())

    def test_only_the_first_server_initialises_etcd(self) -> None:
        self.assertIs(load("init.yaml")["cluster-init"], True)
        join = load("join.yaml")
        self.assertNotIn("cluster-init", join)
        self.assertTrue(join["server"].startswith("https://"))
        self.assertEqual(load("config.yaml")["token-file"], CONFIG_MOUNT + "token")

    def test_no_single_node_leader_election_tweaks(self) -> None:
        for name in self.files:
            with self.subTest(file=name):
                self.assertNotIn("leader-elect", (BAREMETAL / name).read_text())

    def test_placeholders_are_the_documented_host_parameters(self) -> None:
        found = {
            match
            for name in self.files
            for match in PLACEHOLDER.findall((BAREMETAL / name).read_text())
        }
        self.assertEqual(found, HOST_PARAMETERS)
        self.assertFalse(PLACEHOLDER.search((BAREMETAL / "config.yaml").read_text()))


if __name__ == "__main__":
    unittest.main()

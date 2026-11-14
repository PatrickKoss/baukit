from __future__ import annotations

import ipaddress
import os
import re
import tempfile
import unittest
from pathlib import Path

import helm_yaml

PLATFORM = Path(__file__).resolve().parent.parent
KUSTOMIZE = PLATFORM.parent.parent / ".tools/bin/kustomize"
COMPONENT = PLATFORM / "components/network-policy/node-cidr"
APISERVER_EGRESS = PLATFORM / "components/network-policy/apiserver-egress"
LOCAL_COMPONENTS = PLATFORM / "overlays/local/components"
PLACEHOLDER = "192.0.2.0/32"
NODE_CIDR_LABEL = "baukit.dev/node-cidr-peer"
OVERLAYS = [PLATFORM / "overlays/everything", *sorted(LOCAL_COMPONENTS.iterdir())]


def build(path: Path):
    return helm_yaml.process_runner.run_process(
        [str(KUSTOMIZE), "build", str(path)], capture_output=True, text=True
    )


def write_overlay(directory: Path, *, with_value: bool) -> None:
    resources = ["../base"] + (["value.yaml"] if with_value else [])
    (directory / "overlay").mkdir()
    (directory / "overlay/kustomization.yaml").write_text(
        "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\nresources:\n"
        + "".join(f"  - {resource}\n" for resource in resources)
        + f"components:\n  - {os.path.relpath(COMPONENT, directory / 'overlay')}\n"
    )
    (directory / "overlay/value.yaml").write_text(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: baukit-cluster-network\n"
        '  annotations:\n    config.kubernetes.io/local-config: "true"\n'
        "data:\n  nodeCIDR: 198.51.100.0/24\n"
    )
    (directory / "base").mkdir()
    (directory / "base/kustomization.yaml").write_text(
        "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\nnamespace: probe\n"
        f"components:\n  - {os.path.relpath(APISERVER_EGRESS, directory / 'base')}\n"
    )


class NodeCidrReplacementTest(unittest.TestCase):
    def test_bases_ship_the_fail_closed_placeholder(self) -> None:
        result = build(PLATFORM / "cert-manager")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(PLACEHOLDER, result.stdout)

    def test_every_overlay_replaces_the_placeholder(self) -> None:
        for overlay in OVERLAYS:
            with self.subTest(overlay=overlay.name):
                result = build(overlay)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertNotIn(PLACEHOLDER, result.stdout)
                self.assertNotIn("baukit-cluster-network", result.stdout)

    def test_overlay_value_reaches_the_policy(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            write_overlay(Path(directory), with_value=True)
            result = build(Path(directory) / "overlay")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("198.51.100.0/24", result.stdout)
        self.assertNotIn(PLACEHOLDER, result.stdout)
        self.assertIn(NODE_CIDR_LABEL, result.stdout)

    def test_overlay_without_a_value_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            write_overlay(Path(directory), with_value=False)
            result = build(Path(directory) / "overlay")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("baukit-cluster-network", result.stderr)

    def test_local_k3d_subnet_sits_inside_the_local_node_cidr(self) -> None:
        value = (PLATFORM / "overlays/local/cluster-network/cluster-network.yaml").read_text()
        node_cidr = re.search(r"nodeCIDR: (\S+)", value)
        lifecycle = (PLATFORM / "platform-up.sh").read_text()
        subnet = re.search(r"BAUKIT_PLATFORM_SUBNET:-([^}]+)}", lifecycle)
        self.assertIsNotNone(node_cidr)
        self.assertIsNotNone(subnet)
        assert node_cidr is not None and subnet is not None
        self.assertTrue(
            ipaddress.ip_network(subnet.group(1)).subnet_of(
                ipaddress.ip_network(node_cidr.group(1))
            )
        )


if __name__ == "__main__":
    unittest.main()

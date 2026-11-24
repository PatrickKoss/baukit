import importlib.util
import os
import shutil
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
HELM_YAML_PATH = HERE.parent / "lint" / "helm_yaml.py"
HELM_YAML_SPEC = importlib.util.spec_from_file_location("helm_yaml", HELM_YAML_PATH)
helm_yaml = importlib.util.module_from_spec(HELM_YAML_SPEC)
HELM_YAML_SPEC.loader.exec_module(helm_yaml)

COMPONENT = HERE.parent / "components" / "emulator-slots"
KUSTOMIZE = shutil.which("kustomize")


def render(directory: Path) -> list[dict]:
    output = helm_yaml.process_runner.run_process(
        [KUSTOMIZE, "build", "--load-restrictor", "LoadRestrictionsNone", str(directory)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    documents = helm_yaml.split_documents(output)
    return [doc for doc in helm_yaml.parse_documents(documents) if doc]


def daemon_set(documents: list[dict]) -> dict:
    return next(doc for doc in documents if doc["kind"] == "DaemonSet")


def node_term(plugin: dict) -> dict:
    affinity = plugin["spec"]["template"]["spec"]["affinity"]["nodeAffinity"]
    return affinity["requiredDuringSchedulingIgnoredDuringExecution"]["nodeSelectorTerms"][0][
        "matchExpressions"
    ][0]


def device_arg(plugin: dict) -> str:
    return plugin["spec"]["template"]["spec"]["containers"][0]["args"][1]


@unittest.skipUnless(KUSTOMIZE, "kustomize is not installed")
class EmulatorSlotsRenderTest(unittest.TestCase):
    def test_defaults_render_two_slots_on_labelled_nodes(self) -> None:
        documents = render(HERE)
        self.assertEqual(sorted(doc["kind"] for doc in documents), ["DaemonSet", "PolicyException"])
        plugin = daemon_set(documents)
        self.assertEqual(
            node_term(plugin),
            {"key": "baukit.dev/emulators", "operator": "In", "values": ["true"]},
        )
        self.assertEqual(
            device_arg(plugin),
            "--device={name: kvm, groups: [{count: 2 , paths: [{path: /dev/kvm}]}]}",
        )
        image = plugin["spec"]["template"]["spec"]["containers"][0]["image"]
        self.assertRegex(image, r"@sha256:[0-9a-f]{64}$")

    def test_an_overlay_sets_label_and_count(self) -> None:
        with tempfile.TemporaryDirectory() as scratch:
            overlay = Path(scratch)
            (overlay / "kustomization.yaml").write_text(
                "apiVersion: kustomize.config.k8s.io/v1beta1\n"
                "kind: Kustomization\n"
                "resources: [parameters.yaml]\n"
                f"components: [{os.path.relpath(COMPONENT, overlay)}]\n"
            )
            (overlay / "parameters.yaml").write_text(
                "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: emulator-slots\n"
                '  annotations:\n    config.kubernetes.io/local-config: "true"\n'
                'data:\n  nodeLabelKey: lab/emulators\n  nodeLabelValue: "yes"\n'
                '  devicesPerNode: "3"\n'
            )
            plugin = daemon_set(render(overlay))
        self.assertEqual(node_term(plugin)["key"], "lab/emulators")
        self.assertEqual(node_term(plugin)["values"], ["yes"])
        self.assertIn("{count: 3 ,", device_arg(plugin))


if __name__ == "__main__":
    unittest.main()

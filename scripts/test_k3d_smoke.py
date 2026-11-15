from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import tempfile
import unittest
from collections.abc import Iterator
from pathlib import Path
from types import ModuleType

ROOT = Path(__file__).resolve().parents[1]
SMOKE = ROOT / "deploy" / "smoke" / "k3d-smoke.sh"
PLATFORM_HELPERS = ROOT / "deploy" / "platform" / "local-git-http.sh"
ENFORCE = ROOT / "deploy" / "smoke" / "kyverno-enforce.sh"
KYVERNO_RELEASE = ROOT / "deploy" / "platform" / "kyverno" / "helm-release.yaml"
KYVERNO_SOURCE = ROOT / "deploy" / "platform" / "kyverno" / "oci-repository.yaml"
POLICY_DIR = ROOT / "deploy" / "platform" / "kyverno-policies" / "policies"
EXCEPTION_DIR = ROOT / "deploy" / "platform" / "kyverno-policies" / "exceptions"
ENFORCED_KINDS = {"ValidatingPolicy", "ImageValidatingPolicy"}


def load_module(name: str, path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


process_runner = load_module("process_runner", ROOT / "scripts" / "process_runner.py")
helm_yaml = load_module("helm_yaml", ROOT / "deploy" / "platform" / "lint" / "helm_yaml.py")

STUB_KUBECTL = """#!/bin/sh
printf 'kubectl %s\\n' "$*" >>"$STUB_LOG"
case "$*" in
    kustomize*) exec "$REAL_KUBECTL" "$@" ;;
    "create namespace "*) printf 'kind: Namespace\\nmetadata:\\n  name: %s\\n' "$3" ;;
    "label --local"*) cat ;;
    *"jsonpath={.metadata.labels.pod-security"*) printf restricted ;;
    *"jsonpath={.metadata.labels.grafana_dashboard}"*) printf 1 ;;
    "create -f -"|"apply -f -") cat >>"$STUB_LOG" ;;
    "apply -f "*) cat "$3" >>"$STUB_LOG" ;;
esac
exit 0
"""
STUB_K3D = """#!/bin/sh
printf 'k3d %s\\n' "$*" >>"$STUB_LOG"
exit 0
"""
STUB_HELM = """#!/bin/sh
printf 'helm %s\\n' "$*" >>"$STUB_LOG"
case "$1" in
    template) exec "$REAL_HELM" "$@" ;;
    pull)
        while [ $# -gt 0 ]; do
            [ "$1" = --destination ] && touch "$2/kyverno-stub.tgz"
            shift
        done
        printf 'Digest: %s\\n' "$STUB_CHART_DIGEST"
        ;;
esac
exit 0
"""
STUB_CURL = """#!/bin/sh
case "$*" in
    *--write-out*) printf 200 ;;
esac
exit 0
"""
STUB_OK = "#!/bin/sh\nexit 0\n"
STUB_DOCKER = "#!/bin/sh\nprintf '%s\\n' \"${STUB_CGROUP_VERSION:-2}\"\n"
TOKEN_COMMAND = "#!/bin/sh\nprintf 'stub-token\\n' >\"$BAUKIT_SMOKE_ACCESS_TOKEN_FILE\"\n"


def write_executable(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(0o755)


def image_tags(value: object, path: str = "") -> Iterator[tuple[str, str]]:
    if isinstance(value, dict):
        image = value.get("image")
        if isinstance(image, dict) and isinstance(image.get("tag"), str):
            yield f"{path}.image", image["tag"]
        for key, child in value.items():
            yield from image_tags(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from image_tags(child, f"{path}[{index}]")


def render(out: Path, library: Path | None = None) -> object:
    env = dict(os.environ)
    if library is not None:
        env["BAUKIT_SMOKE_KYVERNO_POLICY_LIBRARY"] = str(library)
    return process_runner.run_process(
        [str(ENFORCE), "render", str(out)], capture_output=True, text=True, env=env
    )


def parse(path: Path) -> list[dict]:
    documents = helm_yaml.split_documents(path.read_text())
    return [doc for doc in helm_yaml.parse_documents(documents) if doc]


class RenderTest(unittest.TestCase):
    def test_every_library_policy_is_rendered_with_deny(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            result = render(out)
            self.assertEqual(result.returncode, 0, result.stderr)
            objects = parse(out / "policies.yaml")
            names = {doc["metadata"]["name"] for doc in objects if doc["kind"] != "PolicyException"}
            library = {path.stem for path in POLICY_DIR.glob("*.yaml")}
            self.assertEqual(names, library)
            exceptions = {
                doc["metadata"]["name"] for doc in objects if doc["kind"] == "PolicyException"
            }
            exception_files = {
                p.stem for p in EXCEPTION_DIR.glob("*.yaml") if p.name != "kustomization.yaml"
            }
            self.assertEqual(exceptions, exception_files)
            self.assertFalse(
                [d for d in objects if d["kind"].startswith("Helm") or d["kind"] == "OCIRepository"]
            )
            enforced = [doc for doc in objects if doc["kind"] in ENFORCED_KINDS]
            self.assertTrue(enforced)
            for doc in enforced:
                name = doc["metadata"]["name"]
                self.assertEqual(doc["spec"]["validationActions"], ["Deny"], name)

    def test_chart_pin_and_values_come_from_the_platform_release(self) -> None:
        release = parse(KYVERNO_RELEASE)[0]
        source = parse(KYVERNO_SOURCE)[0]
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            self.assertEqual(render(out).returncode, 0)
            chart = parse(out / "chart.yaml")[0]
            values = parse(out / "values.yaml")[0]
        self.assertEqual(release["spec"]["chartRef"]["name"], source["metadata"]["name"])
        self.assertEqual(chart["version"], source["spec"]["ref"]["tag"])
        self.assertEqual(chart["digest"], source["spec"]["ref"]["digest"])
        self.assertEqual(chart["chart"], "oci://ghcr.io/kyverno/charts/kyverno")
        self.assertEqual(values, release["spec"]["values"])

    def test_a_release_without_the_oci_source_fails_the_render(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            kyverno = Path(tmp) / "kyverno"
            shutil.copytree(KYVERNO_SOURCE.parent, kyverno)
            source = kyverno / "oci-repository.yaml"
            source.write_text(source.read_text().replace("    digest: ", "    pinned: "))
            env = dict(os.environ, BAUKIT_SMOKE_KYVERNO_DIR=str(kyverno))
            result = process_runner.run_process(
                [str(ENFORCE), "render", str(Path(tmp) / "out")],
                capture_output=True,
                text=True,
                env=env,
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not pin ref.digest", result.stderr)

    def test_smoke_install_pins_every_kyverno_image_by_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            self.assertEqual(render(out).returncode, 0)
            values = parse(out / "values.yaml")[0]
        tags = dict(image_tags(values))
        self.assertGreaterEqual(len(tags), 9)
        for path, tag in tags.items():
            self.assertRegex(tag, r"@sha256:[0-9a-f]{64}$", path)

    def test_exceptions_are_kept_and_unknown_policy_kinds_fail(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            library = Path(tmp) / "library"
            library.mkdir()
            (library / "policy.yaml").write_text(
                "apiVersion: policies.kyverno.io/v1\nkind: ValidatingPolicy\n"
                "metadata:\n  name: audited\nspec:\n  validationActions: [Audit]\n"
            )
            (library / "exception.yaml").write_text(
                "apiVersion: policies.kyverno.io/v1\nkind: PolicyException\n"
                "metadata:\n  name: allowed\n  namespace: kyverno\nspec: {}\n"
            )
            kustomization = library / "kustomization.yaml"
            kustomization.write_text("resources: [policy.yaml, exception.yaml]\n")
            out = Path(tmp) / "out"
            self.assertEqual(render(out, library).returncode, 0)
            kinds = {doc["kind"]: doc for doc in parse(out / "policies.yaml")}
            self.assertEqual(kinds["ValidatingPolicy"]["spec"]["validationActions"], ["Deny"])
            self.assertIn("PolicyException", kinds)

            (library / "legacy.yaml").write_text(
                "apiVersion: kyverno.io/v1\nkind: ClusterPolicy\nmetadata:\n  name: legacy\n"
                "spec:\n  validationFailureAction: Audit\n"
            )
            kustomization.write_text("resources: [policy.yaml, legacy.yaml]\n")
            result = render(Path(tmp) / "failed", library)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("cannot enforce ClusterPolicy legacy", result.stderr)


class SmokeOrderTest(unittest.TestCase):
    def smoke_process(
        self,
        tmp: Path,
        digest: str | None = None,
        extra_env: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        bin_dir = tmp / "bin"
        write_executable(bin_dir / "kubectl", STUB_KUBECTL)
        write_executable(bin_dir / "helm", STUB_HELM)
        write_executable(bin_dir / "curl", STUB_CURL)
        write_executable(bin_dir / "k3d", STUB_K3D)
        write_executable(bin_dir / "docker", STUB_DOCKER)
        observability = tmp / "observability"
        observability.mkdir()
        (observability / "Chart.yaml").write_text("name: stub\n")
        write_executable(observability / "verify" / "verify-observability.sh", STUB_OK)
        write_executable(tmp / "token.sh", TOKEN_COMMAND)
        values = tmp / "values.yaml"
        values.write_text("{}\n")
        log = tmp / "calls.log"
        env = dict(os.environ)
        env.update(
            PATH=f"{bin_dir}{os.pathsep}{env['PATH']}",
            REAL_KUBECTL=shutil.which("kubectl") or "kubectl",
            REAL_HELM=shutil.which("helm") or "helm",
            STUB_LOG=str(log),
            BAUKIT_SMOKE_PRODUCT="demo",
            BAUKIT_SMOKE_VALUES_FILE=str(values),
            BAUKIT_SMOKE_IMAGES="demo-api:1 postgres:18",
            BAUKIT_SMOKE_TOKEN_COMMAND=str(tmp / "token.sh"),
            BAUKIT_SMOKE_OBSERVABILITY_CHART=str(observability),
            BAUKIT_SMOKE_K3D_API_PORT="127.0.0.1:6443",
            STUB_CHART_DIGEST=digest or parse(KYVERNO_SOURCE)[0]["spec"]["ref"]["digest"],
        )
        env.update(extra_env or {})
        return process_runner.run_process(
            ["sh", str(SMOKE)], capture_output=True, text=True, env=env
        )

    def run_smoke(
        self,
        tmp: Path,
        digest: str | None = None,
        extra_env: dict[str, str] | None = None,
    ) -> list[str]:
        result = self.smoke_process(tmp, digest, extra_env)
        log = tmp / "calls.log"
        if digest is not None:
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("is not the pinned sha256:", result.stderr)
            return []
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return log.read_text().splitlines()

    def index(self, lines: list[str], prefix: str, suffix: str = "") -> int:
        for position, line in enumerate(lines):
            if line.startswith(prefix) and line.endswith(suffix):
                return position
        self.fail(f"no call starting with {prefix!r}")

    def test_kyverno_and_enforced_policies_come_before_the_product_namespace(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            lines = self.run_smoke(Path(tmp))
        source = parse(KYVERNO_SOURCE)[0]["spec"]
        pull = self.index(lines, f"helm pull {source['url']} --version {source['ref']['tag']} ")
        kyverno = self.index(lines, "helm upgrade --install kyverno ")
        priority = self.index(lines, "kubectl apply -f ", "/cluster-baseline/priority-classes.yaml")
        allowlist = self.index(lines, "kubectl -n kyverno create configmap baukit-registry-")
        policies = self.index(lines, "kubectl apply -f ", "/policies.yaml") + 1
        namespace = self.index(lines, "kubectl label --local -f - -o yaml baukit.dev/tenant=true")
        product = self.index(lines, "helm upgrade --install demo ")
        self.assertLess(pull, kyverno)
        self.assertLess(priority, kyverno)
        self.assertIn("/kyverno-stub.tgz ", lines[kyverno])
        self.assertLess(kyverno, allowlist)
        self.assertLess(allowlist, policies)
        self.assertLess(policies, namespace)
        self.assertLess(namespace, product)
        self.assertIn("registries=demo-api:1\npostgres:18 ", "\n".join(lines))
        applied = "\n".join(lines[policies:namespace])
        self.assertIn("kind: ValidatingPolicy", applied)
        self.assertNotIn("- Audit", applied)
        self.assertIn("kubectl wait --for=jsonpath=", applied)

    def cluster_create(self, lines: list[str]) -> list[str]:
        return lines[self.index(lines, "k3d cluster create ")].split()

    def test_cluster_uses_the_platform_k3s_image_pinned_by_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            args = self.cluster_create(self.run_smoke(Path(tmp)))
        image = args[args.index("--image") + 1]
        self.assertRegex(image, r"^rancher/k3s:v[0-9.]+-k3s[0-9]+@sha256:[0-9a-f]{64}$")
        self.assertIn(f"printf '%s\\n' {image}\n", PLATFORM_HELPERS.read_text())

    def test_cluster_image_can_be_overridden(self) -> None:
        override = "rancher/k3s:v1.33.0-k3s1@sha256:" + "a" * 64
        with tempfile.TemporaryDirectory() as tmp:
            lines = self.run_smoke(Path(tmp), extra_env={"BAUKIT_SMOKE_K3S_IMAGE": override})
        args = self.cluster_create(lines)
        self.assertEqual(args[args.index("--image") + 1], override)

    def test_cgroup_v1_host_fails_before_k3d(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            result = self.smoke_process(Path(tmp), extra_env={"STUB_CGROUP_VERSION": "1"})
            log = Path(tmp) / "calls.log"
            calls = log.read_text() if log.exists() else ""
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cgroup_no_v1=all", result.stderr)
        self.assertNotIn("k3d cluster create", calls)

    def test_a_chart_with_another_digest_is_not_installed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.run_smoke(Path(tmp), digest="sha256:" + "0" * 64)
            calls = (Path(tmp) / "calls.log").read_text()
        self.assertNotIn("helm upgrade --install kyverno", calls)


if __name__ == "__main__":
    unittest.main()

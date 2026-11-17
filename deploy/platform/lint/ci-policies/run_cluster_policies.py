"""Run the in-cluster Kyverno policies offline.

Runs kyverno test over the in-cluster policy suites, then kyverno apply with every policy the
cluster gets plus the Flux convention policies against each rendered base, overlay, chart and
baukit-app profile. A fail fails the gate when its policy denies admission (Deny or Enforce) or
runs only offline. A fail from an Audit-only policy goes into the table, as the cluster would
only report it. Every error fails.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from collections import Counter, defaultdict
from collections.abc import Iterable
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path

import chart_contents
import run_ci_policies

helm_yaml = chart_contents.helm_yaml
HERE = run_ci_policies.HERE
PLATFORM = HERE.parents[1]
REPO_ROOT = HERE.parents[3]
APP_CHART = REPO_ROOT / "deploy/chart/baukit-app"
APP_PROFILES = {
    "all-options": APP_CHART / "tests/all-options-values.yaml",
    "generated-platform": PLATFORM
    / "kyverno-policies/tests/app-profiles/generated-platform-values.yaml",
}
APP_NAMESPACE = "product"
APP_EXTRA_LABELS = {"baukit.dev/environment": "production"}
# A product release pins every image by digest; the profiles carry placeholder tags.
RELEASE_DIGEST = "@sha256:" + "0" * 64
POD_SPEC_PATHS = (
    ("spec",),
    ("spec", "template", "spec"),
    ("spec", "jobTemplate", "spec", "template", "spec"),
)
CONTAINER_KEYS = ("containers", "initContainers")
# Signature checks fetch from the registry, which a static render cannot do. Their kyverno tests
# cover the offline rules.
REGISTRY_POLICIES = {"verify-baukit-images"}
TENANT_COMPONENT = PLATFORM / "components/tenant/kustomization.yaml"
ALLOWLIST = ("kyverno", "baukit-registry-allowlist")
POLICY_KINDS = {
    "ValidatingPolicy",
    "ImageValidatingPolicy",
    "GeneratingPolicy",
    "MutatingPolicy",
    "DeletingPolicy",
}
CLUSTER_SCOPED = {
    "APIService",
    "CSIDriver",
    "ClusterRole",
    "ClusterRoleBinding",
    "CustomResourceDefinition",
    "IngressClass",
    "MutatingWebhookConfiguration",
    "Namespace",
    "PersistentVolume",
    "PriorityClass",
    "RuntimeClass",
    "StorageClass",
    "ValidatingAdmissionPolicy",
    "ValidatingAdmissionPolicyBinding",
    "ValidatingWebhookConfiguration",
}
CRD_KIND = re.compile(r"^kind: CustomResourceDefinition$", re.MULTILINE)
CRD_NAME = re.compile(r"^  name: (\S+)$", re.MULTILINE)
CRD_CLUSTER_KIND = re.compile(
    r"^    kind: (\S+)$(?=.*^  scope: Cluster$)", re.MULTILINE | re.DOTALL
)
COUNTED = run_ci_policies.COUNTED
ENFORCING_ACTIONS = {"Deny", "Enforce"}
APPLY_WORKERS = 4
# The identity that admits each universe: platform Flux applies bases and overlays, helm-controller
# installs charts, and a tenant's own flux-reconciler applies baukit-app.
APPLIERS = {
    "base-": "system:serviceaccount:flux-system:kustomize-controller",
    "overlay-": "system:serviceaccount:flux-system:kustomize-controller",
    "chart-": "system:serviceaccount:flux-system:helm-controller",
    "app-": f"system:serviceaccount:{APP_NAMESPACE}:flux-reconciler",
}

Runner = run_ci_policies.Runner


class StageError(Exception):
    pass


@dataclass
class Universe:
    name: str
    objects: list[dict]


@dataclass
class Inputs:
    universes: list[Universe]
    policies: list[dict]
    exceptions: list[dict]
    crds: list[str]
    namespaces: dict[str, dict]
    context: list[dict]


@dataclass
class Tally:
    modes: dict[str, str]
    counts: dict[str, Counter] = field(default_factory=lambda: defaultdict(Counter))
    enforced: list[str] = field(default_factory=list)
    audited: list[str] = field(default_factory=list)

    def add(self, universe: str, report: dict) -> None:
        for result in report.get("results") or []:
            policy = result.get("policy", "?")
            outcome = result.get("result", "error")
            self.counts[policy][outcome] += 1
            if outcome not in ("fail", "error"):
                continue
            target = self.audited if self.audit_only(policy, outcome) else self.enforced
            for resource in result.get("resources") or [{}]:
                ref = (
                    f"{resource.get('kind')}/{resource.get('namespace', '')}/{resource.get('name')}"
                )
                target.append(f"{universe}: {policy}: {ref}: {result.get('message', '')}")

    def audit_only(self, policy: str, outcome: str) -> bool:
        return outcome == "fail" and self.modes.get(policy) == "Audit"

    @property
    def results(self) -> int:
        return sum(sum(counts.values()) for counts in self.counts.values())

    def table(self) -> str:
        header = f"{'POLICY':<36} {'MODE':<8} {'PASS':>5} {'FAIL':>5} {'ERROR':>5} {'SKIP':>5}"
        rows = [header]
        for policy in sorted(set(self.modes) | set(self.counts)):
            counts = self.counts.get(policy, Counter())
            mode = self.modes.get(policy, "?")
            rows.append(
                f"{policy:<36} {mode:<8} " + " ".join(f"{counts[key]:>5}" for key in COUNTED)
            )
        return "\n".join(rows)


def policy_mode(policy: dict) -> str:
    """Deny when admission rejects a violation, Audit when it only reports, else the kind."""
    spec = policy.get("spec") or {}
    actions = set(spec.get("validationActions") or [])
    action = spec.get("validationFailureAction")
    if action:
        actions.add(action)
    if actions & ENFORCING_ACTIONS:
        return "Deny"
    if actions:
        return "Audit"
    return policy.get("kind", "?").removesuffix("Policy") or "?"


def name_of(obj: dict) -> str:
    return str((obj.get("metadata") or {}).get("name", ""))


def namespace_of(obj: dict) -> str:
    return str((obj.get("metadata") or {}).get("namespace", ""))


def with_namespace(objects: Iterable[dict], namespace: str, cluster_kinds: set[str]) -> list[dict]:
    return [
        obj
        if obj.get("kind") in cluster_kinds or namespace_of(obj)
        else {**obj, "metadata": {**(obj.get("metadata") or {}), "namespace": namespace}}
        for obj in objects
    ]


def crd_documents(texts: Iterable[str]) -> tuple[list[str], set[str]]:
    by_name: dict[str, str] = {}
    cluster_kinds: set[str] = set()
    for text in texts:
        for document in helm_yaml.split_documents(text):
            if not CRD_KIND.search(document):
                continue
            name = CRD_NAME.search(document)
            if name:
                by_name.setdefault(name.group(1), document.strip("\n") + "\n")
            cluster_kinds.update(CRD_CLUSTER_KIND.findall(document))
    return list(by_name.values()), cluster_kinds


def without_nulls(value: object) -> object:
    """Drop null fields the way the API server does, so CEL sees absent fields, not nulls."""
    if isinstance(value, dict):
        return {key: without_nulls(item) for key, item in value.items() if item is not None}
    if isinstance(value, list):
        return [without_nulls(item) for item in value if item is not None]
    return value


def parse_text(text: str) -> list[dict]:
    documents = [
        document
        for document in helm_yaml.split_documents(text)
        if document.strip() and not CRD_KIND.search(document)
    ]
    parsed = (without_nulls(obj) for obj in helm_yaml.parse_documents(documents))
    return [obj for obj in parsed if isinstance(obj, dict) and obj.get("kind")]


def tenant_labels(component: Path) -> dict[str, str]:
    labels: dict[str, str] = {}
    for kustomization in parse_text(component.read_text()):
        for patch in kustomization.get("patches") or []:
            for obj in parse_text(patch.get("patch", "")):
                if obj.get("kind") == "Namespace":
                    labels.update((obj.get("metadata") or {}).get("labels") or {})
    if labels.get("baukit.dev/tenant") != "true":
        raise StageError(f"{component} does not label the tenant namespace")
    return labels


def render_app(profile: str, values: Path, run: Runner) -> str:
    code, output = run(
        [
            "helm",
            "template",
            profile,
            str(APP_CHART),
            "--namespace",
            APP_NAMESPACE,
            "--values",
            str(values),
        ]
    )
    if code != 0:
        raise StageError(f"helm template baukit-app with {values} exited {code}:\n{output.strip()}")
    return output


def pin_release_digests(objects: list[dict]) -> list[dict]:
    for obj in objects:
        for path in POD_SPEC_PATHS:
            spec = obj
            for key in path:
                spec = spec.get(key) if isinstance(spec, dict) else None
            if not isinstance(spec, dict):
                continue
            for key in CONTAINER_KEYS:
                for container in spec.get(key) or []:
                    image = container.get("image", "")
                    if image and "@sha256:" not in image:
                        container["image"] = image + RELEASE_DIGEST
    return objects


def release_namespaces(rendered: Path) -> dict[str, str]:
    namespaces = {}
    for release, chart, _ in chart_contents.releases_by_render(rendered):
        spec = release.get("spec") or {}
        namespaces[chart.name] = spec.get("targetNamespace") or namespace_of(release) or "default"
    return namespaces


def collect_inputs(
    rendered: Path, app_profiles: dict[str, Path], extra_crds: list[Path], run: Runner
) -> Inputs:
    files = run_ci_policies.universes(rendered) + sorted(rendered.glob("chart-*.yaml"))
    texts = {path.stem: path.read_text() for path in files}
    for profile, values in app_profiles.items():
        texts[f"app-{profile}"] = render_app(profile, values, run)
    crds, cluster_kinds = crd_documents(
        [*texts.values(), *(path.read_text() for path in extra_crds)]
    )
    cluster_kinds |= CLUSTER_SCOPED
    release_ns = release_namespaces(rendered)

    universes = []
    for name, text in texts.items():
        objects = parse_text(text)
        if name.startswith("chart-"):
            objects = with_namespace(
                objects, release_ns.get(f"{name}.yaml", "default"), cluster_kinds
            )
        elif name.startswith("app-"):
            objects = pin_release_digests(with_namespace(objects, APP_NAMESPACE, cluster_kinds))
        universes.append(Universe(name, objects))

    cluster = [
        obj
        for universe in universes
        if not universe.name.startswith("app-")
        for obj in universe.objects
    ]
    policies = unique(
        obj
        for obj in cluster
        if obj.get("kind") in POLICY_KINDS and name_of(obj) not in REGISTRY_POLICIES
    )
    exceptions = unique(obj for obj in cluster if obj.get("kind") == "PolicyException")
    namespaces: dict[str, dict] = {}
    for obj in cluster:
        if obj.get("kind") == "Namespace":
            namespaces.setdefault(name_of(obj), {}).update(
                (obj.get("metadata") or {}).get("labels") or {}
            )
    namespaces[APP_NAMESPACE] = {**tenant_labels(TENANT_COMPONENT), **APP_EXTRA_LABELS}
    allowlist = unique(
        obj
        for obj in cluster
        if obj.get("kind") == "ConfigMap" and (namespace_of(obj), name_of(obj)) == ALLOWLIST
    )
    ingresses = unique(
        obj for universe in universes for obj in universe.objects if obj.get("kind") == "Ingress"
    )
    context = allowlist + ingresses
    if not policies:
        raise StageError(f"{rendered}: no rendered in-cluster Kyverno policy")
    if not allowlist:
        raise StageError(f"{rendered}: no ConfigMap {'/'.join(ALLOWLIST)} in any overlay")
    return Inputs(universes, policies, exceptions, crds, namespaces, context)


def unique(objects: Iterable[dict]) -> list[dict]:
    seen: dict[tuple[str, str, str], dict] = {}
    for obj in objects:
        seen.setdefault((obj.get("kind", ""), namespace_of(obj), name_of(obj)), obj)
    return list(seen.values())


def json_documents(objects: Iterable[dict]) -> str:
    return "".join(f"---\n{json.dumps(obj, sort_keys=True)}\n" for obj in objects)


def write_inputs(inputs: Inputs, work: Path) -> dict[str, Path]:
    paths = {
        "policies": work / "cluster-policies.yaml",
        "crds": work / "crds.yaml",
        "values": work / "values.yaml",
        "context": work / "context.yaml",
    }
    paths["policies"].write_text(json_documents(inputs.policies))
    paths["crds"].write_text("".join(f"---\n{crd}" for crd in inputs.crds))
    values = {
        "apiVersion": "cli.kyverno.io/v1alpha1",
        "kind": "Values",
        "metadata": {"name": "namespaces"},
        "namespaceSelector": [
            {"name": name, "labels": labels} for name, labels in sorted(inputs.namespaces.items())
        ],
    }
    paths["values"].write_text(json.dumps(values, sort_keys=True))
    context = {
        "apiVersion": "cli.kyverno.io/v1alpha1",
        "kind": "Context",
        "metadata": {"name": "cluster"},
        "spec": {"resources": inputs.context},
    }
    paths["context"].write_text(json.dumps(context, sort_keys=True))
    if inputs.exceptions:
        paths["exceptions"] = work / "exceptions.yaml"
        paths["exceptions"].write_text(json_documents(inputs.exceptions))
    return paths


def applier(universe: str) -> str | None:
    return next((user for prefix, user in APPLIERS.items() if universe.startswith(prefix)), None)


def write_userinfo(username: str, work: Path) -> Path:
    namespace = username.split(":")[2]
    userinfo = {
        "apiVersion": "cli.kyverno.io/v1alpha1",
        "kind": "UserInfo",
        "metadata": {"name": "applier"},
        "userInfo": {
            "username": username,
            "groups": [
                "system:serviceaccounts",
                f"system:serviceaccounts:{namespace}",
                "system:authenticated",
            ],
        },
    }
    path = work / f"userinfo-{username.replace(':', '-')}.yaml"
    path.write_text(json.dumps(userinfo, sort_keys=True))
    return path


def apply_command(
    paths: dict[str, Path], ci_policies: Path, resources: Path, userinfo: Path | None = None
) -> list[str]:
    command = [
        "kyverno",
        "apply",
        str(paths["policies"]),
        str(ci_policies),
        "--resource",
        str(resources),
        "--crd-paths",
        str(paths["crds"]),
        f"--values-file={paths['values']}",
        f"--context-file={paths['context']}",
        "--policy-report",
        "--output-format",
        "json",
    ]
    if "exceptions" in paths:
        command += ["--exception", str(paths["exceptions"])]
    if userinfo is not None:
        command += ["--userinfo", str(userinfo)]
    return command


def parse_report(output: str) -> dict | None:
    """Return the policy report, skipping the YAML that GeneratingPolicies print before it."""
    decoder = json.JSONDecoder()
    start = output.find("{")
    while start >= 0:
        try:
            report = decoder.raw_decode(output[start:])[0]
        except json.JSONDecodeError:
            report = None
        if isinstance(report, dict) and "summary" in report:
            return report
        start = output.find("{", start + 1)
    return None


def policy_modes(inputs: Inputs, ci_policies: Path) -> dict[str, str]:
    ci_objects = parse_text(
        "\n---\n".join(path.read_text() for path in sorted(ci_policies.glob("*.yaml")))
    )
    modes = {name_of(policy): policy_mode(policy) for policy in inputs.policies}
    for policy in ci_objects:
        if policy.get("kind") in POLICY_KINDS:
            modes[name_of(policy)] = "CI"
    return modes


def apply_policies(
    inputs: Inputs, ci_policies: Path, run: Runner, work: Path
) -> tuple[Tally, list[str]]:
    paths = write_inputs(inputs, work)
    tally = Tally(policy_modes(inputs, ci_policies))
    problems = []
    commands = {}
    for universe in inputs.universes:
        if universe.objects:
            resources = work / f"{universe.name}.yaml"
            resources.write_text(json_documents(universe.objects))
            username = applier(universe.name)
            userinfo = write_userinfo(username, work) if username else None
            commands[universe.name] = apply_command(paths, ci_policies, resources, userinfo)
    with ThreadPoolExecutor(max_workers=APPLY_WORKERS) as pool:
        outputs = dict(zip(commands, pool.map(run, commands.values()), strict=True))
    for name, (code, output) in outputs.items():
        report = parse_report(output)
        if report is None and code == 0 and not output.strip():
            continue
        if report is None:
            problems.append(
                f"{name}: kyverno apply exited {code} without a report:\n{output.strip()}"
            )
            continue
        tally.add(name, report)
        summary = report.get("summary") or {}
        if code != 0 and not (summary.get("fail") or summary.get("error")):
            problems.append(f"{name}: kyverno apply exited {code}:\n{output.strip()}")
    if tally.results == 0 and not problems:
        problems.append("no rendered object produced a cluster policy result")
    return tally, problems


def run_tests(tests: Path, run: Runner) -> bool:
    code, output = run(["kyverno", "test", str(tests)])
    summary = re.search(r"^Test Summary: .*$", output, re.MULTILINE)
    print(summary.group(0) if summary else output.strip())
    if code != 0:
        print(output.strip(), file=sys.stderr)
    return code == 0


def write_report(path: Path, tally: Tally) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [tally.table()]
    lines += [f"FAIL {line}" for line in tally.enforced]
    lines += [f"AUDIT {line}" for line in tally.audited]
    path.write_text("\n".join(lines) + "\n")


def main(argv: list[str] | None = None, run: Runner = run_ci_policies.run_command) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, required=True)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--crds", type=Path, action="append", default=[])
    parser.add_argument("--ci-policies", type=Path, default=HERE / "policies")
    parser.add_argument("--tests", type=Path, default=PLATFORM / "kyverno-policies/tests")
    args = parser.parse_args(argv)

    status = 0 if run_tests(args.tests, run) else 1
    with tempfile.TemporaryDirectory() as work:
        try:
            inputs = collect_inputs(args.rendered, APP_PROFILES, args.crds, run)
        except (StageError, helm_yaml.YamlError) as error:
            print(f"ERROR: {error}", file=sys.stderr)
            return 1
        tally, problems = apply_policies(inputs, args.ci_policies, run, Path(work))
    print(tally.table())
    if args.report:
        write_report(args.report, tally)
    for line in tally.audited:
        print(f"AUDIT: {line}")
    for line in tally.enforced + problems:
        print(f"ERROR: {line}", file=sys.stderr)
    return 1 if status or tally.enforced or problems else 0


if __name__ == "__main__":
    raise SystemExit(main())

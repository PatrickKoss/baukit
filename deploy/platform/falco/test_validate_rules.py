import importlib.util
import json
import os
import re
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

FALCO_DIR = Path(__file__).resolve().parent
REPO_ROOT = FALCO_DIR.parents[2]
SCRIPT = FALCO_DIR / "validate-rules.sh"
PLATFORM_DIR = FALCO_DIR.parent
BAUKIT_RULES = FALCO_DIR / "rules/baukit.rules.yaml"
K8SAUDIT_RULES = FALCO_DIR / "k8saudit-rules/baukit.k8saudit.rules.yaml"
K8SAUDIT_RELEASE = FALCO_DIR / "k8saudit-helm-release.yaml"


def load_process_runner():
    spec = importlib.util.spec_from_file_location(
        "process_runner", REPO_ROOT / "scripts/process_runner.py"
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PROCESS_RUNNER = load_process_runner()

BROKEN_RULES = {
    "undefined macro": """
- rule: Broken baukit rule
  desc: Uses a macro nobody defined.
  condition: spawned_process and no_such_baukit_macro
  output: broken | process=%proc.name
  priority: WARNING
""",
    "misspelled output field": """
- rule: Broken baukit rule
  desc: Prints a field that does not exist.
  condition: spawned_process
  output: broken | pod=%k8s.pod.nmae
  priority: WARNING
""",
}


def validate(*rules_files: Path) -> subprocess.CompletedProcess[str]:
    return PROCESS_RUNNER.run_process(
        [str(SCRIPT), *map(str, rules_files)], capture_output=True, text=True
    )


class ValidateRulesTest(unittest.TestCase):
    def test_shipped_rules_validate(self) -> None:
        result = validate()

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("baukit.rules.yaml: Ok", result.stdout)
        self.assertIn("baukit.k8saudit.rules.yaml: Ok", result.stdout)

    def test_broken_rules_fail(self) -> None:
        for name, content in BROKEN_RULES.items():
            with self.subTest(name), tempfile.TemporaryDirectory() as directory:
                rules_file = Path(directory) / "broken.rules.yaml"
                rules_file.write_text(content, encoding="utf-8")

                result = validate(rules_file)

                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_broken_k8saudit_rules_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            rules_file = Path(directory) / "broken.rules.yaml"
            rules_file.write_text(
                "- rule: Broken audit rule\n  desc: Misspells a k8saudit field.\n"
                "  condition: ka.verbb=get\n  output: broken\n  priority: WARNING\n"
                "  source: k8s_audit\n",
                encoding="utf-8",
            )

            result = validate(Path("--k8saudit"), rules_file)

            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_k8saudit_rules_need_the_plugin(self) -> None:
        result = validate(K8SAUDIT_RULES)

        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)


RESTORE_TEST = "system:serviceaccount:postgres:postgres-restore-test"
CNPG_OPERATOR = "system:serviceaccount:cnpg-system:cloudnative-pg"


FLAGGER = "system:serviceaccount:flagger-system:flagger"


def secret_event(index: int, user: str, namespace: str, verb: str) -> str:
    event = json.loads(audit_event(index, user, namespace, "", verb))
    event["requestURI"] = f"/api/v1/namespaces/{namespace}/secrets/s"
    event["objectRef"] = {
        "resource": "secrets",
        "namespace": namespace,
        "name": "s",
        "apiVersion": "v1",
    }
    event["responseStatus"]["code"] = 200
    return json.dumps(event)


def audit_event(index: int, user: str, namespace: str, subresource: str, verb: str) -> str:
    timestamp = f"2026-11-16T21:00:{index:02d}Z"
    return json.dumps(
        {
            "kind": "Event",
            "apiVersion": "audit.k8s.io/v1",
            "level": "Metadata",
            "auditID": f"00000000-0000-0000-0000-{index:012d}",
            "stage": "ResponseComplete",
            "requestURI": f"/api/v1/namespaces/{namespace}/pods/p/{subresource}?command=true",
            "verb": verb,
            "user": {"username": user, "groups": ["system:authenticated"]},
            "sourceIPs": ["10.42.0.9"],
            "objectRef": {
                "resource": "pods",
                "namespace": namespace,
                "name": "p",
                "apiVersion": "v1",
                "subresource": subresource,
            },
            "responseStatus": {"metadata": {}, "code": 101},
            "requestReceivedTimestamp": timestamp,
            "stageTimestamp": timestamp,
            "annotations": {"authorization.k8s.io/decision": "allow"},
        }
    )


class K8sAuditReplayTest(unittest.TestCase):
    def test_only_operator_execs_into_postgres_are_expected(self) -> None:
        events = [
            (CNPG_OPERATOR, "postgres", "exec", "create"),
            (RESTORE_TEST, "postgres", "exec", "create"),
            ("system:admin", "postgres", "exec", "get"),
            (RESTORE_TEST, "postgres", "attach", "create"),
            (RESTORE_TEST, "default", "exec", "create"),
            ("system:serviceaccount:postgres:other", "postgres", "exec", "create"),
            (CNPG_OPERATOR, "cnpg-system", "exec", "create"),
            (CNPG_OPERATOR, "postgres", "attach", "create"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            events_file = Path(directory) / "audit.jsonl"
            events_file.write_text(
                "".join(audit_event(i, *event) + "\n" for i, event in enumerate(events, 1)),
                encoding="utf-8",
            )

            result = validate(Path("--k8saudit-replay"), events_file)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        fired = [tuple(line.split("\t")[1:]) for line in result.stdout.splitlines()]
        self.assertEqual(
            fired,
            [(user, namespace, subresource) for user, namespace, subresource, _ in events[1:]],
        )

    def test_flagger_gets_secrets_only_in_tenant_namespaces(self) -> None:
        events = [
            (FLAGGER, "leitbild-testing", "get"),
            (FLAGGER, "leitbild-production", "get"),
            (FLAGGER, "leitbild-testing", "list"),
            (FLAGGER, "kube-system", "get"),
            ("system:serviceaccount:leitbild-testing:leitbild", "leitbild-testing", "get"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            events_file = Path(directory) / "audit.jsonl"
            events_file.write_text(
                "".join(secret_event(i, *event) + "\n" for i, event in enumerate(events, 1)),
                encoding="utf-8",
            )

            result = validate(Path("--k8saudit-replay"), events_file)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        fired = [tuple(line.split("\t")[:3]) for line in result.stdout.splitlines()]
        rule = "Baukit secret read by unexpected service account"
        self.assertEqual(
            fired,
            [(rule, user, namespace) for user, namespace, _ in events[2:]],
        )


class ImagePinTest(unittest.TestCase):
    def test_release_uses_the_validated_falco_image(self) -> None:
        manifest = tomllib.loads((REPO_ROOT / "deploy/tools.toml").read_text(encoding="utf-8"))
        image = manifest["images"]["falco"]
        tag = image.split(":", 1)[1]

        release = (FALCO_DIR / "helm-release.yaml").read_text(encoding="utf-8")

        self.assertIn(f'tag: "{tag}"', release)
        self.assertIn(f'tag: "{tag}"', K8SAUDIT_RELEASE.read_text(encoding="utf-8"))

    def test_k8saudit_plugin_is_pinned_by_digest(self) -> None:
        release = K8SAUDIT_RELEASE.read_text(encoding="utf-8")

        self.assertRegex(
            release,
            r"- ghcr\.io/falcosecurity/plugins/plugin/k8saudit:[0-9.]+@sha256:[a-f0-9]{64}\n",
        )
        self.assertRegex(release, r'repository: falcosecurity/falcoctl\n\s+tag: "[0-9.]+@sha256:')
        self.assertIn("indexes: []", release)


def macro_condition(text: str, name: str) -> str:
    match = re.search(
        rf"^- macro: {name}\n  condition: (>\n(?:    .*\n)+|.*\n)", text, re.MULTILINE
    )
    if match is None:
        raise AssertionError(f"macro {name} not found")
    return " ".join(match.group(1).removeprefix(">").split())


def config_maps(rendered: str) -> dict[str, str]:
    found = {}
    for document in re.split(r"^---$", rendered, flags=re.MULTILINE):
        name = re.search(r"^  name: (\S+)$", document, re.MULTILINE)
        if re.search(r"^kind: ConfigMap$", document, re.MULTILINE) and name:
            found[name.group(1)] = document
    return found


def kustomize_build(directory: Path) -> str:
    tools = REPO_ROOT / ".tools/bin/kustomize"
    binary = str(tools) if os.access(tools, os.X_OK) else shutil.which("kustomize")
    if binary is None:
        raise AssertionError("kustomize missing; run scripts/install-deploy-tools.sh")
    result = PROCESS_RUNNER.run_process(
        [binary, "build", str(directory)], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return result.stdout


class SharedKernelFilterTest(unittest.TestCase):
    def test_base_watches_host_processes(self) -> None:
        rules = BAUKIT_RULES.read_text(encoding="utf-8")
        condition = macro_condition(rules, "baukit_shared_kernel")

        self.assertEqual(condition, "(never_true)")

    def test_composed_platform_keeps_the_base_default(self) -> None:
        rendered = kustomize_build(PLATFORM_DIR / "overlays/everything")

        maps = config_maps(rendered)
        base = maps["falco-baukit-rules"]
        self.assertIn("baukit_shared_kernel\n      condition: (never_true)", base)
        self.assertNotIn("falco-overlay-rules", maps)

    def test_local_component_turns_the_filter_on(self) -> None:
        rendered = kustomize_build(PLATFORM_DIR / "overlays/local/components/falco")

        overlay = config_maps(rendered)["falco-overlay-rules"]
        self.assertIn("  namespace: falco", overlay)
        self.assertRegex(
            overlay,
            r"- macro: baukit_shared_kernel\n\s+condition: \(evt\.num >= 0\)\n"
            r"\s+override:\n\s+condition: replace",
        )


class RuntimeExecTest(unittest.TestCase):
    rules = BAUKIT_RULES.read_text(encoding="utf-8")

    def test_an_exec_with_a_lost_parent_still_counts(self) -> None:
        # Upstream container_entrypoint is "not proc.pname exists or proc.pname in (runc, ...)".
        condition = macro_condition(self.rules, "baukit_runtime_exec")

        self.assertIn("container_entrypoint", condition)
        self.assertNotIn("pname", condition)

    def test_cnpg_exceptions_name_each_process(self) -> None:
        child = macro_condition(self.rules, "baukit_cnpg_instance_manager_child")
        exec_ = macro_condition(self.rules, "baukit_cnpg_runtime_exec")

        self.assertIn("k8s.ns.name = postgres", child)
        self.assertIn("container.image.repository = ghcr.io/cloudnative-pg/postgresql", child)
        self.assertEqual(
            set(re.findall(r"proc\.name = (\S+?)\)?(?:\s|$)", child)),
            {"pg_isready", "sh", "barman-cloud-wa"},
        )
        self.assertIn('proc.cmdline startswith "sh -c -- /controller/manager wal-archive "', child)
        self.assertEqual(
            set(re.findall(r'proc\.cmdline (?:=|startswith) "([^"]+)"', exec_)),
            {"manager backup ", "manager healthcheck unix"},
        )

    def test_barman_plugin_exception_names_only_the_sidecar_scripts(self) -> None:
        plugin = macro_condition(self.rules, "baukit_cnpg_plugin_child")
        exec_ = macro_condition(self.rules, "baukit_cnpg_runtime_exec")

        self.assertIn("or baukit_cnpg_plugin_child", exec_)
        self.assertIn("k8s.ns.name = postgres", plugin)
        sidecar = "ghcr.io/cloudnative-pg/plugin-barman-cloud-sidecar"
        self.assertIn(f"container.image.repository = {sidecar}", plugin)
        self.assertIn("container.name = plugin-barman-cloud", plugin)
        self.assertEqual(
            set(re.findall(r"proc\.name = (\S+)", plugin)), {"barman-cloud-ba", "barman-cloud-wa"}
        )
        self.assertEqual(
            set(re.findall(r'proc\.cmdline startswith "[^ ]+ /venv/bin/([^"]+)"', plugin)),
            {
                "barman-cloud-backup-list ",
                "barman-cloud-backup-delete ",
                "barman-cloud-wal-archive ",
            },
        )
        self.assertNotRegex(plugin, r"proc\.name (=|in) \(?sh")

    def test_flux_exception_names_only_git_in_kustomize_controller(self) -> None:
        child = macro_condition(self.rules, "baukit_flux_controller_child")

        self.assertEqual(
            set(re.findall(r"(\S+) = (\S+?)\)?(?:\s|$)", child)),
            {
                ("(k8s.ns.name", "flux-system"),
                ("container.image.repository", "ghcr.io/fluxcd/kustomize-controller"),
                ("proc.name", "git"),
            },
        )
        self.assertEqual(child.count(" and "), 2)
        self.assertNotIn(" or ", child)
        self.assertNotIn("source-controller", child)
        self.assertNotIn("sh", re.findall(r"proc\.name = (\S+?)\)?(?:\s|$)", child))

    def test_protected_exec_rule_excludes_the_flux_controller_git_children(self) -> None:
        rule = self.rules.split("- rule: Baukit exec into protected platform namespace", 1)[1]
        condition = rule.split("  output:", 1)[0]

        self.assertIn("and not baukit_flux_controller_child", condition)
        self.assertIn("baukit_runtime_exec", condition)

    def test_kyverno_exception_names_only_the_legacy_policy_check(self) -> None:
        check = macro_condition(self.rules, "baukit_kyverno_legacy_check")
        rule = self.rules.split("- rule: Baukit exec into protected platform namespace", 1)[1]
        condition = rule.split("  output:", 1)[0]

        self.assertIn("k8s.ns.name = kyverno", check)
        self.assertIn("container.image.repository = reg.kyverno.io/kyverno/kyverno-cli", check)
        self.assertIn('proc.cmdline = "kubectl-kyverno check-legacy-policies"', check)
        self.assertNotIn("startswith", check)
        self.assertNotIn(" or ", check)
        self.assertIn("and not baukit_kyverno_legacy_check", condition)


if __name__ == "__main__":
    unittest.main()

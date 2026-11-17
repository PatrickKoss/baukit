import importlib.util
import tempfile
import unittest
from pathlib import Path

E2E = Path(__file__).resolve().parent
PLATFORM = E2E.parent
POLICIES = PLATFORM / "kyverno-policies/policies"
CASES = E2E / "tests/admission/cases"
CASE_FILES = {"deny.yaml", "admit.yaml"}
OPTIONAL_FILES = {"setup.yaml", "as"}
VALIDATING_KINDS = {"ValidatingPolicy", "ImageValidatingPolicy"}
IRREGULAR_PLURALS = {
    "Canary": "canaries",
    "Ingress": "ingresses",
    "NetworkPolicy": "networkpolicies",
}


def load_helm_yaml():
    path = PLATFORM / "lint/helm_yaml.py"
    spec = importlib.util.spec_from_file_location("helm_yaml", path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


helm_yaml = load_helm_yaml()


def parse_file(path):
    documents = [d for d in helm_yaml.split_documents(path.read_text()) if d.strip()]
    return helm_yaml.parse_documents(documents)


def deny_policies(policy_dir=POLICIES):
    policies = {}
    for path in sorted(policy_dir.glob("*.yaml")):
        for policy in parse_file(path):
            if policy.get("kind") not in VALIDATING_KINDS:
                continue
            if "Deny" in policy.get("spec", {}).get("validationActions", []):
                policies[policy["metadata"]["name"]] = policy
    return policies


def resource_of(manifest):
    group, _, _ = manifest["apiVersion"].rpartition("/")
    kind = manifest["kind"]
    return group, IRREGULAR_PLURALS.get(kind, kind.lower() + "s")


def policy_matches(policy, manifest):
    group, resource = resource_of(manifest)
    rules = policy["spec"].get("matchConstraints", {}).get("resourceRules", [])
    return any(
        group in rule.get("apiGroups", []) and resource in rule.get("resources", [])
        for rule in rules
    )


def case_problems(policies, cases_dir=CASES):
    problems = []
    cases = (
        {path.name for path in cases_dir.iterdir() if path.is_dir()}
        if cases_dir.is_dir()
        else set()
    )
    problems += [
        f"Deny policy {name} has no admission case" for name in sorted(set(policies) - cases)
    ]
    problems += [f"case {name} names no Deny policy" for name in sorted(cases - set(policies))]
    for name in sorted(cases & set(policies)):
        files = {path.name for path in (cases_dir / name).iterdir()}
        problems += [f"case {name} lacks {file}" for file in sorted(CASE_FILES - files)]
        problems += [
            f"case {name} has unexpected {file}"
            for file in sorted(files - CASE_FILES - OPTIONAL_FILES)
        ]
        for file in sorted(CASE_FILES & files):
            manifests = parse_file(cases_dir / name / file)
            if len(manifests) != 1:
                problems.append(f"case {name}/{file} holds {len(manifests)} manifests, not one")
            elif not policy_matches(policies[name], manifests[0]):
                problems.append(
                    f"case {name}/{file} is a {manifests[0]['kind']} the policy does not match"
                )
    return problems


class AdmissionCaseTest(unittest.TestCase):
    def test_every_deny_policy_has_one_matching_case(self):
        self.assertEqual(case_problems(deny_policies()), [])

    def test_deny_policies_are_found(self):
        self.assertIn("require-probes", deny_policies())
        self.assertNotIn("restrict-wildcard-rbac", deny_policies())

    def write(self, path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_reports_missing_extra_and_unmatched_cases(self):
        policy = deny_policies()["require-probes"]
        deployment = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: d\n"
        service = "apiVersion: v1\nkind: Service\nmetadata:\n  name: s\n"
        with tempfile.TemporaryDirectory() as tmp:
            cases = Path(tmp)
            self.write(cases / "require-probes/deny.yaml", service)
            self.write(cases / "require-probes/notes.txt", "")
            self.write(cases / "stale/deny.yaml", deployment)
            problems = case_problems({"require-probes": policy, "other": policy}, cases)
        self.assertEqual(
            problems,
            [
                "Deny policy other has no admission case",
                "case stale names no Deny policy",
                "case require-probes lacks admit.yaml",
                "case require-probes has unexpected notes.txt",
                "case require-probes/deny.yaml is a Service the policy does not match",
            ],
        )


if __name__ == "__main__":
    unittest.main()

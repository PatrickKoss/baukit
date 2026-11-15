"""Checks the LogQL alert rules in deploy/observability-platform/loki-rules. Needs helm in .tools.

promtool cannot evaluate LogQL, so each rule's line filters are matched against a log line copied
from the emitting source at the pinned version, and against a quiet line that must not match.
"""

from __future__ import annotations

import importlib.util
import os
import re
import unittest
from unittest import mock
from pathlib import Path
from types import ModuleType

ROOT = Path(__file__).resolve().parents[3]
LOKI_RULES = ROOT / "deploy/observability-platform/loki-rules"
RUNBOOKS = ROOT / "docs/runbooks/platform"
SEVERITIES = {"critical", "warning", "info"}
LINE_FILTER = re.compile(r'\|=\s*"((?:[^"\\]|\\.)*)"')
OTHER_FILTER = re.compile(r'!=|\|~|!~')

# Traefik v3.7.13 pkg/provider/kubernetes/crd/kubernetes.go:182 logs
# Msgf("Provider error, retrying in %s", time) through zerolog's ConsoleWriter (no log.format set).
SAMPLES = {
    "BaukitTraefikConfigReloadFailing": (
        '2026-11-15T10:00:00Z ERR Provider error, retrying in 1.5s error="ingressroutes.traefik.io'
        ' is forbidden" providerName=kubernetescrd',
        "2026-11-15T10:00:00Z INF Starting provider *crd.Provider",
    ),
}


def filters_match(expression: str, line: str) -> bool:
    filters = LINE_FILTER.findall(expression)
    return bool(filters) and all(text in line for text in filters)


def load_helm_yaml() -> ModuleType:
    path = ROOT / "deploy/platform/lint/helm_yaml.py"
    spec = importlib.util.spec_from_file_location("helm_yaml", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def loki_rules() -> list[tuple[Path, dict]]:
    helm_yaml = load_helm_yaml()
    paths = sorted(LOKI_RULES.glob("*.y*ml"))
    tools_path = f"{ROOT / '.tools/bin'}{os.pathsep}{os.environ.get('PATH', '')}"
    with mock.patch.dict(os.environ, {"PATH": tools_path}):
        documents = helm_yaml.parse_documents([path.read_text() for path in paths])
    return [
        (path, rule)
        for path, document in zip(paths, documents, strict=True)
        for group in document.get("groups") or []
        for rule in group.get("rules") or []
    ]


class LokiRulesTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.rules = loki_rules()

    def test_pack_has_loki_rules(self) -> None:
        self.assertTrue(self.rules)

    def test_metadata_and_runbooks(self) -> None:
        for path, rule in self.rules:
            name = str(rule.get("alert", ""))
            with self.subTest(alert=name):
                self.assertTrue(name.startswith("Baukit"))
                labels = rule.get("labels") or {}
                self.assertIn(labels.get("severity"), SEVERITIES)
                self.assertEqual(labels.get("component"), path.name.split(".")[0])
                annotations = rule.get("annotations") or {}
                for key in ("summary", "description"):
                    self.assertTrue(str(annotations.get(key, "")).strip())
                url = str(annotations.get("runbook_url", ""))
                self.assertTrue(url.startswith("RUNBOOK_BASE_URL/"), url)
                slug = url.removeprefix("RUNBOOK_BASE_URL/")
                self.assertTrue((RUNBOOKS / f"{slug}.md").is_file(), slug)

    def test_line_filters_match_the_source_log_line(self) -> None:
        self.assertEqual({str(rule.get("alert")) for _, rule in self.rules}, set(SAMPLES))
        for _, rule in self.rules:
            name = str(rule.get("alert"))
            expression = str(rule.get("expr", ""))
            firing, quiet = SAMPLES[name]
            with self.subTest(alert=name):
                self.assertIsNone(OTHER_FILTER.search(expression), "only |= filters are checked")
                self.assertTrue(filters_match(expression, firing))
                self.assertFalse(filters_match(expression, quiet))

    def test_a_filter_that_drifts_from_the_source_fails(self) -> None:
        firing, _ = SAMPLES["BaukitTraefikConfigReloadFailing"]
        drifted = '{namespace="traefik"} |= "Provider error, retrying after"'
        self.assertFalse(filters_match(drifted, firing))
        self.assertFalse(filters_match('{namespace="traefik"}', firing))


if __name__ == "__main__":
    unittest.main()

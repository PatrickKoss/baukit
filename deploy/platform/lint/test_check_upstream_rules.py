from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_upstream_rules as cur

RULES = {"storage.rules.yml": ("chart.yaml", "stack-storage")}
GROUPS = """groups:
- name: storage
  rules:
  - alert: VolumeFull
    expr: free < {threshold}
"""
RENDERED = """apiVersion: v1
kind: ConfigMap
metadata:
  name: other
---
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: {name}
spec:
  groups:
  - name: storage
    rules:
    - alert: VolumeFull
      expr: free < 0.03
"""


class UpstreamRulesTest(unittest.TestCase):
    def check(self, copy: str, rendered: str, extra: str | None = None) -> list[str]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "upstream").mkdir()
            (root / "upstream/storage.rules.yml").write_text(copy)
            if extra is not None:
                (root / "upstream" / extra).write_text(copy)
            (root / "chart.yaml").write_text(rendered)
            return cur.check(root, root / "upstream", RULES)

    def test_matching_copy_passes(self) -> None:
        copy = GROUPS.format(threshold="0.03")
        self.assertEqual(self.check(copy, RENDERED.format(name="stack-storage")), [])

    def test_changed_upstream_expression_fails(self) -> None:
        copy = GROUPS.format(threshold="0.05")
        [problem] = self.check(copy, RENDERED.format(name="stack-storage"))
        self.assertIn("differs from PrometheusRule stack-storage", problem)

    def test_rule_missing_from_render_fails(self) -> None:
        [problem] = self.check(GROUPS.format(threshold="0.03"), RENDERED.format(name="renamed"))
        self.assertIn("has no PrometheusRule stack-storage", problem)

    def test_unlisted_copy_fails(self) -> None:
        copy = GROUPS.format(threshold="0.03")
        [problem] = self.check(copy, RENDERED.format(name="stack-storage"), extra="extra.rules.yml")
        self.assertIn("extra.rules.yml has no source", problem)

    def test_committed_copies_are_listed(self) -> None:
        names = {path.name for path in cur.UPSTREAM_DIR.glob("*.y*ml")}
        self.assertEqual(names, set(cur.UPSTREAM_RULES))


if __name__ == "__main__":
    unittest.main()

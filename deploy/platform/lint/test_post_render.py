from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import helm_yaml
import post_render

TOOLS = Path(__file__).resolve().parents[3] / ".tools/bin"

RELEASE = """\
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: web
spec:
  postRenderers:
    - kustomize:
        patches:
          - target:
              kind: Deployment
            patch: |-
              - op: add
                path: /spec/template/spec/priorityClassName
                value: platform-standard
          - target:
              version: v1
              kind: Pod
            patch: |-
              - op: add
                path: /spec/priorityClassName
                value: platform-standard
"""

RENDERED = """\
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
spec:
  template:
    spec:
      containers:
        - name: web
          image: web
---
apiVersion: v1
kind: Pod
metadata:
  name: web-test
  annotations:
    helm.sh/hook: test
spec:
  containers:
    - name: test
      image: test
"""


class KustomizationsTest(unittest.TestCase):
    def test_kustomize_renderers_become_kustomizations(self) -> None:
        release = {"spec": {"postRenderers": [{"kustomize": {"patches": [{"patch": "p"}]}}]}}
        self.assertEqual(
            post_render.kustomizations(release),
            [{"resources": ["rendered.yaml"], "patches": [{"patch": "p"}]}],
        )

    def test_release_without_renderers_has_no_steps(self) -> None:
        self.assertEqual(post_render.kustomizations({"spec": {}}), [])

    def test_unsupported_renderers_fail(self) -> None:
        for renderer in ({"kustomize": {"patchesJson6902": []}}, {"other": {}}, "kustomize"):
            with (
                self.subTest(renderer=renderer),
                self.assertRaises(post_render.PostRenderError),
            ):
                post_render.kustomizations({"spec": {"postRenderers": [renderer]}})


class MainTest(unittest.TestCase):
    def run_main(self, release: str) -> tuple[int, list[dict], str]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "release.yaml").write_text(release)
            rendered = root / "rendered.yaml"
            rendered.write_text(RENDERED)
            stderr = io.StringIO()
            argv = ["--release", str(root / "release.yaml"), "--rendered", str(rendered)]
            with contextlib.redirect_stderr(stderr):
                code = post_render.main([*argv, "--kustomize", str(TOOLS / "kustomize")])
            documents = helm_yaml.parse_documents(helm_yaml.split_documents(rendered.read_text()))
        return code, [document for document in documents if document], stderr.getvalue()

    def test_patches_reach_workloads_and_hooks(self) -> None:
        code, documents, stderr = self.run_main(RELEASE)
        self.assertEqual(code, 0, stderr)
        by_kind = {document["kind"]: document for document in documents}
        self.assertEqual(
            by_kind["Deployment"]["spec"]["template"]["spec"]["priorityClassName"],
            "platform-standard",
        )
        self.assertEqual(by_kind["Pod"]["spec"]["priorityClassName"], "platform-standard")

    def test_release_without_renderers_keeps_the_render(self) -> None:
        release = "apiVersion: helm.toolkit.fluxcd.io/v2\nkind: HelmRelease\nspec: {}\n"
        code, documents, stderr = self.run_main(release)
        self.assertEqual(code, 0, stderr)
        self.assertNotIn("priorityClassName", json.dumps(documents))

    def test_failing_patch_fails(self) -> None:
        release = RELEASE.replace("/spec/priorityClassName", "/spec/missing/priorityClassName")
        code, _, stderr = self.run_main(release)
        self.assertEqual(code, 1)
        self.assertIn("kustomize post-render failed", stderr)


if __name__ == "__main__":
    unittest.main()

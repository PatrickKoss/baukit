import datetime
import os
import stat
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import platform_images

run_process = platform_images.process_runner.run_process

TODAY = datetime.date(2026, 11, 14)
DIGEST = "@sha256:" + "a" * 64
RENDERED = """\
apiVersion: apps/v1
kind: Deployment
spec:
  template:
    spec:
      initContainers:
        - name: init
          image: busybox:1.38.0
      containers:
        - name: app
          image: ghcr.io/example/app:1.0.0
---
apiVersion: v1
kind: Pod
spec:
  ephemeralContainers:
    - name: debug
      image: docker.io/library/alpine:3.22
  containers:
    - name: app
      image: ghcr.io/example/app:1.0.0
---
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
spec:
  imageName: ghcr.io/cloudnative-pg/postgresql:18.6-system-trixie
---
apiVersion: postgresql.cnpg.io/v1
kind: ImageCatalog
spec:
  images:
    - major: 18
      image: ghcr.io/cloudnative-pg/postgresql:18.6{digest}
---
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: falco
spec:
  template:
    spec:
      containers:
        - name: falco
          image: docker.io/falcosecurity/falco:0.45.2
        - name: falcosidekick
          image: docker.io/falcosecurity/falcosidekick:2.35.1
---
apiVersion: kyverno.io/v1
kind: ClusterPolicy
spec:
  rules:
    - validate:
        pattern:
          spec:
            containers:
              - image: "!*:latest"
""".replace("{digest}", DIGEST)
RAW_PATCH = """\
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
patches:
  - patch: |
      - op: replace
        path: /spec/jobTemplate/spec/template/spec/containers/0/image
        value: rancher/k3s:v1.34.8-k3s1
"""
RAW_LIST_PATCH = """\
- op: replace
  path: /spec/template/spec/containers/0/image
  value: "quay.io/example/patched:2.0"
"""


class ExtractImagesTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)

    def write(self, name: str, text: str) -> Path:
        path = self.directory / name
        path.write_text(text)
        return path

    def test_every_image_source_kind_is_found_once(self):
        paths = [
            self.write("chart.yaml", RENDERED),
            self.write("kustomization.yaml", RAW_PATCH),
            self.write("patch.yaml", RAW_LIST_PATCH),
        ]
        images = platform_images.extract_images(platform_images.load_documents(paths))
        self.assertEqual(
            images,
            [
                "busybox:1.38.0",
                "docker.io/falcosecurity/falco:0.45.2",
                "docker.io/falcosecurity/falcosidekick:2.35.1",
                "docker.io/library/alpine:3.22",
                "ghcr.io/cloudnative-pg/postgresql:18.6-system-trixie",
                f"ghcr.io/cloudnative-pg/postgresql:18.6{DIGEST}",
                "ghcr.io/example/app:1.0.0",
                "quay.io/example/patched:2.0",
                "rancher/k3s:v1.34.8-k3s1",
            ],
        )


class ImageReferenceTests(unittest.TestCase):
    def test_patterns_and_schema_text_are_not_images(self):
        for candidate in ["!*:latest", "?*", "{{ .Values.image }}", "10.0.0.0/8 ", "string", ""]:
            with self.subTest(candidate=candidate):
                self.assertFalse(platform_images.is_image_reference(candidate))

    def test_tag_and_digest_references_are_images(self):
        for candidate in [
            "redis:8.10.2-alpine",
            f"posthog/posthog{DIGEST}",
            "localhost:5000/app:1",
        ]:
            with self.subTest(candidate=candidate):
                self.assertTrue(platform_images.is_image_reference(candidate))


class IgnoreFileTests(unittest.TestCase):
    def problems(self, text: str) -> list[str]:
        return [str(problem) for problem in platform_images.check_ignore_file(text, TODAY)]

    def test_entry_with_reason_and_future_expiry_passes(self):
        self.assertEqual(self.problems("# bump pending in SC2\nCVE-2026-0001 exp:2026-11-28\n"), [])

    def test_entry_without_reason_fails(self):
        self.assertEqual(
            self.problems("CVE-2026-0001 exp:2026-11-28\n"),
            ["line 1: CVE-2026-0001 has no reason comment above it"],
        )

    def test_blank_line_separates_a_reason_from_the_entry(self):
        problems = self.problems("# bump pending in SC2\n\nCVE-2026-0001 exp:2026-11-28\n")
        self.assertEqual(problems, ["line 3: CVE-2026-0001 has no reason comment above it"])

    def test_entry_without_expiry_fails(self):
        self.assertEqual(
            self.problems("# bump pending in SC2\nCVE-2026-0001\n"),
            ["line 2: CVE-2026-0001 has no exp:YYYY-MM-DD expiry"],
        )

    def test_expired_entry_fails(self):
        self.assertEqual(
            self.problems("# bump pending in SC2\nCVE-2026-0001 exp:2026-11-13\n"),
            ["line 2: CVE-2026-0001 expired on 2026-11-13"],
        )

    def test_invalid_date_fails(self):
        self.assertEqual(
            self.problems("# reason\nCVE-2026-0001 exp:2026-13-01\n"),
            ["line 2: 2026-13-01 is not a valid date"],
        )


class JinjaDockerfileTests(unittest.TestCase):
    def test_superset_render_keeps_every_branch(self):
        template = (
            "FROM base\n{% if x %}RUN apt-get install {{ package }}{% endif %}\n"
            "{% for s in stages %}FROM {{ s.image }} AS {{ s.name }}\n{% endfor %}"
        )
        self.assertEqual(
            platform_images.render_jinja_superset(template),
            "FROM base\nRUN apt-get install placeholder\nFROM placeholder AS placeholder\n",
        )


FAKE_TRIVY = """\
#!/bin/sh
printf '%s\\n' "$*" >>"$TRIVY_CALLS"
for argument in "$@"; do
  case $argument in
    *vulnerable*) exit 1 ;;
  esac
done
exit 0
"""


class MainTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        bin_dir = self.directory / "bin"
        bin_dir.mkdir()
        trivy = bin_dir / "trivy"
        trivy.write_text(FAKE_TRIVY)
        trivy.chmod(trivy.stat().st_mode | stat.S_IXUSR)
        self.calls = self.directory / "calls"
        self.calls.touch()
        self.environment = {
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "TRIVY_CALLS": str(self.calls),
        }
        self.repo = self.directory / "repo"
        (self.repo / "deploy").mkdir(parents=True)
        (self.repo / "deploy" / "app.Dockerfile").write_text("FROM scratch\n")
        run_process(["git", "init", "-q", str(self.repo)], check=True)
        run_process(["git", "-C", str(self.repo), "add", "."], check=True)
        self.rendered = self.directory / "rendered"
        self.rendered.mkdir()
        self.ignore = self.directory / ".trivyignore"
        self.ignore.write_text("")

    def run_main(self, image: str) -> int:
        (self.rendered / "base.yaml").write_text(
            f"kind: Pod\nspec:\n  containers:\n    - image: {image}\n"
        )
        arguments = [
            "--rendered", str(self.rendered), "--repo-root", str(self.repo),
            "--cache", str(self.directory / "cache"), "--work", str(self.directory / "work"),
            "--ignore-file", str(self.ignore), "--today", TODAY.isoformat(),
        ]  # fmt: skip
        with mock.patch.dict(os.environ, self.environment):
            return platform_images.main(arguments)

    def test_clean_images_and_dockerfiles_pass(self):
        self.assertEqual(self.run_main("ghcr.io/example/clean:1.0"), 0)
        calls = self.calls.read_text()
        self.assertIn("--ignore-unfixed --exit-code 1", calls)
        self.assertIn("ghcr.io/example/clean:1.0", calls)
        self.assertRegex(calls, r"(?m)^config .*/work/dockerfiles$")
        staged = self.directory / "work" / "dockerfiles" / "deploy__app." / "Dockerfile"
        self.assertTrue(staged.is_file())

    def test_finding_fails_the_step(self):
        self.assertEqual(self.run_main("ghcr.io/example/vulnerable:1.0"), 1)

    def test_invalid_ignore_file_fails_the_step(self):
        self.ignore.write_text("CVE-2026-0001\n")
        self.assertEqual(self.run_main("ghcr.io/example/clean:1.0"), 1)


if __name__ == "__main__":
    unittest.main()

import base64
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


class SecretImageTests(unittest.TestCase):
    def test_base64_image_keys_in_a_secret_are_found(self):
        sidecar = f"ghcr.io/cloudnative-pg/plugin-barman-cloud-sidecar:v0.16.0{DIGEST}"
        secret = {
            "kind": "Secret",
            "data": {
                "SIDECAR_IMAGE": base64.b64encode(sidecar.encode()).decode(),
                "password": base64.b64encode(b"redis:8.10.2").decode(),
            },
            "stringData": {"HELPER_IMAGE": "busybox:1.38.0"},
        }
        self.assertEqual(platform_images.extract_images([secret]), ["busybox:1.38.0", sidecar])

    def test_image_keys_outside_a_secret_are_not_decoded(self):
        config = {"kind": "ConfigMap", "data": {"SIDECAR_IMAGE": "cmVkaXM6OA=="}}
        self.assertEqual(platform_images.extract_images([config]), [])

    def test_undecodable_secret_image_fails(self):
        secret = {"kind": "Secret", "data": {"SIDECAR_IMAGE": "not base64!"}}
        with self.assertRaises(platform_images.ScanError):
            platform_images.extract_images([secret])


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


def allowance(ids, expires=datetime.date(2026, 11, 28), reason="bump pending in SC2"):
    return platform_images.Allowance(expires, reason, frozenset(ids))


class CompareTests(unittest.TestCase):
    def compare(self, reported, allowed, known=None):
        return platform_images.compare(reported, allowed, TODAY, known)

    def test_listed_findings_pass(self):
        self.assertEqual(self.compare({"img:1": {"CVE-1"}}, {"img:1": allowance(["CVE-1"])}), [])

    def test_unlisted_finding_fails(self):
        self.assertEqual(
            self.compare({"img:1": {"CVE-1"}}, {}), ["img:1: CVE-1 is not in the allowlist"]
        )

    def test_allowance_does_not_cover_another_image(self):
        problems = self.compare(
            {"img:1": {"CVE-1"}, "img:2": {"CVE-1"}}, {"img:1": allowance(["CVE-1"])}
        )
        self.assertEqual(problems, ["img:2: CVE-1 is not in the allowlist"])

    def test_stale_id_fails_so_the_list_only_shrinks(self):
        problems = self.compare({"img:1": {"CVE-1"}}, {"img:1": allowance(["CVE-1", "CVE-2"])})
        self.assertEqual(problems, ["img:1: CVE-2 is no longer reported; remove it from the list"])

    def test_listed_id_rescored_below_the_gate_is_not_stale(self):
        problems = self.compare({}, {"img:1": allowance(["CVE-1"])}, {"img:1": {"CVE-1"}})
        self.assertEqual(problems, [])

    def test_listed_id_gone_at_every_severity_is_stale(self):
        problems = self.compare({}, {"img:1": allowance(["CVE-1"])}, {"img:1": {"CVE-2"}})
        self.assertEqual(problems, ["img:1: CVE-1 is no longer reported; remove it from the list"])

    def test_entry_for_an_image_that_is_gone_fails(self):
        problems = self.compare({}, {"img:1": allowance(["CVE-1"])})
        self.assertEqual(problems, ["img:1: no findings left; remove the entry"])

    def test_expired_entry_fails(self):
        problems = self.compare(
            {"img:1": {"CVE-1"}}, {"img:1": allowance(["CVE-1"], datetime.date(2026, 11, 13))}
        )
        self.assertEqual(problems, ["img:1: expired on 2026-11-13"])

    def test_missing_expiry_fails(self):
        problems = self.compare({"img:1": {"CVE-1"}}, {"img:1": allowance(["CVE-1"], None)})
        self.assertEqual(problems, ["img:1: expires must be a YYYY-MM-DD date"])

    def test_reason_without_task_fails(self):
        problems = self.compare(
            {"img:1": {"CVE-1"}}, {"img:1": allowance(["CVE-1"], reason="later")}
        )
        self.assertEqual(problems, ["img:1: reason must name the follow-up task"])

    def test_empty_ids_fail(self):
        problems = self.compare({}, {"img:1": allowance([])})
        self.assertIn("img:1: ids is empty; remove the entry", problems)


class AllowlistFileTests(unittest.TestCase):
    def load(self, text):
        with tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False) as handle:
            handle.write(text)
        self.addCleanup(Path(handle.name).unlink)
        return platform_images.load_allowlist(Path(handle.name))

    def test_sections_parse_with_toml_dates(self):
        loaded = self.load(
            '[images."redis:8"]\nexpires = 2026-11-28\nreason = "SC2"\nids = ["CVE-1"]\n'
            '[dockerfiles."deploy/x.Dockerfile"]\nexpires = 2026-11-28\nreason = "S6"\n'
            'ids = ["DS-0002"]\n'
        )
        self.assertEqual(loaded["images"]["redis:8"], allowance(["CVE-1"], reason="SC2"))
        self.assertEqual(loaded["dockerfiles"]["deploy/x.Dockerfile"].ids, {"DS-0002"})

    def test_string_expiry_is_not_a_date(self):
        loaded = self.load('[images."a:1"]\nexpires = "2026-11-28"\nreason = "SC2"\nids = ["X"]\n')
        self.assertIsNone(loaded["images"]["a:1"].expires)

    def test_unknown_section_fails(self):
        with self.assertRaises(platform_images.ScanError):
            self.load('[ignore."a"]\nids = []\n')


class JinjaDockerfileTests(unittest.TestCase):
    def test_superset_render_keeps_every_branch(self):
        template = (
            "FROM base\n{% if x %}RUN apt-get install {{ package }}{% endif %}\n"
            "{% for s in stages %}FROM {{ s.image }} AS {{ s.name }}\n{% endfor %}"
        )
        self.assertEqual(
            platform_images.render_jinja_superset(template),
            "FROM base\nRUN apt-get install placeholder1\nFROM placeholder2 AS placeholder3\n",
        )


FAKE_TRIVY = """\
#!/bin/sh
printf '%s\\n' "$*" >>"$TRIVY_CALLS"
case " $* " in
  *" --download-"*) exit 0 ;;
  *" config "*)
    if [ -n "${FAKE_DS:-}" ]; then
      printf '{"Results":[{"Target":"deploy__app./Dockerfile","Misconfigurations":'
      printf '[{"ID":"DS-0002","Status":"FAIL"},{"ID":"DS-0001","Status":"PASS"}]}]}'
    else
      printf '{"Results":[]}'
    fi
    exit 0 ;;
  *vulnerable*)
    printf '{"Results":[{"Target":"x","Vulnerabilities":'
    printf '[{"VulnerabilityID":"CVE-2026-1","Status":"fixed","Severity":"HIGH"}]}]}'
    exit 0 ;;
  *rescored*)
    printf '{"Results":[{"Target":"x","Vulnerabilities":'
    printf '[{"VulnerabilityID":"CVE-2026-1","Status":"fixed","Severity":"MEDIUM"}]}]}'
    exit 0 ;;
  *broken*) exit 1 ;;
esac
printf '{"Results":[{"Target":"x","Vulnerabilities":null}]}'
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
        self.allowlist = self.directory / "image-findings.toml"
        self.allowlist.write_text("")

    def run_main(self, image: str, *extra: str, **environment: str) -> int:
        (self.rendered / "base.yaml").write_text(
            f"kind: Pod\nspec:\n  containers:\n    - image: {image}\n"
        )
        arguments = [
            "--rendered", str(self.rendered), "--repo-root", str(self.repo),
            "--cache", str(self.directory / "cache"), "--work", str(self.directory / "work"),
            "--allowlist", str(self.allowlist), "--today", TODAY.isoformat(), *extra,
        ]  # fmt: skip
        with mock.patch.dict(os.environ, self.environment | environment):
            return platform_images.main(arguments)

    def test_clean_images_and_dockerfiles_pass(self):
        self.assertEqual(self.run_main("ghcr.io/example/clean:1.0"), 0)
        calls = self.calls.read_text()
        self.assertIn("--ignore-unfixed --format json ghcr.io/example/clean:1.0", calls)
        self.assertIn("--scanners vuln", calls)
        self.assertNotIn("--ignorefile", calls)
        self.assertRegex(calls, r"(?m)^config .*/work/dockerfiles$")

    def test_unlisted_image_finding_fails(self):
        self.assertEqual(self.run_main("ghcr.io/example/vulnerable:1.0"), 1)

    def test_image_scans_read_every_severity(self):
        self.run_main("ghcr.io/example/clean:1.0")
        image_calls = [line for line in self.calls.read_text().splitlines() if "clean:1.0" in line]
        self.assertEqual(len(image_calls), 1)
        self.assertNotIn("--severity", image_calls[0])

    def test_unlisted_finding_below_the_gate_passes(self):
        self.assertEqual(self.run_main("ghcr.io/example/rescored:1.0"), 0)

    def test_listed_finding_rescored_below_the_gate_passes(self):
        self.allowlist.write_text(
            '[images."ghcr.io/example/rescored:1.0"]\n'
            'expires = 2026-11-28\nreason = "bump pending in SC2"\nids = ["CVE-2026-1"]\n'
        )
        self.assertEqual(self.run_main("ghcr.io/example/rescored:1.0"), 0)

    def test_listed_image_finding_passes(self):
        self.allowlist.write_text(
            '[images."ghcr.io/example/vulnerable:1.0"]\n'
            'expires = 2026-11-28\nreason = "bump pending in SC2"\nids = ["CVE-2026-1"]\n'
        )
        self.assertEqual(self.run_main("ghcr.io/example/vulnerable:1.0"), 0)

    def test_dockerfile_finding_is_keyed_by_repository_path(self):
        self.assertEqual(self.run_main("ghcr.io/example/clean:1.0", FAKE_DS="1"), 1)
        self.allowlist.write_text(
            '[dockerfiles."deploy/app.Dockerfile"]\n'
            'expires = 2026-11-28\nreason = "USER pending in S6"\nids = ["DS-0002"]\n'
        )
        self.assertEqual(self.run_main("ghcr.io/example/clean:1.0", FAKE_DS="1"), 0)

    def test_image_scans_read_the_mirror_config_when_given(self):
        config = self.directory / "trivy.yaml"
        self.assertEqual(
            self.run_main("ghcr.io/example/clean:1.0", "--trivy-config", str(config)), 0
        )
        image_scan = next(
            line for line in self.calls.read_text().splitlines() if "ghcr.io/example/clean" in line
        )
        self.assertIn(f"--config {config}", image_scan)

    def test_image_scans_use_no_config_by_default(self):
        self.assertEqual(self.run_main("ghcr.io/example/clean:1.0"), 0)
        self.assertNotIn("--config", self.calls.read_text())

    def test_scans_download_databases_into_the_run_and_share_layers(self):
        shared = self.directory / "cache"
        (shared / "db").mkdir(parents=True)
        (shared / "db" / "metadata.json").write_text('{"NextUpdate": "2999-01-01T00:00:00Z"}')
        self.assertEqual(self.run_main("ghcr.io/example/clean:1.0"), 0)
        run_cache = self.directory / "work" / "trivy-cache"
        calls = self.calls.read_text().splitlines()
        self.assertIn(f"image --cache-dir {run_cache} --quiet --download-db-only", calls)
        for call in calls:
            with self.subTest(call=call):
                self.assertIn(f"--cache-dir {run_cache} ", call)
        self.assertEqual((run_cache / "fanal").resolve(), (shared / "fanal").resolve())

    def test_trivy_error_fails_the_step(self):
        self.assertEqual(self.run_main("ghcr.io/example/broken:1.0"), 1)


if __name__ == "__main__":
    unittest.main()

import io
import tarfile
import tempfile
import textwrap
import unittest
from pathlib import Path

import chart_contents as cc

RELEASE = """\
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: {name}
  namespace: demo
spec:
  chart:
    spec:
      chart: {chart}
      version: "{version}"
      sourceRef:
        kind: HelmRepository
        name: demo
"""
OCI_RELEASE = """\
apiVersion: source.toolkit.fluxcd.io/v1
kind: OCIRepository
metadata:
  name: ops
  namespace: demo
spec:
  url: oci://ghcr.io/acme/charts/ops
  ref:
    tag: "1.2.3"
---
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: ops
  namespace: demo
spec:
  chartRef:
    kind: OCIRepository
    name: ops
"""
TEST_HOOK = """\
apiVersion: v1
kind: Pod
metadata:
  name: check
  annotations:
    helm.sh/hook: test
"""
POLICY = """\
spec:
  variables:
    - name: {variable}
      expression: '{names}'
"""


def archive(path: Path, members: list[str]) -> None:
    with tarfile.open(path, "w:gz") as tar:
        for member in members:
            data = b"kind: CustomResourceDefinition\n"
            info = tarfile.TarInfo(member)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))


class Fixture:
    def __init__(self, root: Path) -> None:
        self.rendered = root / "rendered"
        self.cache = root / "cache"
        self.policies = root / "policies"
        for directory in (self.rendered, self.cache, self.policies):
            directory.mkdir()
        archive(
            self.cache / "ops-1.2.3-42.tgz", ["ops/Chart.yaml", "ops/charts/sub/crds/thing.yaml"]
        )
        archive(
            self.cache / "web-2.0.0-42.tgz", ["web/Chart.yaml", "web/templates/crds/thing.yaml"]
        )
        releases = [
            RELEASE.format(name="ops", chart="ops", version="1.2.3"),
            RELEASE.format(name="web", chart="web", version="2.0.0"),
        ]
        (self.rendered / "base-demo.yaml").write_text("---\n".join(releases))
        (self.rendered / "chart-demo-ops.yaml").write_text("kind: ConfigMap\n")
        (self.rendered / "chart-demo-web.yaml").write_text(TEST_HOOK)
        self.write_policies('["ops"]', '["web"]')
        self.data = root / "chart-contents.toml"

    def write_policies(self, crds: str, tests: str) -> None:
        for field, names in (("crds", crds), ("tests", tests)):
            policy, variable = cc.POLICY_LISTS[field]
            (self.policies / policy).write_text(POLICY.format(variable=variable, names=names))

    def collect(self) -> dict[str, cc.Contents]:
        return cc.collect(self.rendered, self.cache, self.rendered)


class ChartContentsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.fixture = Fixture(Path(self.tmp.name))

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_crds_come_from_helm_crds_directories_only(self) -> None:
        contents = self.fixture.collect()
        self.assertEqual(contents["ops@1.2.3"], cc.Contents(crds=True, tests=False))
        self.assertEqual(contents["web@2.0.0"], cc.Contents(crds=False, tests=True))

    def test_written_data_and_policies_pass_the_check(self) -> None:
        fixture = self.fixture
        contents = fixture.collect()
        fixture.write_policies("[]", "[]")
        cc.write(contents, fixture.data, fixture.policies)
        self.assertEqual(cc.check(contents, fixture.data, fixture.policies), [])
        self.assertEqual(
            cc.policy_list(fixture.policies / "helmrelease-crd-policy.yaml", "chartsWithCrds"),
            ["ops"],
        )

    def test_data_file_drift_fails(self) -> None:
        fixture = self.fixture
        contents = fixture.collect()
        cc.write(contents, fixture.data, fixture.policies)
        fixture.data.write_text(fixture.data.read_text().replace("crds = true", "crds = false"))
        problems = cc.check(contents, fixture.data, fixture.policies)
        self.assertTrue(any("ops@1.2.3" in problem for problem in problems), problems)

    def test_policy_list_drift_fails(self) -> None:
        fixture = self.fixture
        contents = fixture.collect()
        cc.write(contents, fixture.data, fixture.policies)
        fixture.write_policies("[]", '["web"]')
        problems = cc.check(contents, fixture.data, fixture.policies)
        self.assertEqual(len(problems), 1)
        self.assertIn("chartsWithCrds", problems[0])

    def test_unrecorded_chart_version_fails(self) -> None:
        fixture = self.fixture
        cc.write(fixture.collect(), fixture.data, fixture.policies)
        archive(fixture.cache / "ops-1.2.4-42.tgz", ["ops/Chart.yaml"])
        base = fixture.rendered / "base-demo.yaml"
        base.write_text(base.read_text().replace('"1.2.3"', '"1.2.4"'))
        problems = cc.check(fixture.collect(), fixture.data, fixture.policies)
        self.assertTrue(any("ops@1.2.4" in problem for problem in problems), problems)

    def test_chart_ref_reads_name_and_version_from_the_oci_repository(self) -> None:
        base = self.fixture.rendered / "base-demo.yaml"
        base.write_text(
            base.read_text().replace(
                RELEASE.format(name="ops", chart="ops", version="1.2.3"), OCI_RELEASE
            )
        )
        contents = self.fixture.collect()
        self.assertEqual(contents["ops@1.2.3"], cc.Contents(crds=True, tests=False))

    def test_chart_ref_without_its_oci_repository_raises(self) -> None:
        base = self.fixture.rendered / "base-demo.yaml"
        base.write_text(OCI_RELEASE.split("---\n", 1)[1])
        with self.assertRaisesRegex(cc.ContentsError, "missing OCIRepository demo/ops"):
            self.fixture.collect()

    def test_missing_chart_archive_raises(self) -> None:
        (self.fixture.cache / "ops-1.2.3-42.tgz").unlink()
        with self.assertRaises(cc.ContentsError):
            self.fixture.collect()

    def test_test_hook_detection(self) -> None:
        quoted = textwrap.dedent("""\
            metadata:
              annotations:
                "helm.sh/hook": test-success
            """)
        self.assertTrue(cc.TEST_HOOK.search(quoted))
        self.assertFalse(cc.TEST_HOOK.search("    helm.sh/hook: pre-install\n"))


class RepositoryDataTest(unittest.TestCase):
    def test_checked_in_policies_match_the_data_file(self) -> None:
        recorded = cc.load_data(cc.DATA_FILE)
        problems = cc.check(recorded, cc.DATA_FILE, cc.POLICY_DIR)
        self.assertEqual(problems, [])


if __name__ == "__main__":
    unittest.main()

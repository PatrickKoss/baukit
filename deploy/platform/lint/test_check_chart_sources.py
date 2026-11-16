import contextlib
import io
import tempfile
import unittest
from pathlib import Path

import check_chart_sources as ccs

ISSUER = r"^https://token\.actions\.githubusercontent\.com$"
SUBJECT = r"^https://github\.com/acme/charts/\.github/workflows/release\.yml@refs/heads/main$"
README = f"""# Bases

## Chart sources

| Chart | Source | Signed | Flux verify | Identity | Reason |
|---|---|---|---|---|---|
| `signed` | OCI `oci://ghcr.io/acme/charts/signed` | keyless | cosign | `{ISSUER}` `{SUBJECT}` | |
| `plain` | OCI `oci://ghcr.io/acme/charts/plain` | no | none | | Upstream does not sign. |
| `index` | HelmRepository `https://charts.example.com` | no | none | | No OCI artifact. |

## Next
"""


def oci_repository(name: str, url: str, verify: dict | None = None) -> dict:
    spec = {"url": url, "ref": {"tag": "1.0.0"}}
    if verify is not None:
        spec["verify"] = verify
    return {"kind": "OCIRepository", "metadata": {"name": name, "namespace": "demo"}, "spec": spec}


def chart_ref_release(name: str, source: str) -> dict:
    return {
        "kind": "HelmRelease",
        "metadata": {"name": name, "namespace": "demo"},
        "spec": {"chartRef": {"kind": "OCIRepository", "name": source}},
    }


def helm_repository_release(name: str, chart: str) -> dict:
    return {
        "kind": "HelmRelease",
        "metadata": {"name": name, "namespace": "demo"},
        "spec": {
            "chart": {
                "spec": {
                    "chart": chart,
                    "version": "1.0.0",
                    "sourceRef": {"kind": "HelmRepository", "name": "repo"},
                }
            }
        },
    }


def cosign(subject: str = SUBJECT) -> dict:
    return {"provider": "cosign", "matchOIDCIdentity": [{"issuer": ISSUER, "subject": subject}]}


def good_objects() -> list[dict]:
    return [
        oci_repository("signed", "oci://ghcr.io/acme/charts/signed", cosign()),
        chart_ref_release("signed", "signed"),
        oci_repository("plain", "oci://ghcr.io/acme/charts/plain"),
        chart_ref_release("plain", "plain"),
        helm_repository_release("index", "index"),
    ]


class ChartSourcesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.table = ccs.parse_table(README)

    def test_matching_sources_pass(self) -> None:
        self.assertEqual(ccs.check(good_objects(), self.table), [])

    def test_helm_repository_for_an_oci_chart_fails(self) -> None:
        objects = good_objects()
        objects[3] = helm_repository_release("plain", "plain")
        errors = ccs.check(objects, self.table)
        self.assertEqual(len(errors), 1)
        self.assertIn("plain is published as OCI, use spec.chartRef", errors[0])

    def test_signed_chart_without_verify_fails(self) -> None:
        objects = good_objects()
        objects[0] = oci_repository("signed", "oci://ghcr.io/acme/charts/signed")
        self.assertIn("has no cosign verify", ccs.check(objects, self.table)[0])

    def test_signed_chart_with_another_identity_fails(self) -> None:
        objects = good_objects()
        objects[0] = oci_repository(
            "signed", "oci://ghcr.io/acme/charts/signed", cosign("^https://github\\.com/evil/.*$")
        )
        self.assertIn("is not the README identity", ccs.check(objects, self.table)[0])

    def test_verify_on_an_unsigned_chart_fails(self) -> None:
        objects = good_objects()
        objects[2] = oci_repository("plain", "oci://ghcr.io/acme/charts/plain", cosign())
        self.assertIn("marks plain unverified", ccs.check(objects, self.table)[0])

    def test_oci_repository_named_apart_from_its_chart_fails(self) -> None:
        objects = good_objects()
        objects[2] = oci_repository("other", "oci://ghcr.io/acme/charts/plain")
        objects[3] = chart_ref_release("plain", "other")
        self.assertIn("must be named after its chart plain", ccs.check(objects, self.table)[0])

    def test_url_other_than_the_table_fails(self) -> None:
        objects = good_objects()
        objects[2] = oci_repository("plain", "oci://docker.io/acme/plain")
        self.assertIn("url is not the README url", ccs.check(objects, self.table)[0])

    def test_chart_without_a_row_and_stale_rows_fail(self) -> None:
        objects = [*good_objects()[:4], helm_repository_release("new", "new")]
        errors = ccs.check(objects, self.table)
        self.assertIn("chart new has no row", errors[0])
        self.assertIn("row index matches no HelmRelease", errors[1])

    def test_missing_oci_repository_fails(self) -> None:
        errors = ccs.check(good_objects()[1:], self.table)
        self.assertIn("missing OCIRepository demo/signed", errors[0])

    def test_helm_repository_row_needs_a_reason(self) -> None:
        table = ccs.parse_table(README.replace("No OCI artifact.", ""))
        self.assertIn("gives no reason", ccs.check(good_objects(), table)[0])

    def test_cosign_row_needs_issuer_and_subject(self) -> None:
        with self.assertRaisesRegex(ccs.TableError, "issuer and subject"):
            ccs.parse_table(README.replace(f"`{ISSUER}` ", ""))

    def test_missing_section_fails(self) -> None:
        with self.assertRaisesRegex(ccs.TableError, "no '## Chart sources' section"):
            ccs.parse_table("# Bases\n")


class RepositoryTableTest(unittest.TestCase):
    def test_platform_readme_table_parses(self) -> None:
        table = ccs.parse_table(ccs.README.read_text())
        self.assertIn("kyverno", table)
        self.assertEqual(table["flagger"].verify, ccs.COSIGN)

    def test_main_reports_errors_from_a_render(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "base-demo.yaml").write_text(
                "apiVersion: helm.toolkit.fluxcd.io/v2\nkind: HelmRelease\nmetadata:\n"
                "  name: plain\n  namespace: demo\nspec:\n  chart:\n    spec:\n"
                "      chart: plain\n      version: 1.0.0\n"
            )
            readme = root / "README.md"
            readme.write_text(README)
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(io.StringIO()):
                code = ccs.main(["--rendered", str(root), "--readme", str(readme)])
        self.assertEqual(code, 1)
        self.assertIn("plain is published as OCI", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import base64
import tempfile
import unittest
from pathlib import Path

import check_alertmanager_config as cac

CONFIG = """route:
  receiver: tickets
  routes:
    - receiver: heartbeat
      matchers:
        - alertname = "Watchdog"
    - receiver: blackhole
      matchers:
        - alertname = "InfoInhibitor"
    - receiver: pager
      matchers:
        - severity = "critical"
inhibit_rules:
  - source_matchers:
      - severity = "critical"
    target_matchers:
      - severity =~ "warning|info"
    equal: [namespace, alertname]
  - source_matchers:
      - alertname = "InfoInhibitor"
    target_matchers:
      - severity = "info"
    equal: [namespace]
receivers:
  - name: heartbeat
    webhook_configs:
      - url: https://heartbeat.test/ping
  - name: blackhole
  - name: pager
    webhook_configs:
      - url: https://pager.test/
  - name: tickets
    webhook_configs:
      - url: http://alert-sink.observability.svc.cluster.local:8080/alerts
"""

CHART = """apiVersion: monitoring.coreos.com/v1
kind: Alertmanager
metadata:
  name: stack-alertmanager
  namespace: observability
spec:
  configSecret: {config_secret}
  secrets: {secrets}
"""

BASE = """apiVersion: notification.toolkit.fluxcd.io/v1
kind: Alert
metadata:
  name: flux-errors
  namespace: flux-system
spec:
  eventSeverity: error
  eventSources:
    - kind: Kustomization
      name: "*"
    - kind: HelmRelease
      name: "*"
      namespace: observability
---
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: kube-prometheus-stack
  namespace: observability
"""

OTHER_RELEASE = """apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: traefik
  namespace: traefik
"""

POSTHOG_REPOSITORY = """apiVersion: source.toolkit.fluxcd.io/v1
kind: HelmRepository
metadata:
  name: posthog
  namespace: posthog
"""

TENANT_RELEASE = """apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: app
  namespace: acme
"""

TENANT_REPOSITORY = """apiVersion: source.toolkit.fluxcd.io/v1
kind: OCIRepository
metadata:
  name: app
  namespace: acme
"""


def tenant_alert(namespace: str) -> str:
    return f"""apiVersion: notification.toolkit.fluxcd.io/v1
kind: Alert
metadata:
  name: errors
  namespace: {namespace}
spec:
  eventSeverity: error
  eventSources:
    - kind: HelmRelease
      name: "*"
"""


SINK_SERVICE = """apiVersion: v1
kind: Service
metadata:
  name: alert-sink
  namespace: observability
"""


def secret(config: str, key: str = "alertmanager.yaml", service: str = SINK_SERVICE) -> str:
    encoded = base64.b64encode(config.encode()).decode()
    return (
        f"""apiVersion: v1
kind: Secret
metadata:
  name: alertmanager-routing
  namespace: observability
data:
  {key}: {encoded}
---
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: local-observability
  namespace: flux-system
---
"""
        + service
    )


class Case:
    def __init__(self, test: unittest.TestCase) -> None:
        directory = tempfile.TemporaryDirectory()
        test.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.rendered = self.root / "rendered"
        self.rendered.mkdir()
        self.example = self.root / "example.yaml"
        self.example.write_text(CONFIG)
        self.chart(cac.ROUTING_NAME, f"[{cac.ROUTING_NAME}]")
        self.base(BASE)
        self.overlays = {"local": secret(CONFIG)}

    def chart(self, config_secret: str, secrets: str) -> None:
        text = CHART.format(config_secret=config_secret, secrets=secrets)
        (self.rendered / cac.CHART_RENDER).write_text(text)

    def base(self, text: str) -> None:
        (self.rendered / cac.BASE_RENDER).write_text(text)

    def problems(self) -> list[str]:
        return cac.check(self.rendered, self.example, self.overlays, ["local"])


class AlertmanagerConfigTest(unittest.TestCase):
    def assert_one_problem(self, case: Case, expected: str) -> None:
        problems = case.problems()
        self.assertEqual(len(problems), 1, problems)
        self.assertIn(expected, problems[0])

    def test_complete_routing_passes(self) -> None:
        self.assertEqual(Case(self).problems(), [])

    def test_route_to_receiver_without_integration_fails(self) -> None:
        case = Case(self)
        case.example.write_text(CONFIG.replace("receiver: pager", "receiver: blackhole"))
        self.assert_one_problem(case, "routes[2] sends alerts to receiver 'blackhole'")

    def test_root_route_to_null_fails(self) -> None:
        case = Case(self)
        case.overlays["local"] = secret(
            CONFIG.replace("receiver: tickets", "receiver: blackhole", 1)
        )
        self.assert_one_problem(case, "local Secret alertmanager-routing: route sends alerts")

    def test_missing_watchdog_route_fails(self) -> None:
        case = Case(self)
        case.example.write_text(CONFIG.replace('alertname = "Watchdog"', 'alertname = "Other"'))
        self.assert_one_problem(case, 'route.routes[0] must match alertname = "Watchdog"')

    def test_watchdog_to_null_fails(self) -> None:
        case = Case(self)
        case.example.write_text(CONFIG.replace("receiver: heartbeat", "receiver: blackhole"))
        problems = case.problems()
        self.assertTrue(
            any("Watchdog goes to 'blackhole'" in problem for problem in problems), problems
        )

    def test_missing_severity_inhibit_rule_fails(self) -> None:
        case = Case(self)
        case.example.write_text(
            CONFIG.replace("equal: [namespace, alertname]", "equal: [namespace]")
        )
        self.assert_one_problem(case, "critical inhibits warning")

    def test_missing_info_inhibitor_rule_fails(self) -> None:
        case = Case(self)
        case.example.write_text(
            CONFIG.replace(
                '- alertname = "InfoInhibitor"\n    target', '- alertname = "X"\n    target'
            )
        )
        self.assert_one_problem(case, "InfoInhibitor inhibits info")

    def test_alertmanager_without_secret_mount_fails(self) -> None:
        case = Case(self)
        case.chart(cac.ROUTING_NAME, "[]")
        self.assert_one_problem(case, "does not mount Secret alertmanager-routing")

    def test_alertmanager_with_other_config_secret_fails(self) -> None:
        case = Case(self)
        case.chart("other", f"[{cac.ROUTING_NAME}]")
        self.assert_one_problem(case, "configSecret is not alertmanager-routing")

    def test_base_shipping_the_secret_fails(self) -> None:
        case = Case(self)
        case.base(BASE + "---\n" + secret(CONFIG))
        self.assert_one_problem(case, "base-observability-stack.yaml: renders Secret")

    def test_required_overlay_without_secret_fails(self) -> None:
        case = Case(self)
        case.overlays["local"] = OTHER_RELEASE.replace("traefik", "observability")
        self.assert_one_problem(case, "local: has no Secret observability/alertmanager-routing")

    def test_secret_without_config_key_fails(self) -> None:
        case = Case(self)
        case.overlays["local"] = secret(CONFIG, key="config.yaml")
        self.assert_one_problem(case, "has no alertmanager.yaml key")

    def test_cluster_url_without_rendered_service_fails(self) -> None:
        case = Case(self)
        case.overlays["local"] = secret(CONFIG.replace("observability.svc", "e2e-receiver.svc"))
        self.assert_one_problem(case, "renders no Service e2e-receiver/alert-sink")

    def test_cluster_url_with_service_missing_from_overlay_fails(self) -> None:
        case = Case(self)
        case.overlays["local"] = secret(CONFIG, service="kind: ConfigMap\n")
        self.assert_one_problem(case, "receiver 'tickets' posts to http://alert-sink")

    def test_optional_overlay_without_secret_passes(self) -> None:
        case = Case(self)
        case.overlays["everything"] = BASE.split("---\n")[1]
        self.assertEqual(case.problems(), [])

    def test_flux_namespace_without_event_source_fails(self) -> None:
        case = Case(self)
        (case.rendered / "base-traefik.yaml").write_text(OTHER_RELEASE)
        self.assert_one_problem(case, "no eventSource for HelmRelease in namespace traefik")

    def test_source_namespace_without_event_source_fails(self) -> None:
        case = Case(self)
        (case.rendered / "base-posthog.yaml").write_text(POSTHOG_REPOSITORY)
        self.assert_one_problem(case, "no eventSource for HelmRepository in namespace posthog")

    def test_tenant_alert_in_own_namespace_passes(self) -> None:
        case = Case(self)
        case.overlays["tenant"] = TENANT_RELEASE + "---\n" + tenant_alert("acme")
        self.assertEqual(case.problems(), [])

    def test_tenant_alert_does_not_cover_other_kind(self) -> None:
        case = Case(self)
        case.overlays["tenant"] = (
            TENANT_RELEASE + "---\n" + TENANT_REPOSITORY + "---\n" + tenant_alert("acme")
        )
        self.assert_one_problem(case, "no eventSource for OCIRepository in namespace acme")

    def test_alert_in_other_namespace_does_not_cover_tenant(self) -> None:
        case = Case(self)
        case.overlays["tenant"] = TENANT_RELEASE + "---\n" + tenant_alert("other")
        self.assert_one_problem(case, "no eventSource for HelmRelease in namespace acme")

    def test_committed_example_passes(self) -> None:
        config = cac.parse_config(cac.EXAMPLE.read_text())
        self.assertEqual(cac.config_problems(config, "example"), [])


if __name__ == "__main__":
    unittest.main()

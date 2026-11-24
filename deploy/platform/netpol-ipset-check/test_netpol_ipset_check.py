"""Tests check.py against hand-built pods and ipsets, and the CronJob that runs it."""

import datetime
import importlib.util
import pathlib
import re
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("netpol_ipset_check", HERE / "check.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

NOW = datetime.datetime(2026, 11, 24, 3, 0, tzinfo=datetime.UTC)
OLD = "2026-11-24T01:00:00Z"
ZK_DST = "KUBE-DST-LJVESSO3SGJF4H5I"


def pod(name, ip, labels, **overrides):
    fields = {"namespace": "posthog", "started": OLD, "phase": "Running", "host_network": False}
    fields.update(overrides)
    return {
        "metadata": {"name": name, "namespace": fields["namespace"], "labels": labels},
        "spec": {"hostNetwork": fields["host_network"]},
        "status": {"podIP": ip, "phase": fields["phase"], "startTime": fields["started"]},
    }


def policy(name, selector, types=None, namespace="posthog"):
    spec = {"podSelector": selector}
    if types is not None:
        spec["policyTypes"] = types
    return {"metadata": {"name": name, "namespace": namespace}, "spec": spec}


ZOOKEEPER = pod(
    "posthog-zookeeper-0", "10.42.0.56", {"app.kubernetes.io/name": "posthog-zookeeper"}
)
ALLOW_ZK = policy(
    "allow-zookeeper", {"matchLabels": {"app.kubernetes.io/name": "posthog-zookeeper"}}, ["Ingress"]
)


class SetNameTest(unittest.TestCase):
    def test_matches_the_set_kube_router_built_on_a_live_cluster(self):
        self.assertEqual(check.set_name("KUBE-DST-", "posthog", "allow-zookeeper"), ZK_DST)

    def test_policy_types_pick_dst_and_src(self):
        self.assertEqual(
            [s[:9] for s in check.policy_sets(policy("p", {}, ["Ingress", "Egress"]))],
            ["KUBE-DST-", "KUBE-SRC-"],
        )
        self.assertEqual(
            [s[:9] for s in check.policy_sets(policy("p", {}, ["Egress"]))], ["KUBE-SRC-"]
        )
        self.assertEqual([s[:9] for s in check.policy_sets(policy("p", {}))], ["KUBE-DST-"])


class FindMissingTest(unittest.TestCase):
    def test_reports_a_pod_left_out_of_its_policy_set(self):
        missing = check.find_missing([ZOOKEEPER], [ALLOW_ZK], {ZK_DST: set()}, NOW)
        self.assertEqual(
            missing, [("posthog", "posthog-zookeeper-0", "10.42.0.56", "allow-zookeeper", ZK_DST)]
        )

    def test_reports_a_missing_set(self):
        self.assertEqual(len(check.find_missing([ZOOKEEPER], [ALLOW_ZK], {}, NOW)), 1)

    def test_passes_when_the_pod_is_in_the_set(self):
        self.assertEqual(
            check.find_missing([ZOOKEEPER], [ALLOW_ZK], {ZK_DST: {"10.42.0.56"}}, NOW), []
        )

    def test_skips_pods_kube_router_ignores_or_has_not_caught_up_with(self):
        labels = {"app.kubernetes.io/name": "posthog-zookeeper"}
        skipped = [
            pod("young", "10.42.0.60", labels, started="2026-11-24T02:55:00Z"),
            pod("done", "10.42.0.61", labels, phase="Succeeded"),
            pod("host", "10.89.1.7", labels, host_network=True),
            pod("pending", "", labels, phase="Pending"),
            pod("other-ns", "10.42.0.62", labels, namespace="kafka"),
            pod("unselected", "10.42.0.63", {"app.kubernetes.io/name": "posthog-kafka"}),
        ]
        self.assertEqual(check.find_missing(skipped, [ALLOW_ZK], {ZK_DST: set()}, NOW), [])

    def test_match_expressions(self):
        web = pod("web", "10.42.0.70", {"role": "web"})
        worker = pod("worker", "10.42.0.71", {"role": "worker", "canary": "1"})
        selectors = {
            "In": {"key": "role", "operator": "In", "values": ["events", "web"]},
            "NotIn": {"key": "role", "operator": "NotIn", "values": ["web"]},
            "Exists": {"key": "canary", "operator": "Exists"},
            "DoesNotExist": {"key": "canary", "operator": "DoesNotExist"},
        }
        expected = {
            "In": ["web"],
            "NotIn": ["worker"],
            "Exists": ["worker"],
            "DoesNotExist": ["web"],
        }
        for op, expr in selectors.items():
            found = check.find_missing(
                [web, worker], [policy("p", {"matchExpressions": [expr]})], {}, NOW
            )
            self.assertEqual([m[1] for m in found], expected[op], op)


class ParseTest(unittest.TestCase):
    def test_parses_ipset_save_output(self):
        text = (
            f"create {ZK_DST} hash:ip family inet hashsize 1024 maxelem 65536 timeout 0\n"
            f"add {ZK_DST} 10.42.0.56 timeout 0\n"
            "create KUBE-SRC-EMPTY hash:ip family inet\n"
        )
        self.assertEqual(
            check.parse_ipset_save(text), {ZK_DST: {"10.42.0.56"}, "KUBE-SRC-EMPTY": set()}
        )


class ManifestTest(unittest.TestCase):
    cronjob = (HERE / "cronjob.yaml").read_text()

    def test_only_the_init_container_gets_net_admin(self):
        self.assertIn("hostNetwork: true", self.cronjob)
        self.assertEqual(self.cronjob.count("add: [NET_ADMIN]"), 1)
        self.assertEqual(self.cronjob.count("drop: [ALL]"), 2)
        init, main = self.cronjob.split("containers:\n            - name: check")
        self.assertIn("add: [NET_ADMIN]", init)
        self.assertIn("runAsNonRoot: true", main)
        self.assertIn(check.IPSET_SAVE, init)

    def test_init_image_is_the_platform_k3s_image(self):
        script = (HERE.parent / "local-git-http.sh").read_text()
        image = re.search(r"printf '%s\\n' (rancher/k3s:\S+)", script).group(1)
        self.assertIn(f"image: {image}\n", self.cronjob)


if __name__ == "__main__":
    unittest.main()

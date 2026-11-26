import dataclasses
import json
import subprocess
import unittest
from unittest import mock

import netpol_matrix
import probe

POD = "netpol-probe-1"
POD_IP = "10.42.0.9"
RULES = (
    f'-A KUBE-ROUTER-FORWARD -s {POD_IP}/32 -m comment --comment "ns/{POD} out" -j KUBE-POD-FW-X\n'
    f'-A KUBE-POD-FW-X -m comment --comment "ns/{POD} reject" -j REJECT\n'
)
POLICY_JUMP = '-A KUBE-POD-FW-X -m comment --comment "ns/allow-x policy" -j KUBE-NWPLCY-Y\n'
DEFAULT_EGRESS = '-A KUBE-POD-FW-X -m comment --comment "default egress" -j KUBE-NWPLCY-DEFAULT\n'
ALLOW_X = {"metadata": {"name": "allow-x"}, "spec": {"podSelector": {"matchLabels": {"app": "x"}}}}
TERMINATING = {"metadata": {"namespace": "other", "name": "old"}, "status": {"phase": "Running"}}


def row(expect="deny", destination="  to: {service: svc.dest, port: 80}\n"):
    text = "- from: {namespace: ns, labels: {app: x}}\n" + destination + f"  expect: {expect}\n"
    return netpol_matrix.parse(text)[0]


class FakeRunner:
    """Answers kubectl and docker like a one-node cluster; nc and nslookup results are scripted."""

    def __init__(self, connects=(0,), sentinel=(0,), lookup=0, endpoints=True, rules=RULES):
        self.sharers = []
        self.policies = []
        self.connects = list(connects)
        self.sentinel = list(sentinel)
        self.lookup = lookup
        self.endpoints = endpoints
        self.rules = rules
        self.commands = []

    def __call__(self, command, **_):
        self.commands.append(command)
        if command[0] == "docker":
            rules = self.rules.pop(0) if isinstance(self.rules, list) else self.rules
            return self.done(rules)
        args = command[1:]
        if args[0] == "exec":
            return self.done(self.exec(args[-1]))
        if args[:3] == ["get", "pods", "-A"]:
            sharers = self.sharers.pop(0) if self.sharers else []
            return self.done(json.dumps({"items": sharers}))
        answer = self.objects().get(tuple(args[:2]), "")
        return self.done(json.dumps(answer) if answer else "")

    def objects(self):
        addresses = [{"type": "InternalIP", "address": "10.89.100.2"}]
        slice_ = {
            "ports": [{"name": "http"}],
            "endpoints": [{"conditions": {"ready": self.endpoints}}],
        }
        return {
            ("get", "pod"): {"spec": {"nodeName": "k3d-node"}, "status": {"podIP": POD_IP}},
            ("get", "node"): {"status": {"addresses": addresses}},
            ("get", "service"): {"spec": {"ports": [{"name": "http", "port": 80}]}},
            ("get", "endpointslices"): {"items": [slice_]},
            ("get", "networkpolicies"): {"items": self.policies},
        }

    def exec(self, script):
        if script.startswith(f"nslookup {probe.SENTINEL_NAME}"):
            code = self.sentinel.pop(0) if len(self.sentinel) > 1 else self.sentinel[0]
            return f"{probe.RC_MARKER}{code}\n"
        if script.startswith("nslookup"):
            return f"server can't find\n{probe.RC_MARKER}{self.lookup}\n"
        code = self.connects.pop(0) if len(self.connects) > 1 else self.connects[0]
        message = "" if code == 0 else "nc: svc (10.43.0.5:80): Operation timed out\n"
        return f"{message}{probe.RC_MARKER}{code}\n"

    @staticmethod
    def done(stdout):
        return subprocess.CompletedProcess([], 0, stdout, "")

    def ran(self, word):
        return [command for command in self.commands if word in " ".join(command)]


@mock.patch.object(probe.time, "sleep", lambda _: None)
class RunRowTests(unittest.TestCase):
    def run_row(self, runner, test_row):
        return probe.run_row(probe.Cluster(runner), test_row)

    def test_a_deny_row_passes_when_the_connection_times_out(self):
        outcome = self.run_row(FakeRunner(connects=[1]), row("deny"))
        self.assertEqual((outcome.status, outcome.detail), ("PASS", "deny (timed out)"))

    def test_a_deny_row_fails_when_the_connection_succeeds(self):
        outcome = self.run_row(FakeRunner(connects=[0]), row("deny"))
        self.assertEqual(outcome.status, "FAIL")
        self.assertIn("expected deny, got allow", outcome.detail)

    def test_an_allow_row_retries_before_it_fails(self):
        runner = FakeRunner(connects=[1, 1, 0])
        outcome = self.run_row(runner, row("allow"))
        self.assertEqual(
            (outcome.status, outcome.detail), ("PASS", "allow (connected, 3 attempts)")
        )

    def test_a_deny_row_gets_one_attempt(self):
        runner = FakeRunner(connects=[1, 0])
        self.assertEqual(self.run_row(runner, row("deny")).status, "PASS")
        self.assertEqual(len(runner.ran(" nc ")), 1)

    def test_no_ready_endpoint_is_an_error_not_a_deny(self):
        outcome = self.run_row(FakeRunner(connects=[1], endpoints=False), row("deny"))
        self.assertEqual(outcome.status, "ERROR")
        self.assertIn("no ready endpoint for port 80", outcome.detail)

    def test_a_failed_dns_lookup_is_an_error_not_a_deny(self):
        outcome = self.run_row(FakeRunner(connects=[1], lookup=1), row("deny"))
        self.assertEqual(outcome.status, "ERROR")
        self.assertIn("DNS lookup of svc.dest.svc.cluster.local failed", outcome.detail)

    def test_it_waits_for_the_known_allow_before_probing(self):
        runner = FakeRunner(connects=[1], sentinel=[1, 1, 0])
        self.assertEqual(self.run_row(runner, row("deny")).status, "PASS")
        self.assertEqual(len(runner.ran(f"nslookup {probe.SENTINEL_NAME}")), 3)

    def test_a_known_allow_that_never_works_is_an_error(self):
        runner = FakeRunner(connects=[1], sentinel=[1])
        with mock.patch.object(probe, "SENTINEL_TIMEOUT_SECONDS", -1):
            outcome = self.run_row(runner, row("deny"))
        self.assertEqual(outcome.status, "ERROR")
        self.assertIn("known-allow DNS lookup", outcome.detail)
        self.assertEqual(runner.ran(" nc "), [])

    def test_an_unfenced_pod_is_never_probed(self):
        runner = FakeRunner(connects=[1], rules="")
        with mock.patch.object(probe, "FENCE_TIMEOUT_SECONDS", -1):
            outcome = self.run_row(runner, row("deny"))
        self.assertEqual(outcome.status, "ERROR")
        self.assertIn("no firewall chain", outcome.detail)
        self.assertEqual(runner.ran(" nc "), [])

    def test_it_waits_while_a_terminating_pod_holds_the_probe_ip(self):
        runner = FakeRunner(connects=[1])
        runner.sharers = [[TERMINATING], [TERMINATING], []]
        self.assertEqual(self.run_row(runner, row("deny")).status, "PASS")
        self.assertEqual(len(runner.ran("status.podIP=")), 3)

    def test_a_shared_ip_that_never_frees_is_an_error(self):
        runner = FakeRunner(connects=[0])
        runner.sharers = [[TERMINATING]] * 3
        with mock.patch.object(probe, "FENCE_TIMEOUT_SECONDS", -1):
            outcome = self.run_row(runner, row("deny"))
        self.assertEqual(outcome.status, "ERROR")
        self.assertIn("still holds the probe pod's IP", outcome.detail)
        self.assertEqual(runner.ran(" nc "), [])

    def test_a_finished_pod_on_the_same_ip_does_not_block(self):
        finished = {**TERMINATING, "status": {"phase": "Succeeded"}}
        runner = FakeRunner(connects=[1])
        runner.sharers = [[finished]]
        self.assertEqual(self.run_row(runner, row("deny")).status, "PASS")

    def test_a_chain_lost_during_the_probe_is_an_error_not_a_verdict(self):
        runner = FakeRunner(connects=[0], rules=[RULES, ""])
        outcome = self.run_row(runner, row("deny"))
        self.assertEqual(outcome.status, "ERROR")
        self.assertIn("dropped the probe pod's firewall chain", outcome.detail)

    def test_it_waits_until_the_chain_jumps_to_the_selecting_policy(self):
        dumps = [RULES + DEFAULT_EGRESS] + [RULES + POLICY_JUMP] * 2
        runner = FakeRunner(connects=[1], rules=dumps)
        runner.policies = [ALLOW_X]
        with mock.patch.object(probe, "POLL_SECONDS", 0):
            outcome = self.run_row(runner, row("deny", "  to: {host: node, port: 10250}\n"))
        self.assertEqual(outcome.status, "PASS")
        self.assertEqual(len(runner.ran("iptables-save")), 3)

    def test_a_chain_that_never_reaches_the_selecting_policy_is_never_probed(self):
        runner = FakeRunner(connects=[0], rules=RULES + DEFAULT_EGRESS)
        runner.policies = [ALLOW_X]
        with mock.patch.object(probe, "FENCE_TIMEOUT_SECONDS", -1):
            outcome = self.run_row(runner, row("deny"))
        self.assertEqual(outcome.status, "ERROR")
        self.assertIn("no firewall chain", outcome.detail)
        self.assertEqual(runner.ran(" nc "), [])

    def test_node_host_resolves_to_the_probe_node(self):
        runner = FakeRunner(connects=[0])
        outcome = self.run_row(runner, row("allow", "  to: {host: node, port: 6443}\n"))
        self.assertEqual(outcome.status, "PASS")
        self.assertTrue(runner.ran("10.89.100.2 6443"))

    def test_the_pod_prefix_names_the_probe_pod(self):
        runner = FakeRunner(connects=[1], rules=RULES.replace(POD, "other-1"))
        outcome = probe.run_row(probe.Cluster(runner), row("deny"), "other")
        self.assertEqual(outcome.status, "PASS")
        self.assertTrue(runner.ran("pod/other-1"))

    def test_the_probe_pod_is_deleted_after_an_error(self):
        runner = FakeRunner(endpoints=False)
        self.run_row(runner, row("deny"))
        self.assertIn("--wait=false", runner.commands[-1])


class SkipTests(unittest.TestCase):
    def test_a_row_is_skipped_when_any_of_its_namespaces_is_absent(self):
        rows = [
            row(),
            row(destination="  to: {service: kured.kured, port: 8080}\n"),
            row(destination="  to: {pod: {namespace: kured, labels: {a: b}}, port: 80}\n"),
        ]
        rows = [dataclasses.replace(item, number=number) for number, item in enumerate(rows, 1)]
        skipped = probe.skipped_outcomes(rows, {"kured"})
        self.assertEqual([item.row.number for item in skipped], [2, 3])
        self.assertEqual({item.status for item in skipped}, {"SKIP"})
        self.assertTrue(skipped[0].detail.endswith("namespace kured absent"))

    def test_absent_namespaces_compares_against_the_cluster(self):
        namespaces = {"items": [{"metadata": {"name": "ns"}}]}
        answer = subprocess.CompletedProcess([], 0, json.dumps(namespaces), "")
        absent = probe.absent_namespaces(probe.Cluster(lambda *_, **__: answer), [row()])
        self.assertEqual(absent, {"dest"})

    def test_the_summary_counts_skips(self):
        outcome = probe.Outcome(row(), "SKIP", "namespace dest absent")
        self.assertIn("1 skipped", probe.summary([outcome], 1))


class HelperTests(unittest.TestCase):
    def test_fenced_needs_the_reject_rule_and_the_egress_jump_from_the_pod_address(self):
        self.assertTrue(probe.fenced(RULES, "ns", POD, POD_IP))
        reject_only = RULES.splitlines()[1]
        self.assertFalse(probe.fenced(reject_only, "ns", POD, POD_IP))
        self.assertFalse(probe.fenced(RULES.replace("-s ", "-d "), "ns", POD, POD_IP))
        self.assertFalse(probe.fenced(RULES, "ns", POD, "10.42.0.10"))
        self.assertFalse(probe.fenced(RULES, "other", POD, POD_IP))
        self.assertFalse(probe.fenced(RULES, "ns", f"{POD}0", POD_IP))

    def test_fenced_needs_a_jump_to_each_selecting_policy_from_the_pod_chain(self):
        policies = ("allow-x",)
        self.assertTrue(probe.fenced(RULES + POLICY_JUMP, "ns", POD, POD_IP, policies))
        self.assertFalse(probe.fenced(RULES + DEFAULT_EGRESS, "ns", POD, POD_IP, policies))
        elsewhere = POLICY_JUMP.replace("KUBE-POD-FW-X", "KUBE-POD-FW-Z")
        self.assertFalse(probe.fenced(RULES + elsewhere, "ns", POD, POD_IP, policies))

    def test_selects_matches_labels_and_expressions(self):
        labels = {"app": "x", "tier": "web"}
        self.assertTrue(probe.selects({}, labels))
        self.assertTrue(probe.selects({"matchLabels": {"app": "x"}}, labels))
        self.assertFalse(probe.selects({"matchLabels": {"app": "y"}}, labels))
        expressions = [
            ({"key": "tier", "operator": "In", "values": ["web"]}, True),
            ({"key": "tier", "operator": "NotIn", "values": ["web"]}, False),
            ({"key": "app", "operator": "Exists"}, True),
            ({"key": "app", "operator": "DoesNotExist"}, False),
        ]
        for expression, expected in expressions:
            self.assertEqual(probe.selects({"matchExpressions": [expression]}, labels), expected)

    def test_classify(self):
        self.assertEqual(probe.classify(0, ""), ("allow", "connected"))
        self.assertEqual(probe.classify(1, "nc: x: Operation timed out"), ("deny", "timed out"))
        self.assertEqual(probe.classify(1, ""), ("deny", "refused"))
        with self.assertRaises(probe.ProbeError):
            probe.classify(1, "nc: bad address 'x'")

    def test_the_probe_pod_is_restricted_and_never_an_endpoint(self):
        manifest = probe.pod_manifest(row(), POD)
        container = manifest["spec"]["containers"][0]
        self.assertEqual(manifest["metadata"]["labels"], {"app": "x", probe.PROBE_LABEL: "true"})
        self.assertTrue(manifest["spec"]["securityContext"]["runAsNonRoot"])
        self.assertEqual(container["securityContext"]["capabilities"], {"drop": ["ALL"]})
        self.assertFalse(container["securityContext"]["allowPrivilegeEscalation"])
        self.assertEqual(container["readinessProbe"]["exec"]["command"], ["false"])

    def test_a_dump_torn_by_a_set_change_reads_as_not_fenced_yet(self):
        torn = "iptables-save v1.8.11 (nf_tables): Set with index 248 in kernel doesn't exist."
        results = iter([(1, "", torn), (0, RULES, "")])
        cluster = probe.Cluster(lambda *_, **__: subprocess.CompletedProcess([], *next(results)))
        with mock.patch.object(probe.time, "monotonic", side_effect=[10.0, 10.0, 12.0, 12.0]):
            self.assertEqual(cluster.node_rules("k3d-node"), "")
            self.assertEqual(cluster.node_rules("k3d-node"), RULES)

    def test_any_other_dump_failure_is_an_error(self):
        cluster = probe.Cluster(
            lambda *_, **__: subprocess.CompletedProcess([], 1, "", "permission denied")
        )
        with self.assertRaises(probe.ProbeError):
            cluster.node_rules("k3d-node")

    def test_exec_without_an_exit_status_is_an_error(self):
        cluster = probe.Cluster(lambda *_, **__: subprocess.CompletedProcess([], 0, "garbage", ""))
        with self.assertRaises(probe.ProbeError):
            cluster.exec("ns", POD, "true")


if __name__ == "__main__":
    unittest.main()

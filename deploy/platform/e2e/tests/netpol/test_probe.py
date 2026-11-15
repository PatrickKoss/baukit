import json
import subprocess
import unittest
from unittest import mock

import netpol_matrix
import probe

POD = "netpol-probe-1"
POD_IP = "10.42.0.9"
RULES = f'-A KUBE-POD-FW-X -d {POD_IP}/32 -m comment --comment "default ingress"\n' + (
    f'-A KUBE-POD-FW-X -m comment --comment "ns/{POD} reject" -j REJECT\n'
)


def row(expect="deny", destination="  to: {service: svc.dest, port: 80}\n"):
    text = "- from: {namespace: ns, labels: {app: x}}\n" + destination + f"  expect: {expect}\n"
    return netpol_matrix.parse(text)[0]


class FakeRunner:
    """Answers kubectl and docker like a one-node cluster; nc and nslookup results are scripted."""

    def __init__(self, connects=(0,), sentinel=(0,), lookup=0, endpoints=True, rules=RULES):
        self.connects = list(connects)
        self.sentinel = list(sentinel)
        self.lookup = lookup
        self.endpoints = endpoints
        self.rules = rules
        self.commands = []

    def __call__(self, command, **_):
        self.commands.append(command)
        if command[0] == "docker":
            return self.done(self.rules)
        args = command[1:]
        if args[0] == "exec":
            return self.done(self.exec(args[-1]))
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

    def test_node_host_resolves_to_the_probe_node(self):
        runner = FakeRunner(connects=[0])
        outcome = self.run_row(runner, row("allow", "  to: {host: node, port: 6443}\n"))
        self.assertEqual(outcome.status, "PASS")
        self.assertTrue(runner.ran("10.89.100.2 6443"))

    def test_the_probe_pod_is_deleted_after_an_error(self):
        runner = FakeRunner(endpoints=False)
        self.run_row(runner, row("deny"))
        self.assertIn("--wait=false", runner.commands[-1])


class HelperTests(unittest.TestCase):
    def test_fenced_needs_the_reject_rule_and_the_pod_address(self):
        self.assertTrue(probe.fenced(RULES, "ns", POD, POD_IP))
        self.assertFalse(probe.fenced(RULES, "ns", POD, "10.42.0.10"))
        self.assertFalse(probe.fenced(RULES, "other", POD, POD_IP))
        self.assertFalse(probe.fenced(RULES, "ns", f"{POD}0", POD_IP))

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

    def test_exec_without_an_exit_status_is_an_error(self):
        cluster = probe.Cluster(lambda *_, **__: subprocess.CompletedProcess([], 0, "garbage", ""))
        with self.assertRaises(probe.ProbeError):
            cluster.exec("ns", POD, "true")


if __name__ == "__main__":
    unittest.main()

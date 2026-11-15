#!/usr/bin/env python3
"""Run every row of the NetworkPolicy matrix as a short-lived probe pod and compare the verdict.

A probe pod starts in the row's source namespace with the row's labels. Before it probes, the runner
waits until kube-router has written the pod's firewall chain on its node and a DNS lookup from the
pod works, so a new pod's unfenced first seconds cannot pass a deny row and its not yet admitted
first seconds cannot fail an allow row. A destination without a ready endpoint, a failed DNS lookup
or a probe that never got that far is an error, never a verdict.
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path

import netpol_matrix

process_runner = netpol_matrix.helm_yaml.process_runner
Row = netpol_matrix.Row
SUITE = Path(__file__).resolve().parent
IMAGE = "localhost/baukit-e2e/echo:local"
PROBE_LABEL = "baukit.dev/netpol-probe"
POD_PREFIX = "netpol-probe"
SENTINEL_NAME = "kubernetes.default.svc.cluster.local"
WORKERS = 16
CONNECT_TIMEOUT_SECONDS = 3
ALLOW_ATTEMPTS = 3
ATTEMPT_GAP_SECONDS = 2
START_TIMEOUT_SECONDS = 180
FENCE_TIMEOUT_SECONDS = 120
SENTINEL_TIMEOUT_SECONDS = 60
POLL_SECONDS = 1
POD_DEADLINE_SECONDS = 900
RC_MARKER = "probe-rc="

Runner = Callable[..., object]


class ProbeError(Exception):
    pass


@dataclass(frozen=True)
class Outcome:
    row: Row
    status: str
    detail: str

    def line(self) -> str:
        return f"{self.status} {self.row.describe()}: {self.detail}"


class Cluster:
    def __init__(self, run: Runner = process_runner.run_process) -> None:
        self.run = run
        self.lock = threading.Lock()
        self.rules: dict[str, tuple[float, str]] = {}

    def kubectl(self, *args: str, stdin: str | None = None) -> str:
        result = self.run(
            ["kubectl", *args], input=stdin, capture_output=True, text=True, check=False
        )
        if result.returncode != 0:
            raise ProbeError(f"kubectl {' '.join(args)[:120]} failed: {result.stderr.strip()}")
        return result.stdout

    def json(self, *args: str) -> dict:
        return json.loads(self.kubectl(*args, "-o", "json"))

    def exec(self, namespace: str, pod: str, command: str) -> tuple[int, str]:
        output = self.kubectl(
            "exec", "-n", namespace, pod, "--", "sh", "-c", f"{command} 2>&1; echo {RC_MARKER}$?"
        )
        head, marker, code = output.rpartition(RC_MARKER)
        if not marker or not code.strip().isdigit():
            raise ProbeError(f"no exit status from the probe pod: {output.strip()[-200:]}")
        return int(code.strip()), head.strip()

    def node_rules(self, node: str) -> str:
        with self.lock:
            stamp, rules = self.rules.get(node, (0.0, ""))
            if time.monotonic() - stamp < POLL_SECONDS:
                return rules
            result = self.run(
                ["docker", "exec", node, "iptables-save", "-t", "filter"],
                capture_output=True,
                text=True,
                check=False,
            )
            if result.returncode != 0:
                raise ProbeError(f"iptables-save on node {node} failed: {result.stderr.strip()}")
            self.rules[node] = (time.monotonic(), result.stdout)
            return result.stdout


def fenced(rules: str, namespace: str, pod: str, address: str) -> bool:
    """kube-router writes one chain per pod with a reject rule naming it and a match on its IP."""
    reject = f'"{namespace}/{pod} reject"'
    return reject in rules and f"-d {address}/32" in rules


def classify(code: int, output: str) -> tuple[str, str]:
    if code == 0:
        return "allow", "connected"
    if "bad address" in output:
        raise ProbeError(f"the probe could not resolve its target: {output}")
    if "timed out" in output:
        return "deny", "timed out"
    return "deny", "refused" if not output else f"refused ({output})"


def pod_manifest(row: Row, name: str) -> dict:
    labels = {**row.labels, PROBE_LABEL: "true"}
    return {
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": name, "namespace": row.namespace, "labels": labels},
        "spec": {
            "restartPolicy": "Never",
            "activeDeadlineSeconds": POD_DEADLINE_SECONDS,
            "automountServiceAccountToken": False,
            "enableServiceLinks": False,
            "terminationGracePeriodSeconds": 0,
            "securityContext": {
                "runAsNonRoot": True,
                "runAsUser": 101,
                "runAsGroup": 101,
                "seccompProfile": {"type": "RuntimeDefault"},
            },
            "containers": [
                {
                    "name": "probe",
                    "image": IMAGE,
                    "imagePullPolicy": "Never",
                    "command": ["sleep", str(POD_DEADLINE_SECONDS)],
                    "readinessProbe": {"exec": {"command": ["false"]}, "periodSeconds": 300},
                    "resources": {
                        "requests": {"cpu": "10m", "memory": "16Mi"},
                        "limits": {"cpu": "100m", "memory": "32Mi"},
                    },
                    "securityContext": {
                        "allowPrivilegeEscalation": False,
                        "readOnlyRootFilesystem": True,
                        "capabilities": {"drop": ["ALL"]},
                    },
                }
            ],
        },
    }


def ready_pod_ip(cluster: Cluster, namespace: str, labels: dict[str, str]) -> str:
    pods = cluster.json("get", "pods", "-n", namespace, "-l", netpol_matrix.selector(labels))
    for pod in pods.get("items") or []:
        if PROBE_LABEL in (pod["metadata"].get("labels") or {}):
            continue
        conditions = (pod.get("status") or {}).get("conditions") or []
        if any(item["type"] == "Ready" and item["status"] == "True" for item in conditions):
            return pod["status"]["podIP"]
    raise ProbeError(f"no ready pod in {namespace} matches {netpol_matrix.selector(labels)}")


def check_service_endpoints(cluster: Cluster, name: str, namespace: str, port: int) -> None:
    service = cluster.json("get", "service", "-n", namespace, name)
    ports = [item for item in service["spec"].get("ports") or [] if item["port"] == port]
    if not ports:
        raise ProbeError(f"Service {namespace}/{name} has no port {port}")
    port_name = ports[0].get("name")
    slices = cluster.json(
        "get", "endpointslices", "-n", namespace, "-l", f"kubernetes.io/service-name={name}"
    )
    for item in slices.get("items") or []:
        if not any(
            (entry.get("name") or None) == (port_name or None) for entry in item.get("ports") or []
        ):
            continue
        for endpoint in item.get("endpoints") or []:
            if (endpoint.get("conditions") or {}).get("ready"):
                return
    raise ProbeError(f"Service {namespace}/{name} has no ready endpoint for port {port}")


def destination_address(cluster: Cluster, row: Row, node_ip: str) -> str:
    if row.kind == "service":
        name, namespace = row.target.split(".")
        check_service_endpoints(cluster, name, namespace, row.port)
        return f"{name}.{namespace}.svc.cluster.local"
    if row.kind == "pod":
        return ready_pod_ip(cluster, row.pod_namespace, row.pod_labels)
    return node_ip if row.target == netpol_matrix.NODE_HOST else row.target


def wait_until(check: Callable[[], bool], timeout: float, message: str) -> None:
    deadline = time.monotonic() + timeout
    while not check():
        if time.monotonic() > deadline:
            raise ProbeError(message)
        time.sleep(POLL_SECONDS)


class Probe:
    def __init__(self, cluster: Cluster, row: Row, prefix: str = POD_PREFIX) -> None:
        self.cluster = cluster
        self.row = row
        self.name = f"{prefix}-{row.number}"

    def start(self) -> tuple[str, str]:
        namespace = self.row.namespace
        self.cluster.kubectl(
            "delete", "pod", "-n", namespace, self.name, "--ignore-not-found", "--wait=true"
        )
        self.cluster.kubectl(
            "create", "-f", "-", stdin=json.dumps(pod_manifest(self.row, self.name))
        )
        self.cluster.kubectl(
            "wait",
            "-n",
            namespace,
            f"pod/{self.name}",
            "--for=jsonpath={.status.phase}=Running",
            f"--timeout={START_TIMEOUT_SECONDS}s",
        )
        pod = self.cluster.json("get", "pod", "-n", namespace, self.name)
        node = pod["spec"]["nodeName"]
        address = pod["status"]["podIP"]
        wait_until(
            lambda: fenced(self.cluster.node_rules(node), namespace, self.name, address),
            FENCE_TIMEOUT_SECONDS,
            f"kube-router wrote no firewall chain for the probe pod on {node}",
        )
        wait_until(
            lambda: self.cluster.exec(namespace, self.name, f"nslookup {SENTINEL_NAME}")[0] == 0,
            SENTINEL_TIMEOUT_SECONDS,
            f"the known-allow DNS lookup of {SENTINEL_NAME} never worked from the probe pod",
        )
        return node, self.node_ip(node)

    def node_ip(self, node: str) -> str:
        addresses = self.cluster.json("get", "node", node)["status"]["addresses"]
        return next(item["address"] for item in addresses if item["type"] == "InternalIP")

    def resolve(self, address: str) -> None:
        if self.row.kind != "service":
            return
        code, output = self.cluster.exec(self.row.namespace, self.name, f"nslookup {address}")
        if code != 0:
            raise ProbeError(f"DNS lookup of {address} failed: {output[-200:]}")

    def connect(self, address: str) -> tuple[str, str, int]:
        attempts = ALLOW_ATTEMPTS if self.row.expect == "allow" else 1
        command = f"nc -v -z -w {CONNECT_TIMEOUT_SECONDS} {address} {self.row.port}"
        verdict, reason = "deny", ""
        for attempt in range(1, attempts + 1):
            verdict, reason = classify(*self.cluster.exec(self.row.namespace, self.name, command))
            if verdict == "allow":
                return verdict, reason, attempt
            if attempt < attempts:
                time.sleep(ATTEMPT_GAP_SECONDS)
        return verdict, reason, attempts

    def stop(self) -> None:
        self.cluster.run(
            [
                "kubectl",
                "delete",
                "pod",
                "-n",
                self.row.namespace,
                self.name,
                "--wait=false",
                "--ignore-not-found",
            ],
            capture_output=True,
            text=True,
            check=False,
        )


def run_row(cluster: Cluster, row: Row, prefix: str = POD_PREFIX) -> Outcome:
    probe = Probe(cluster, row, prefix)
    try:
        _, node_ip = probe.start()
        address = destination_address(cluster, row, node_ip)
        probe.resolve(address)
        verdict, reason, attempts = probe.connect(address)
    except ProbeError as error:
        return Outcome(row, "ERROR", str(error))
    finally:
        probe.stop()
    tries = f", {attempts} attempts" if attempts > 1 else ""
    if verdict == row.expect:
        return Outcome(row, "PASS", f"{verdict} ({reason}{tries})")
    return Outcome(
        row, "FAIL", f"expected {row.expect}, got {verdict} ({reason}{tries}) to {address}"
    )


def run_matrix(cluster: Cluster, rows: list[Row], workers: int, prefix: str) -> list[Outcome]:
    outcomes = []
    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(run_row, cluster, row, prefix) for row in rows]
        for future in as_completed(futures):
            outcome = future.result()
            outcomes.append(outcome)
            print(outcome.line(), flush=True)
    return sorted(outcomes, key=lambda item: item.row.number)


def summary(outcomes: list[Outcome], seconds: float) -> str:
    counts = {
        status: sum(item.status == status for item in outcomes)
        for status in ("PASS", "FAIL", "ERROR")
    }
    return (
        f"netpol matrix: {len(outcomes)} rows, {counts['PASS']} passed, "
        f"{counts['FAIL']} mismatched, {counts['ERROR']} errors in {seconds:.0f}s"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, default=SUITE / "matrix.yaml")
    parser.add_argument("--rows", type=int, nargs="*", help="run only these row numbers")
    parser.add_argument("--workers", type=int, default=WORKERS)
    parser.add_argument(
        "--pod-prefix",
        default=POD_PREFIX,
        help="probe pod name prefix, unique per concurrent suite",
    )
    args = parser.parse_args(argv)
    rows = netpol_matrix.load(args.matrix)
    if args.rows:
        rows = [row for row in rows if row.number in set(args.rows)]
    started = time.monotonic()
    outcomes = run_matrix(Cluster(), rows, args.workers, args.pod_prefix)
    bad = [item for item in outcomes if item.status != "PASS"]
    if bad:
        print("Rows that did not pass:", file=sys.stderr)
        for item in bad:
            print(f"  {item.line()}", file=sys.stderr)
    print(summary(outcomes, time.monotonic() - started), flush=True)
    return 1 if bad or not outcomes else 0


if __name__ == "__main__":
    sys.exit(main())

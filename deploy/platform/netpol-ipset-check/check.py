"""Exits 1 when a running pod is missing from the kube-router ipset of a policy that selects it.

Set names and pod selection follow kube-router v2.11 (pkg/controllers/netpol/policy.go).
"""

import base64
import datetime
import hashlib
import http
import http.client
import json
import os
import ssl
import sys

SA_DIR = "/var/run/secrets/kubernetes.io/serviceaccount"
IPSET_SAVE = "/work/ipset.save"
GRACE_SECONDS = 600
API_TIMEOUT = 30
FINISHED_PHASES = {"Succeeded", "Failed", "Completed"}


def set_name(prefix, namespace, policy):
    digest = hashlib.sha256((namespace + policy + "IPv4").encode()).digest()
    return prefix + base64.b32encode(digest).decode()[:16]


def label_selector_matches(selector, labels):
    for key, value in (selector.get("matchLabels") or {}).items():
        if labels.get(key) != value:
            return False
    for expr in selector.get("matchExpressions") or []:
        key, op, values = expr["key"], expr["operator"], expr.get("values") or []
        if op == "In" and labels.get(key) not in values:
            return False
        if op == "NotIn" and key in labels and labels[key] in values:
            return False
        if op == "Exists" and key not in labels:
            return False
        if op == "DoesNotExist" and key in labels:
            return False
    return True


def policy_sets(policy):
    meta, spec = policy["metadata"], policy["spec"]
    types = set(spec.get("policyTypes") or [])
    ingress = "Ingress" in types or "Egress" not in types
    sets = []
    if ingress:
        sets.append(set_name("KUBE-DST-", meta["namespace"], meta["name"]))
    if "Egress" in types:
        sets.append(set_name("KUBE-SRC-", meta["namespace"], meta["name"]))
    return sets


def settled_ipv4(pod, now):
    status = pod.get("status", {})
    ip = status.get("podIP", "")
    if (
        pod["spec"].get("hostNetwork")
        or status.get("phase") in FINISHED_PHASES
        or ":" in ip
        or not ip
    ):
        return None
    if pod["metadata"].get("deletionTimestamp"):
        return None
    started = status.get("startTime")
    if not started:
        return None
    start = datetime.datetime.fromisoformat(started.replace("Z", "+00:00"))
    return ip if (now - start).total_seconds() >= GRACE_SECONDS else None


def find_missing(pods, policies, members, now):
    missing = []
    for policy in policies:
        namespace = policy["metadata"]["namespace"]
        selector = policy["spec"].get("podSelector") or {}
        for pod in pods:
            if pod["metadata"]["namespace"] != namespace:
                continue
            if not label_selector_matches(selector, pod["metadata"].get("labels") or {}):
                continue
            ip = settled_ipv4(pod, now)
            if ip is None:
                continue
            for name in policy_sets(policy):
                if ip not in members.get(name, set()):
                    missing.append(
                        (namespace, pod["metadata"]["name"], ip, policy["metadata"]["name"], name)
                    )
    return missing


def parse_ipset_save(text):
    members = {}
    for line in text.splitlines():
        command, *args = line.split() or [""]
        if command == "create" and args:
            members.setdefault(args[0], set())
        elif command == "add" and len(args) > 1:
            members.setdefault(args[0], set()).add(args[1])
    return members


def api_items(resource, token, context):
    host, port = os.environ["KUBERNETES_SERVICE_HOST"], os.environ["KUBERNETES_SERVICE_PORT"]
    connection = http.client.HTTPSConnection(host, int(port), context=context, timeout=API_TIMEOUT)
    try:
        connection.request("GET", resource, headers={"Authorization": "Bearer " + token})
        response = connection.getresponse()
        if response.status != http.HTTPStatus.OK:
            raise RuntimeError(f"GET {resource}: HTTP {response.status}")
        return json.load(response)["items"]
    finally:
        connection.close()


def main():
    with open(SA_DIR + "/token") as handle:
        token = handle.read().strip()
    context = ssl.create_default_context(cafile=SA_DIR + "/ca.crt")
    pods = api_items("/api/v1/pods", token, context)
    policies = api_items("/apis/networking.k8s.io/v1/networkpolicies", token, context)
    with open(IPSET_SAVE) as handle:
        members = parse_ipset_save(handle.read())
    now = datetime.datetime.now(datetime.UTC)
    missing = find_missing(pods, policies, members, now)
    for namespace, pod, ip, policy, name in missing:
        print(f"missing {namespace}/{pod} {ip} from {name} (policy {policy})")
    print(f"checked {len(policies)} policies, {len(pods)} pods, {len(missing)} missing")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())

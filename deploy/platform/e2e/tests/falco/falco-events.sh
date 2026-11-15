#!/usr/bin/env bash
# Prints Falco events at or above a priority since a timestamp, from Falco's own output or from Loki.
set -euo pipefail

usage() {
  printf 'usage: %s falco|loki SINCE_RFC3339 MIN_PRIORITY [RULE] [NAMESPACE] [PROCESS]\n' "${0##*/}" >&2
  exit 2
}

[[ $# -ge 3 && $# -le 6 ]] || usage
source_name=$1 since=$2 min_priority=$3 rule=${4:-} namespace=${5:-} process=${6:-}

falco_lines() {
  local pod
  for pod in $(kubectl -n falco get pods -l app.kubernetes.io/name=falco -o name); do
    kubectl -n falco logs "$pod" -c falco --since-time="$since"
  done
}

loki_lines() {
  local start_ns query
  start_ns=$(($(date -d "$since" +%s) * 1000000000))
  query='{source="syscall"}'
  kubectl get --raw "/api/v1/namespaces/observability/services/loki:3100/proxy/loki/api/v1/query_range?limit=5000&direction=forward&start=$start_ns&query=$(
    python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$query"
  )" |
    python3 -c '
import json, sys
for stream in json.load(sys.stdin)["data"]["result"]:
    for _, line in stream["values"]:
        print(line)
'
}

case $source_name in
  falco) lines=falco_lines ;;
  loki) lines=loki_lines ;;
  *) usage ;;
esac

"$lines" | MIN_PRIORITY=$min_priority RULE=$rule NAMESPACE=$namespace PROCESS=$process python3 -c '
import json, os, sys
order = ["Debug", "Informational", "Notice", "Warning", "Error", "Critical", "Alert", "Emergency"]
floor = order.index(os.environ["MIN_PRIORITY"])
rule, namespace, process = os.environ["RULE"], os.environ["NAMESPACE"], os.environ["PROCESS"]
for line in sys.stdin:
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    if not isinstance(event, dict) or event.get("priority") not in order:
        continue
    if order.index(event["priority"]) < floor:
        continue
    if rule and event.get("rule") != rule:
        continue
    fields = event.get("output_fields") or {}
    if namespace and fields.get("k8s.ns.name") != namespace:
        continue
    if process and fields.get("proc.name") != process:
        continue
    print(json.dumps({"time": event.get("time"), "priority": event["priority"], "rule": event.get("rule"), "output": event.get("output")}))
'

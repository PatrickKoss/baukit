#!/usr/bin/env bash
# Prints Falco events at or above a priority since a timestamp, from Falco's own output or from Loki.
set -euo pipefail

usage() {
  cat >&2 <<USAGE
usage: ${0##*/} --source falco|loki --since RFC3339 --min-priority PRIORITY
       [--rule RULE] [--namespace NAMESPACE] [--process NAME] [--pod NAME] [--exclude-pod PREFIX]... [--kubernetes-only true]
USAGE
  exit 2
}

source_name="" since="" min_priority="" rule="" namespace="" process="" pod="" exclude_pods="" kubernetes_only=""
while (($# > 0)); do
  [[ $# -ge 2 ]] || usage
  case $1 in
    --source) source_name=$2 ;;
    --since) since=$2 ;;
    --min-priority) min_priority=$2 ;;
    --rule) rule=$2 ;;
    --namespace) namespace=$2 ;;
    --process) process=$2 ;;
    --pod) pod=$2 ;;
    --exclude-pod) exclude_pods+="$2"$'\n' ;;
    --kubernetes-only) kubernetes_only=$2 ;;
    *) usage ;;
  esac
  shift 2
done
[[ -n $since && -n $min_priority ]] || usage

falco_lines() {
  local pod
  for pod in $(kubectl -n falco get pods -l app.kubernetes.io/name=falco,pod-template-generation -o name); do
    kubectl -n falco logs "$pod" -c falco --since-time="$since"
  done
}

loki_lines() {
  local start_ns query
  start_ns=$(($(date -d "$since" +%s) * 1000000000))
  # Line filters keep a busy cluster's other events from filling the 5000-line page first.
  query='{source="syscall"}'
  [[ -z $rule ]] || query+=" |= \"$rule\""
  [[ -z $namespace ]] || query+=" |= \"$namespace\""
  [[ -z $pod ]] || query+=" |= \"$pod\""
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

"$lines" | MIN_PRIORITY=$min_priority RULE=$rule NAMESPACE=$namespace PROCESS=$process POD=$pod \
  EXCLUDE_PODS=$exclude_pods KUBERNETES_ONLY=$kubernetes_only python3 -c '
import json, os, sys
order = ["Debug", "Informational", "Notice", "Warning", "Error", "Critical", "Alert", "Emergency"]
floor = order.index(os.environ["MIN_PRIORITY"])
rule, namespace, process = os.environ["RULE"], os.environ["NAMESPACE"], os.environ["PROCESS"]
excluded = tuple(prefix for prefix in os.environ["EXCLUDE_PODS"].splitlines() if prefix)
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
    if os.environ["KUBERNETES_ONLY"] and not fields.get("k8s.ns.name"):
        continue
    if process and fields.get("proc.name") != process:
        continue
    if os.environ["POD"] and fields.get("k8s.pod.name") != os.environ["POD"]:
        continue
    if excluded and str(fields.get("k8s.pod.name")).startswith(excluded):
        continue
    print(json.dumps({"time": event.get("time"), "priority": event["priority"], "rule": event.get("rule"), "output": event.get("output")}))
'

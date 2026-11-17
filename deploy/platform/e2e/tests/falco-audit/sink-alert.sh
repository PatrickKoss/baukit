#!/usr/bin/env bash
# Waits until alert-sink holds a Falco alert for RULE whose labels match every KEY=VALUE, then prints it.
set -euo pipefail

if (($# < 2)); then
  printf 'usage: %s TIMEOUT_SECONDS RULE [LABEL=VALUE]...\n' "${0##*/}" >&2
  exit 2
fi
timeout=$1 rule=$2
shift 2

alertmanager="alertmanager-kube-prometheus-stack-alertmanager-0"
sink=http://alert-sink.observability.svc.cluster.local:8080/alerts
deadline=$(($(date +%s) + timeout))
while :; do
  if kubectl -n observability exec "$alertmanager" -c alertmanager -- wget -qO- "$sink" |
    RULE=$rule python3 -c '
import json, os, sys
wanted = dict(arg.split("=", 1) for arg in sys.argv[1:])
wanted.update(alertname=os.environ["RULE"], eventsource="k8s_audit", source="falco")
for notification in json.load(sys.stdin):
    for alert in notification.get("alerts", []):
        labels = alert["labels"]
        if all(labels.get(key) == value for key, value in wanted.items()):
            print(json.dumps({"receiver": notification["receiver"], "labels": labels}))
            sys.exit(0)
sys.exit(1)
' "$@"; then
    exit 0
  fi
  if (($(date +%s) >= deadline)); then
    printf 'no "%s" alert with %s reached alert-sink within %ss\n' "$rule" "$*" "$timeout" >&2
    exit 1
  fi
  sleep 10
done

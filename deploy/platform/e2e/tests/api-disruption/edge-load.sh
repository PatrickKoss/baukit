#!/usr/bin/env bash
# Drives the echo app through Traefik from a pod in $NAMESPACE while a step disrupts it.
#
#   edge-load.sh start <name> <host> <load-seconds>   start the loader pod, return once it is warm
#   edge-load.sh finish <name> <max-failed> <min-ok>  wait for its summary and check the counts
#
# The loader runs the e2e echo image (built from a digest-pinned base and imported into the
# cluster, so it pulls nothing). hostAliases point the ingress host at the Traefik ClusterIP, so
# the request carries the same SNI and Host header a browser would and needs no egress.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
image=localhost/baukit-e2e/echo:local
warm_seconds=60
summary_slack_seconds=60
poll_seconds=2

usage() {
  printf 'usage: %s start NAME HOST LOAD_SECONDS | finish NAME MAX_FAILED MIN_OK\n' "${0##*/}" >&2
  exit 2
}

pod_name() {
  printf 'edge-load-%s\n' "$1"
}

phase() {
  kubectl -n "$NAMESPACE" get pod "$1" -o jsonpath='{.status.phase}' 2>/dev/null || true
}

start() {
  local name host load_seconds pod traefik_ip deadline
  name=$1 host=$2 load_seconds=$3
  pod=$(pod_name "$name")
  traefik_ip=$(kubectl -n traefik get service traefik -o jsonpath='{.spec.clusterIP}')
  kubectl -n "$NAMESPACE" create configmap edge-loop --from-file=edge-loop.sh="$script_dir/edge-loop.sh" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  kubectl apply -f - >/dev/null <<YAML
apiVersion: v1
kind: Pod
metadata:
  name: $pod
  namespace: $NAMESPACE
  labels:
    baukit.dev/e2e-load: "true"
spec:
  restartPolicy: Never
  activeDeadlineSeconds: $((warm_seconds + load_seconds + summary_slack_seconds))
  automountServiceAccountToken: false
  enableServiceLinks: false
  terminationGracePeriodSeconds: 0
  hostAliases:
    - ip: $traefik_ip
      hostnames: [$host]
  securityContext:
    runAsNonRoot: true
    runAsUser: 101
    runAsGroup: 101
    seccompProfile: {type: RuntimeDefault}
  volumes:
    - name: loop
      configMap: {name: edge-loop}
  containers:
    - name: load
      image: $image
      imagePullPolicy: Never
      command: [sh, /load/edge-loop.sh]
      env:
        - {name: EDGE_HOST, value: $host}
        - {name: WARM_SECONDS, value: "$warm_seconds"}
        - {name: LOAD_SECONDS, value: "$load_seconds"}
      volumeMounts:
        - {name: loop, mountPath: /load, readOnly: true}
      resources:
        requests: {cpu: 20m, memory: 16Mi}
        limits: {cpu: 200m, memory: 32Mi}
      securityContext:
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        capabilities: {drop: [ALL]}
YAML
  deadline=$(($(date +%s) + warm_seconds + summary_slack_seconds))
  until kubectl -n "$NAMESPACE" logs "$pod" 2>/dev/null | grep -qx warm; do
    if [[ $(phase "$pod") == Failed || $(date +%s) -ge $deadline ]]; then
      printf 'loader %s never reached %s\n' "$pod" "$host" >&2
      kubectl -n "$NAMESPACE" logs "$pod" >&2 || kubectl -n "$NAMESPACE" describe pod "$pod" >&2
      exit 1
    fi
    sleep "$poll_seconds"
  done
  printf 'loader %s is warm against %s\n' "$pod" "$host"
}

finish() {
  local name max_failed min_ok pod deadline summary ok failed
  name=$1 max_failed=$2 min_ok=$3
  pod=$(pod_name "$name")
  if [[ $(phase "$pod") != Running ]]; then
    printf 'loader %s stopped before the disruption ended; raise its load seconds\n' "$pod" >&2
    kubectl -n "$NAMESPACE" logs "$pod" >&2 || true
    exit 1
  fi
  deadline=$(($(date +%s) + $(kubectl -n "$NAMESPACE" get pod "$pod" -o jsonpath='{.spec.activeDeadlineSeconds}')))
  until [[ $(phase "$pod") == Succeeded || $(phase "$pod") == Failed ]]; do
    if (($(date +%s) >= deadline)); then
      printf 'loader %s never finished\n' "$pod" >&2
      exit 1
    fi
    sleep "$poll_seconds"
  done
  kubectl -n "$NAMESPACE" logs "$pod"
  summary=$(kubectl -n "$NAMESPACE" logs "$pod" | sed -n 's/^summary ok=\([0-9]*\) failed=\([0-9]*\)$/\1 \2/p')
  if [[ -z $summary ]]; then
    printf 'loader %s printed no summary\n' "$pod" >&2
    exit 1
  fi
  read -r ok failed <<<"$summary"
  if ((ok < min_ok)); then
    printf '%s: only %s successful requests, fewer than %s, so the window proves nothing\n' "$name" "$ok" "$min_ok" >&2
    exit 1
  fi
  if ((failed > max_failed)); then
    printf '%s: %s failed requests at the edge, budget %s\n' "$name" "$failed" "$max_failed" >&2
    exit 1
  fi
  printf '%s: %s ok, %s failed, budget %s\n' "$name" "$ok" "$failed" "$max_failed"
  kubectl -n "$NAMESPACE" delete pod "$pod" --ignore-not-found >/dev/null
}

(($# == 4)) || usage
: "${NAMESPACE:?NAMESPACE is not set}"
case $1 in
  start) start "$2" "$3" "$4" ;;
  finish) finish "$2" "$3" "$4" ;;
  *) usage ;;
esac

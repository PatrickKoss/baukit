# shellcheck shell=bash
# Checks of the k3s server flags that chainsaw cannot reach: the Pod Security default before Kyverno
# denies unlabeled namespaces, and a report-only CIS benchmark.

psa_probe_image=registry.k8s.io/pause:3.10

# Usage: psa_probe_pod <namespace> <privileged true|false>. Server dry-run, so PSA admits or rejects
# without scheduling anything.
psa_probe_pod() {
  kubectl -n "$1" run psa-probe --image="$psa_probe_image" --restart=Never --dry-run=server \
    --overrides="{\"spec\":{\"containers\":[{\"name\":\"psa-probe\",\"image\":\"$psa_probe_image\",\"securityContext\":{\"privileged\":$2}}]}}"
}

# An unlabeled namespace enforces baseline and warns on restricted; kube-system stays exempt.
check_pod_security_default() {
  local namespace output
  namespace=psa-default-$(openssl rand -hex 4)
  kubectl create namespace "$namespace" >/dev/null
  if output=$(psa_probe_pod "$namespace" true 2>&1); then
    kubectl delete namespace "$namespace" --wait=false >/dev/null
    printf 'ERROR: an unlabeled namespace admitted a privileged pod\n' >&2
    return 1
  fi
  if [[ $output != *'violates PodSecurity "baseline:latest"'* ]]; then
    kubectl delete namespace "$namespace" --wait=false >/dev/null
    printf 'ERROR: unexpected rejection of the privileged pod:\n%s\n' "$output" >&2
    return 1
  fi
  output=$(psa_probe_pod "$namespace" false 2>&1) || {
    kubectl delete namespace "$namespace" --wait=false >/dev/null
    printf 'ERROR: an unlabeled namespace rejected a baseline pod:\n%s\n' "$output" >&2
    return 1
  }
  kubectl delete namespace "$namespace" --wait=false >/dev/null
  if [[ $output != *'would violate PodSecurity "restricted:latest"'* ]]; then
    printf 'ERROR: no restricted warning for a baseline pod in an unlabeled namespace:\n%s\n' "$output" >&2
    return 1
  fi
  output=$(psa_probe_pod kube-system true 2>&1) || {
    printf 'ERROR: kube-system rejected a privileged pod:\n%s\n' "$output" >&2
    return 1
  }
  printf 'Pod Security default: unlabeled namespaces enforce baseline and warn restricted; kube-system is exempt\n'
}

# Report-only CIS benchmark of the e2e server node. Findings never fail the run; a Job that does not
# finish or prints no summary does.

kube_bench_image=docker.io/aquasec/kube-bench:v0.16.0@sha256:75506f222d1eb6ce2a751a5533bdc0a3b54c898e2e49e7751d0ee22cfb862679
kube_bench_benchmark=k3s-cis-1.9

# kube-system is the only namespace Pod Security leaves privileged, and kube-bench needs hostPID plus
# the k3s state directories.
kube_bench_job() {
  cat <<YAML
apiVersion: batch/v1
kind: Job
metadata:
  name: kube-bench
  namespace: kube-system
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 600
  template:
    spec:
      hostPID: true
      restartPolicy: Never
      automountServiceAccountToken: false
      containers:
        - name: kube-bench
          image: $kube_bench_image
          command: [kube-bench, run, --benchmark, $kube_bench_benchmark]
          resources:
            requests: {cpu: 50m, memory: 64Mi}
            limits: {cpu: 500m, memory: 256Mi}
          volumeMounts:
            - {name: rancher, mountPath: /var/lib/rancher, readOnly: true}
            - {name: etc-rancher, mountPath: /etc/rancher, readOnly: true}
            - {name: kubelet, mountPath: /var/lib/kubelet, readOnly: true}
            - {name: cni, mountPath: /var/lib/cni, readOnly: true}
            - {name: k3s-config, mountPath: /etc/baukit/k3s, readOnly: true}
      volumes:
        - {name: rancher, hostPath: {path: /var/lib/rancher}}
        - {name: etc-rancher, hostPath: {path: /etc/rancher}}
        - {name: kubelet, hostPath: {path: /var/lib/kubelet}}
        - {name: cni, hostPath: {path: /var/lib/cni}}
        - {name: k3s-config, hostPath: {path: /etc/baukit/k3s}}
YAML
}

# Prints "PASS FAIL WARN INFO" from the "== Summary total ==" block of a kube-bench report.
kube_bench_score() {
  awk '
    /^== Summary total ==/ { total = 1; next }
    total && /checks PASS/ { pass = $1 }
    total && /checks FAIL/ { fail = $1 }
    total && /checks WARN/ { warn = $1 }
    total && /checks INFO/ { info = $1 }
    END {
      if (pass == "" || fail == "" || warn == "" || info == "") exit 1
      print pass, fail, warn, info
    }
  ' "$1"
}

kube_bench_report() {
  local report=$1/kube-bench.txt score pass fail warn info
  printf 'Running kube-bench %s (report only)\n' "$kube_bench_benchmark"
  kubectl -n kube-system delete job kube-bench --ignore-not-found >/dev/null
  kube_bench_job | kubectl apply -f - >/dev/null
  if ! kubectl -n kube-system wait job/kube-bench --for=condition=Complete --timeout=5m >/dev/null; then
    kubectl -n kube-system logs job/kube-bench >"$report" 2>&1
    printf 'ERROR: kube-bench did not complete; see %s\n' "$report" >&2
    return 1
  fi
  kubectl -n kube-system logs job/kube-bench >"$report"
  if ! score=$(kube_bench_score "$report"); then
    printf 'ERROR: kube-bench printed no summary; see %s\n' "$report" >&2
    return 1
  fi
  read -r pass fail warn info <<<"$score"
  printf 'kube-bench %s: %s PASS, %s FAIL, %s WARN, %s INFO (report: %s)\n' \
    "$kube_bench_benchmark" "$pass" "$fail" "$warn" "$info" "$report"
}

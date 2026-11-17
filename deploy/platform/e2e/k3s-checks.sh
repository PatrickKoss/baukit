# shellcheck shell=bash
# Checks of the k3s server flags that chainsaw cannot reach: the Pod Security default before Kyverno
# denies unlabeled namespaces, and the CIS benchmark.

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

# CIS benchmark of the e2e server node. Any FAIL outside kube_bench_skips fails the run; WARN (manual
# checks) is report only.

kube_bench_image=docker.io/aquasec/kube-bench:v0.16.0@sha256:75506f222d1eb6ce2a751a5533bdc0a3b54c898e2e49e7751d0ee22cfb862679
kube_bench_benchmark=k3s-cis-1.9

# 1.1.20: k3s writes server/tls/*.crt 0644 and rewrites them on rotation. They are public certificates;
#   the keys are 0600 and 1.1.21 checks them.
# 5.1.1: Flux's kustomize-controller and helm-controller bind cluster-admin to apply cluster-scoped
#   platform manifests.
# 5.1.3: upstream Flux, Flagger and Kubernetes aggregate ClusterRoles (admin, edit, view,
#   storage-version-migrator) carry wildcards the k3s allowlist does not name.
# 5.1.6: operators (Flux, Kyverno, cert-manager, CNPG, Falco, Alloy) need their tokens mounted.
kube_bench_skips=1.1.20,5.1.1,5.1.3,5.1.6

# k3d runs k3s as PID 1 without journald, so the audits' `journalctl -u k3s` finds nothing. This
# journalctl prints the component lines k3s logged to Docker at start.
kube_bench_journal_dir=/opt/baukit-journal
kube_bench_components='kube-apiserver|kube-controller-manager|kube-scheduler|kubelet|kube-proxy'

# Usage: kube_bench_journal <server container>. Fails unless every component logged its args.
kube_bench_journal() {
  local lines component
  lines=$(docker logs "$1" 2>&1 | grep -E "Running ($kube_bench_components) ") || true
  for component in ${kube_bench_components//|/ }; do
    if ! grep -q "Running $component " <<<"$lines"; then
      printf 'ERROR: %s logged no "Running %s" line for kube-bench\n' "$1" "$component" >&2
      return 1
    fi
  done
  printf '%s\n' "$lines"
}

kube_bench_journal_configmap() {
  kubectl -n kube-system create configmap kube-bench-journal --dry-run=client -o yaml \
    --from-literal=k3s.log="$1" \
    --from-literal=journalctl="$(printf '#!/bin/sh\ncat %s/k3s.log\n' "$kube_bench_journal_dir")"
}

# 5.1.5 lists ServiceAccounts with kubectl; nothing else the Job runs reads the API.
kube_bench_rbac() {
  cat <<YAML
apiVersion: v1
kind: ServiceAccount
metadata:
  name: kube-bench
  namespace: kube-system
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: kube-bench
rules:
  - apiGroups: [""]
    resources: [serviceaccounts]
    verbs: [list]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: kube-bench
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: kube-bench
subjects:
  - kind: ServiceAccount
    name: kube-bench
    namespace: kube-system
YAML
}

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
      serviceAccountName: kube-bench
      containers:
        - name: kube-bench
          image: $kube_bench_image
          command: [kube-bench, run, --benchmark, $kube_bench_benchmark, --skip, '$kube_bench_skips']
          env:
            - name: PATH
              value: $kube_bench_journal_dir:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
          resources:
            requests: {cpu: 50m, memory: 64Mi}
            limits: {cpu: 500m, memory: 256Mi}
          volumeMounts:
            - {name: rancher, mountPath: /var/lib/rancher, readOnly: true}
            - {name: etc-rancher, mountPath: /etc/rancher, readOnly: true}
            - {name: kubelet, mountPath: /var/lib/kubelet, readOnly: true}
            - {name: cni, mountPath: /var/lib/cni, readOnly: true}
            - {name: k3s-config, mountPath: /etc/baukit/k3s, readOnly: true}
            - {name: journal, mountPath: $kube_bench_journal_dir, readOnly: true}
      volumes:
        - {name: rancher, hostPath: {path: /var/lib/rancher}}
        - {name: etc-rancher, hostPath: {path: /etc/rancher}}
        - {name: kubelet, hostPath: {path: /var/lib/kubelet}}
        - {name: cni, hostPath: {path: /var/lib/cni}}
        - {name: k3s-config, hostPath: {path: /etc/baukit/k3s}}
        - {name: journal, configMap: {name: kube-bench-journal, defaultMode: 0555}}
YAML
}

# Prints "PASS FAIL WARN INFO" from the "== Summary total ==" block of a kube-bench report.
kube_bench_score() {
  awk -v header="== Summary ${2:-total} ==" '
    /^== Summary / { inside = ($0 == header); next }
    inside && /checks PASS/ { pass = $1 }
    inside && /checks FAIL/ { fail = $1 }
    inside && /checks WARN/ { warn = $1 }
    inside && /checks INFO/ { info = $1 }
    END {
      if (pass == "" || fail == "" || warn == "" || info == "") exit 1
      print pass, fail, warn, info
    }
  ' "$1"
}

# Usage: kube_bench_report <artifacts dir> <server container>
kube_bench_report() {
  local report=$1/kube-bench.txt score pass fail warn info section journal failed=0
  printf 'Running kube-bench %s (skipping %s)\n' "$kube_bench_benchmark" "$kube_bench_skips"
  journal=$(kube_bench_journal "$2") || return 1
  kube_bench_journal_configmap "$journal" | kubectl apply -f - >/dev/null
  kube_bench_rbac | kubectl apply -f - >/dev/null
  kubectl -n kube-system delete job kube-bench --ignore-not-found >/dev/null
  kube_bench_job | kubectl apply -f - >/dev/null
  if ! kubectl -n kube-system wait job/kube-bench --for=condition=Complete --timeout=5m >/dev/null; then
    kubectl -n kube-system logs job/kube-bench >"$report" 2>&1
    printf 'ERROR: kube-bench did not complete; see %s\n' "$report" >&2
    return 1
  fi
  kubectl -n kube-system logs job/kube-bench >"$report"
  for section in total etcd; do
    if ! score=$(kube_bench_score "$report" "$section"); then
      printf 'ERROR: kube-bench printed no %s summary; see %s\n' "$section" "$report" >&2
      return 1
    fi
    read -r pass fail warn info <<<"$score"
    printf 'kube-bench %s %s: %s PASS, %s FAIL, %s WARN, %s INFO (report: %s)\n' \
      "$kube_bench_benchmark" "$section" "$pass" "$fail" "$warn" "$info" "$report"
    ((fail == 0)) || failed=1
  done
  if ((failed)); then
    printf 'ERROR: kube-bench reported FAIL checks:\n' >&2
    grep '^\[FAIL\]' "$report" >&2
    return 1
  fi
}

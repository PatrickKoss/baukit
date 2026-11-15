#!/usr/bin/env bash
# Shared runtime Secrets and ConfigMaps for platform-up.sh and e2e/run.sh. Callers source this file.

platform_runtime_namespaces=(postgres observability observability-agents)

# Usage: platform_apply_secret <namespace> <name> <kubectl create secret generic args>...
platform_apply_secret() {
  local namespace=$1 name=$2
  shift 2
  kubectl -n "$namespace" create secret generic "$name" "$@" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
}

# Usage: platform_ensure_namespace <namespace>. Creates it with the platform part-of label.
platform_ensure_namespace() {
  kubectl create namespace "$1" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  kubectl label namespace "$1" app.kubernetes.io/part-of=baukit-platform --overwrite >/dev/null
}

# Usage: platform_runtime_identity <environment> <postgres-password> <object-access-key>
#   <object-secret-key> <grafana-user> <grafana-password>
platform_runtime_identity() {
  local environment=$1 postgres_password=$2 access_key=$3 secret_key=$4
  local grafana_user=$5 grafana_password=$6 namespace
  for namespace in "${platform_runtime_namespaces[@]}"; do
    platform_ensure_namespace "$namespace"
  done
  platform_apply_secret postgres product-db-credentials --type=kubernetes.io/basic-auth \
    --from-literal=username=product_owner --from-literal=password="$postgres_password"
  platform_apply_secret postgres postgres-backup-credentials \
    --from-literal=ACCESS_KEY_ID="$access_key" --from-literal=ACCESS_SECRET_KEY="$secret_key"
  for namespace in postgres observability; do
    platform_apply_secret "$namespace" rustfs-root \
      --from-literal=username="$access_key" --from-literal=password="$secret_key"
  done
  platform_apply_secret observability grafana-admin \
    --from-literal=admin-user="$grafana_user" --from-literal=admin-password="$grafana_password"
  kubectl -n observability-agents create configmap observability-cluster \
    --from-literal=environment="$environment" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
}

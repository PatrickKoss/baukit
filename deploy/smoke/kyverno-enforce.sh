#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
deploy_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
kyverno_dir=${BAUKIT_SMOKE_KYVERNO_DIR:-$deploy_root/platform/kyverno}
policy_library=${BAUKIT_SMOKE_KYVERNO_POLICY_LIBRARY:-$deploy_root/platform/kyverno-policies}
kyverno_namespace=kyverno
ready_timeout=${BAUKIT_SMOKE_KYVERNO_TIMEOUT:-300s}

usage() {
    echo "usage: $0 render <out-dir> | install <allowed-image>..." >&2
    exit 2
}

write_parser_chart() {
    local chart=$1
    mkdir -p "$chart/templates" "$chart/files"
    printf 'apiVersion: v2\nname: smoke-kyverno\nversion: 0.0.0\n' >"$chart/Chart.yaml"
    cp "$kyverno_dir/helm-release.yaml" "$chart/files/helm-release.yaml"
    cp "$kyverno_dir/helm-repository.yaml" "$chart/files/helm-repository.yaml"
    kubectl kustomize "$policy_library" >"$chart/files/policies.yaml"
    cat >"$chart/templates/_release.tpl" <<'EOF'
{{- define "release" -}}
{{- $hr := fromYaml (.Files.Get "files/helm-release.yaml") -}}
{{- if hasKey $hr "Error" }}{{ fail $hr.Error }}{{ end -}}
{{- toJson $hr -}}
{{- end -}}
EOF
    cat >"$chart/templates/chart.yaml" <<'EOF'
{{- $hr := include "release" . | fromJson }}
{{- $repo := fromYaml (.Files.Get "files/helm-repository.yaml") }}
{{- $spec := $hr.spec.chart.spec }}
{{- if ne $spec.sourceRef.name $repo.metadata.name }}{{ fail (printf "HelmRelease uses repository %s, not %s" $spec.sourceRef.name $repo.metadata.name) }}{{ end }}
{{- if not (regexMatch "^[0-9]+\\.[0-9]+\\.[0-9]+$" (toString $spec.version)) }}{{ fail (printf "chart version %v is not an exact pin" $spec.version) }}{{ end }}
chart: {{ $spec.chart }}
version: {{ $spec.version }}
repository: {{ $repo.spec.url }}
EOF
    cat >"$chart/templates/values.yaml" <<'EOF'
{{- $hr := include "release" . | fromJson }}
{{ toYaml $hr.spec.values }}
EOF
    cat >"$chart/templates/policies.yaml" <<'EOF'
{{- $skipped := list "HelmRelease" "HelmRepository" }}
{{- $enforced := list "ValidatingPolicy" "ImageValidatingPolicy" }}
{{- $known := list "GeneratingPolicy" "PolicyException" }}
{{- range $doc := regexSplit "(?m)^---[ \\t]*$" (.Files.Get "files/policies.yaml") -1 }}
{{- $obj := fromYaml $doc }}
{{- if hasKey $obj "Error" }}{{ fail $obj.Error }}{{ end }}
{{- if and $obj (not (has $obj.kind $skipped)) }}
{{- if has $obj.kind $enforced }}
{{- $_ := set $obj.spec "validationActions" (list "Deny") }}
{{- else if not (has $obj.kind $known) }}
{{- fail (printf "the smoke cannot enforce %s %s" $obj.kind $obj.metadata.name) }}
{{- end }}
---
{{ toYaml $obj }}
{{- end }}
{{- end }}
EOF
}

render() {
    local out=$1 chart
    mkdir -p "$out"
    chart=$(mktemp -d)
    write_parser_chart "$chart"
    for part in chart values policies; do
        helm template smoke-kyverno "$chart" --show-only "templates/$part.yaml" >"$out/$part.yaml"
    done
    rm -r -- "$chart"
}

chart_field() {
    sed -n "s/^$2: //p" "$1/chart.yaml"
}

install() {
    local work chart version repository
    work=$(mktemp -d)
    trap 'rm -r -- "$work"' RETURN
    render "$work"
    chart=$(chart_field "$work" chart)
    version=$(chart_field "$work" version)
    repository=$(chart_field "$work" repository)
    echo "installing Kyverno chart $chart $version from $repository"
    kubectl apply -f "$kyverno_dir/namespace.yaml"
    helm upgrade --install kyverno "$chart" \
        --repo "$repository" \
        --version "$version" \
        --namespace "$kyverno_namespace" \
        --values "$work/values.yaml" \
        --set admissionController.serviceMonitor.enabled=false \
        --set backgroundController.serviceMonitor.enabled=false \
        --set cleanupController.serviceMonitor.enabled=false \
        --set reportsController.serviceMonitor.enabled=false \
        --wait \
        --timeout "$ready_timeout"
    kubectl -n "$kyverno_namespace" create configmap baukit-registry-allowlist \
        --from-literal=registries="$(printf '%s\n' "$@")" \
        --dry-run=client -o yaml | kubectl apply -f -
    echo "applying the Baukit policy library in Enforce mode"
    kubectl apply -f "$work/policies.yaml"
    wait_for_policies "$work/policies.yaml"
}

wait_for_policies() {
    local kind name resource
    while read -r kind name; do
        case $kind in
            ValidatingPolicy) resource=validatingpolicies.policies.kyverno.io ;;
            ImageValidatingPolicy) resource=imagevalidatingpolicies.policies.kyverno.io ;;
            GeneratingPolicy) resource=generatingpolicies.policies.kyverno.io ;;
            *) continue ;;
        esac
        kubectl wait --for=jsonpath='{.status.conditionStatus.ready}'=true "$resource/$name" --timeout="$ready_timeout"
    done < <(awk '/^kind: /{kind=$2} /^  name: /{if (kind != "") print kind, $2; kind=""}' "$1")
}

[ $# -ge 1 ] || usage
command_name=$1
shift
case $command_name in
    render)
        [ $# -eq 1 ] || usage
        render "$1"
        ;;
    install)
        [ $# -ge 1 ] || usage
        install "$@"
        ;;
    *) usage ;;
esac

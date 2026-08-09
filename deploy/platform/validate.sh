#!/usr/bin/env bash
set -euo pipefail

platform_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$platform_dir/../.." && pwd)
cache_dir="$platform_dir/.helm-cache"

missing_tools=()
for tool in kustomize kubeconform helm; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    missing_tools+=("$tool")
  fi
done

if ((${#missing_tools[@]} > 0)); then
  printf 'ERROR: required platform validation tools are missing: %s\n' "${missing_tools[*]}" >&2
  printf 'Install pinned releases into a directory on PATH. Upstream releases:\n' >&2
  printf '  kustomize:  https://github.com/kubernetes-sigs/kustomize/releases\n' >&2
  printf '  kubeconform: https://github.com/yannh/kubeconform/releases\n' >&2
  printf '  helm:        https://github.com/helm/helm/releases\n' >&2
  exit 2
fi

mkdir -p "$cache_dir/repository-cache" "$cache_dir/kubeconform-schemas"
export HELM_REPOSITORY_CONFIG="$cache_dir/repositories.yaml"
export HELM_REPOSITORY_CACHE="$cache_dir/repository-cache"

work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT

# kubeconform validates every core/built-in object strictly. The explicit skip
# list prevents network schema lookups for CRD definitions and custom-resource
# kinds known to these bases:
# Flux HelmRepository/HelmRelease/GitRepository/Kustomization; Traefik
# Middleware; Kyverno ClusterPolicy; cert-manager resources; Prometheus
# Operator ServiceMonitor/PodMonitor/PrometheusRule; CloudNativePG/Barman
# Cluster/ObjectStore/Backup/ScheduledBackup/Pooler; and Keycloak Operator
# resources. -ignore-missing-schemas remains a fallback for newly added CRDs.
# Downloaded core schemas share the offline cache with the Helm chart archives.
kubeconform_skip_kinds=CustomResourceDefinition,HelmRepository,HelmRelease,GitRepository,Kustomization,Middleware,ClusterPolicy,Certificate,Issuer,ClusterIssuer,ServiceMonitor,PodMonitor,PrometheusRule,Cluster,ObjectStore,Backup,ScheduledBackup,Pooler,Keycloak,KeycloakRealmImport

strip_yaml_quotes() {
  local value=$1
  if [[ $value == \"*\" && $value == *\" ]]; then
    value=${value#\"}
    value=${value%\"}
  elif [[ $value == \'*\' && $value == *\' ]]; then
    value=${value#\'}
    value=${value%\'}
  fi
  printf '%s' "$value"
}

metadata_field() {
  local document=$1
  local key=$2
  awk -v wanted="$key" '
    $0 == "metadata:" { in_metadata = 1; next }
    in_metadata && /^[^ ]/ { exit }
    in_metadata && index($0, "  " wanted ":") == 1 {
      value = substr($0, length(wanted) + 4)
      sub(/^[[:space:]]+/, "", value)
      print value
      exit
    }
  ' "$document"
}

spec_url() {
  local document=$1
  awk '
    $0 == "spec:" { in_spec = 1; next }
    in_spec && index($0, "  url:") == 1 {
      value = substr($0, 7)
      sub(/^[[:space:]]+/, "", value)
      print value
      exit
    }
  ' "$document"
}

chart_field() {
  local document=$1
  local key=$2
  awk -v wanted="$key" '
    $0 == "  chart:" { in_chart = 1; next }
    in_chart && $0 == "    spec:" { in_chart_spec = 1; next }
    in_chart_spec && index($0, "      " wanted ":") == 1 {
      value = substr($0, length(wanted) + 9)
      sub(/^[[:space:]]+/, "", value)
      print value
      exit
    }
  ' "$document"
}

source_ref_field() {
  local document=$1
  local key=$2
  awk -v wanted="$key" '
    $0 == "      sourceRef:" { in_source_ref = 1; next }
    in_source_ref && index($0, "        " wanted ":") == 1 {
      value = substr($0, length(wanted) + 11)
      sub(/^[[:space:]]+/, "", value)
      print value
      exit
    }
    in_source_ref && $0 !~ /^        / && $0 !~ /^[[:space:]]*$/ { exit }
  ' "$document"
}

extract_values() {
  local document=$1
  local destination=$2
  awk '
    $0 == "  values:" { in_values = 1; next }
    in_values {
      if ($0 !~ /^    / && $0 !~ /^[[:space:]]*$/) {
        exit
      }
      sub(/^    /, "")
      print
    }
  ' "$document" >"$destination"
  if [[ ! -s $destination ]]; then
    printf '{}\n' >"$destination"
  fi
}

split_documents() {
  local rendered=$1
  local destination=$2
  mkdir -p "$destination"
  awk -v output_dir="$destination" '
    BEGIN { document = 1 }
    /^---[[:space:]]*$/ { document++; next }
    {
      output = sprintf("%s/document-%04d.yaml", output_dir, document)
      print > output
    }
  ' "$rendered"
}

status=0
skipped_charts=0
validated_bases=0
validated_charts=0

mapfile -d '' bases < <(
  find "$platform_dir" -mindepth 1 -maxdepth 1 -type d ! -name overlays \
    -exec test -f '{}/kustomization.yaml' \; -print0 | sort -z
)

if ((${#bases[@]} == 0)); then
  printf 'ERROR: no platform bases found under %s\n' "$platform_dir" >&2
  exit 1
fi

for base in "${bases[@]}"; do
  base_name=${base##*/}
  rendered="$work_dir/$base_name.yaml"
  documents="$work_dir/$base_name-documents"
  printf '==> %s: kustomize build\n' "$base_name"
  if ! kustomize build "$base" >"$rendered"; then
    status=1
    continue
  fi

  printf '==> %s: kubeconform\n' "$base_name"
  if ! kubeconform -strict -summary -ignore-missing-schemas \
    -skip "$kubeconform_skip_kinds" \
    -cache "$cache_dir/kubeconform-schemas" \
    "$rendered"; then
    status=1
    continue
  fi
  ((validated_bases += 1))

  split_documents "$rendered" "$documents"
  declare -A repositories=()

  for document in "$documents"/*.yaml; do
    [[ -s $document ]] || continue
    if grep -qx 'kind: HelmRepository' "$document"; then
      repository_name=$(strip_yaml_quotes "$(metadata_field "$document" name)")
      repository_namespace=$(strip_yaml_quotes "$(metadata_field "$document" namespace)")
      repository_url=$(strip_yaml_quotes "$(spec_url "$document")")
      if [[ -z $repository_name || -z $repository_url ]]; then
        printf 'ERROR: %s contains an incomplete HelmRepository\n' "$base_name" >&2
        status=1
        continue
      fi
      repositories["$repository_namespace/$repository_name"]=$repository_url
    fi
  done

  for document in "$documents"/*.yaml; do
    [[ -s $document ]] || continue
    grep -qx 'kind: HelmRelease' "$document" || continue

    release_name=$(strip_yaml_quotes "$(metadata_field "$document" name)")
    release_namespace=$(strip_yaml_quotes "$(metadata_field "$document" namespace)")
    chart_name=$(strip_yaml_quotes "$(chart_field "$document" chart)")
    chart_version=$(strip_yaml_quotes "$(chart_field "$document" version)")
    source_kind=$(strip_yaml_quotes "$(source_ref_field "$document" kind)")
    source_name=$(strip_yaml_quotes "$(source_ref_field "$document" name)")
    source_namespace=$(strip_yaml_quotes "$(source_ref_field "$document" namespace)")
    source_namespace=${source_namespace:-$release_namespace}
    values_file="$work_dir/$base_name-${document##*/}-values.yaml"
    extract_values "$document" "$values_file"

    if [[ -z $release_name || -z $chart_name || -z $source_kind ]]; then
      printf 'ERROR: %s contains an incomplete HelmRelease chart reference\n' "$base_name" >&2
      status=1
      continue
    fi

    chart_source=""
    case $source_kind in
      HelmRepository)
        if [[ -z $chart_version ]]; then
          printf 'ERROR: %s/%s does not pin a chart version\n' "$base_name" "$release_name" >&2
          status=1
          continue
        fi
        repository_key="$source_namespace/$source_name"
        repository_url=${repositories[$repository_key]:-}
        if [[ -z $repository_url ]]; then
          printf 'ERROR: %s/%s references missing HelmRepository %s\n' \
            "$base_name" "$release_name" "$repository_key" >&2
          status=1
          continue
        fi

        repository_id=$(printf '%s' "$repository_url" | cksum | awk '{print $1}')
        cache_key=$(printf '%s-%s-%s' "$chart_name" "$chart_version" "$repository_id" | tr -c 'A-Za-z0-9._-' '_')
        chart_source="$cache_dir/$cache_key.tgz"

        if [[ ! -s $chart_source ]]; then
          repository_alias="baukit-$repository_id"
          pull_dir="$work_dir/pull-$cache_key"
          mkdir -p "$pull_dir"
          printf '==> %s/%s: cache miss; fetching %s %s\n' \
            "$base_name" "$release_name" "$chart_name" "$chart_version"
          if ! helm repo add "$repository_alias" "$repository_url" --force-update >/dev/null 2>&1 ||
            ! helm pull "$repository_alias/$chart_name" --version "$chart_version" --destination "$pull_dir" >/dev/null 2>&1; then
            printf 'SKIP: %s/%s chart is not cached and could not be fetched (offline?)\n' \
              "$base_name" "$release_name" >&2
            ((skipped_charts += 1))
            continue
          fi
          pulled_chart=$(find "$pull_dir" -maxdepth 1 -type f -name '*.tgz' -print -quit)
          if [[ -z $pulled_chart ]]; then
            printf 'ERROR: Helm reported success but produced no chart for %s/%s\n' \
              "$base_name" "$release_name" >&2
            status=1
            continue
          fi
          mv -- "$pulled_chart" "$chart_source"
        fi
        ;;
      GitRepository)
        if [[ $chart_name != ./* ]]; then
          printf 'ERROR: %s/%s GitRepository chart must be a repository-relative ./ path\n' \
            "$base_name" "$release_name" >&2
          status=1
          continue
        fi
        chart_source="$repo_root/${chart_name#./}"
        if [[ ! -f $chart_source/Chart.yaml ]]; then
          printf 'ERROR: local chart for %s/%s not found at %s\n' \
            "$base_name" "$release_name" "$chart_source" >&2
          status=1
          continue
        fi
        ;;
      *)
        printf 'ERROR: %s/%s uses unsupported chart source kind %s\n' \
          "$base_name" "$release_name" "$source_kind" >&2
        status=1
        continue
        ;;
    esac

    printf '==> %s/%s: helm lint and template\n' "$base_name" "$release_name"
    if ! helm lint "$chart_source" --values "$values_file" >/dev/null ||
      ! helm template "$release_name" "$chart_source" \
        --namespace "${release_namespace:-default}" \
        --include-crds \
        --api-versions monitoring.coreos.com/v1/ServiceMonitor \
        --api-versions monitoring.coreos.com/v1/PodMonitor \
        --api-versions monitoring.coreos.com/v1/PrometheusRule \
        --values "$values_file" >/dev/null; then
      status=1
      continue
    fi
    ((validated_charts += 1))
  done

  unset repositories
done

validated_overlays=0
if [[ -d "$platform_dir/overlays" ]]; then
  mapfile -d '' overlays < <(
    find "$platform_dir/overlays" -mindepth 1 -maxdepth 1 -type d \
      -exec test -f '{}/kustomization.yaml' \; -print0 | sort -z
  )
  for overlay in "${overlays[@]}"; do
    overlay_name="overlays/${overlay##*/}"
    rendered="$work_dir/overlay-${overlay##*/}.yaml"
    printf '==> %s: kustomize build\n' "$overlay_name"
    if ! kustomize build "$overlay" >"$rendered"; then
      status=1
      continue
    fi
    printf '==> %s: kubeconform\n' "$overlay_name"
    if ! kubeconform -strict -summary -ignore-missing-schemas \
      -skip "$kubeconform_skip_kinds" \
      -cache "$cache_dir/kubeconform-schemas" \
      "$rendered"; then
      status=1
      continue
    fi
    ((validated_overlays += 1))
  done
fi

printf 'Platform validation: %d bases, %d charts rendered, %d charts skipped (cold/offline cache), %d overlays.\n' \
  "$validated_bases" "$validated_charts" "$skipped_charts" "$validated_overlays"
exit "$status"

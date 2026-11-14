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

for tool in flux python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'ERROR: required platform validation tool is missing: %s\n' "$tool" >&2
    exit 2
  fi
done

schema_dir="$cache_dir/crd-schemas"
# The default kubernetes-json-schema location has no standalone CustomResourceDefinition
# schema, so generate.py builds one from this pinned _definitions.json.
kubernetes_schema_commit=2bfaff3898a7a4fa4e4760da3d2656211c9b4c71
kubernetes_definitions_sha256=ce5535db873c37e5e16a1e8e4b4fa0a17bf3cdd3847394e135aeaff3bfcfc9a3
schema_template="$schema_dir/{{ .ResourceKind }}_{{ .ResourceAPIVersion }}.json"

# Core kinds come from the default kubernetes-json-schema location, cached offline.
# Every custom resource kind needs a schema generated from its CRD, so a kind
# without one is an error.
run_kubeconform() {
  kubeconform -strict -summary \
    -cache "$cache_dir/kubeconform-schemas" \
    -schema-location default \
    -schema-location "$schema_template" \
    "$@"
}

if [[ ${1:-} == --kubeconform ]]; then
  shift
  if [[ ! -f $schema_dir/.inputs.sha256 ]]; then
    printf 'ERROR: no generated CRD schemas in %s; run %s first\n' "$schema_dir" "$0" >&2
    exit 2
  fi
  mkdir -p "$cache_dir/kubeconform-schemas"
  run_kubeconform "$@"
  exit
fi

pinned_flux_version=$(sed -n 's/^flux_version=[$]{PLATFORM_FLUX_VERSION:-\(v[0-9.]*\)}$/\1/p' \
  "$platform_dir/platform-lifecycle.sh")
installed_flux_version=$(flux version --client | awk '{print $2}')
if [[ -z $pinned_flux_version || $installed_flux_version != "$pinned_flux_version" ]]; then
  printf 'ERROR: flux CLI %s does not match the pinned Flux %s\n' \
    "$installed_flux_version" "${pinned_flux_version:-<unknown>}" >&2
  exit 2
fi

mkdir -p "$cache_dir/repository-cache" "$cache_dir/kubeconform-schemas"
export HELM_REPOSITORY_CONFIG="$cache_dir/repositories.yaml"
export HELM_REPOSITORY_CACHE="$cache_dir/repository-cache"

work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT
rendered_dir="$work_dir/rendered"
mkdir -p "$rendered_dir"

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
rendered_bases=0
rendered_charts=0
rendered_overlays=0

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
  rendered="$rendered_dir/base-$base_name.yaml"
  documents="$work_dir/$base_name-documents"
  printf '==> %s: kustomize build\n' "$base_name"
  if ! kustomize build "$base" >"$rendered"; then
    status=1
    continue
  fi
  ((rendered_bases += 1))

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
            printf 'ERROR: %s/%s chart %s %s is not cached and could not be fetched\n' \
              "$base_name" "$release_name" "$chart_name" "$chart_version" >&2
            status=1
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
        --values "$values_file" >"$rendered_dir/chart-$base_name-$release_name.yaml"; then
      status=1
      continue
    fi
    ((rendered_charts += 1))
  done

  unset repositories
done

if [[ -d "$platform_dir/overlays" ]]; then
  mapfile -d '' overlays < <(
    find "$platform_dir/overlays" -mindepth 1 -maxdepth 1 -type d \
      -exec test -f '{}/kustomization.yaml' \; -print0 | sort -z
  )
  for overlay in "${overlays[@]}"; do
    overlay_name="overlays/${overlay##*/}"
    printf '==> %s: kustomize build\n' "$overlay_name"
    if ! kustomize build "$overlay" >"$rendered_dir/overlay-${overlay##*/}.yaml"; then
      status=1
      continue
    fi
    ((rendered_overlays += 1))
  done
fi

flux_manifest="$work_dir/flux-install-$pinned_flux_version.yaml"
flux install --export >"$flux_manifest"

kubernetes_definitions="$cache_dir/kubernetes-definitions-$kubernetes_schema_commit.json"
if [[ ! -s $kubernetes_definitions ]]; then
  printf '==> fetching kubernetes-json-schema definitions %s\n' "$kubernetes_schema_commit"
  if ! curl -fsSL --retry 3 -o "$work_dir/definitions.json" \
    "https://raw.githubusercontent.com/yannh/kubernetes-json-schema/$kubernetes_schema_commit/master/_definitions.json"; then
    printf 'ERROR: kubernetes-json-schema definitions are not cached and could not be fetched\n' >&2
    exit 1
  fi
  mv -- "$work_dir/definitions.json" "$kubernetes_definitions"
fi
if ! printf '%s  %s\n' "$kubernetes_definitions_sha256" "$kubernetes_definitions" | sha256sum --check --quiet; then
  printf 'ERROR: %s does not match its pinned sha256\n' "$kubernetes_definitions" >&2
  exit 1
fi

mapfile -d '' rendered_files < <(find "$rendered_dir" -type f -name '*.yaml' -print0 | sort -z)

printf '==> CRD schemas\n'
if ! python3 "$platform_dir/schemas/generate.py" --output "$schema_dir" \
  --kubernetes-definitions "$kubernetes_definitions" \
  "$flux_manifest" "${rendered_files[@]}"; then
  exit 1
fi

checked_objects=0
for rendered in "${rendered_files[@]}"; do
  printf '==> %s: kubeconform\n' "${rendered##*/}"
  summary_file="$work_dir/kubeconform-summary"
  if ! run_kubeconform "$rendered" | tee "$summary_file"; then
    status=1
  fi
  valid=$(sed -n 's/^Summary: .* Valid: \([0-9]*\),.*/\1/p' "$summary_file")
  ((checked_objects += ${valid:-0}))
done

printf '==> schema and fixture tests\n'
if ! PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s "$platform_dir/schemas" -p 'test_*.py' ||
  ! PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s "$platform_dir/lint" -p 'test_*.py'; then
  status=1
fi

printf 'Platform validation: %d bases, %d charts, %d overlays rendered; %d objects schema-checked.\n' \
  "$rendered_bases" "$rendered_charts" "$rendered_overlays" "$checked_objects"
exit "$status"

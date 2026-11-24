#!/usr/bin/env bash
# Pull-through caches for the e2e clusters and the platform-images trivy scan, shared across runs.
# Each mirror publishes on 127.0.0.1 only and keeps its blobs in a named volume. Clients still check
# every digest pin, and go straight to the upstream registry when a mirror is down.
#
#   deploy/platform/registry-mirrors.sh start    start missing or stopped mirrors
#   deploy/platform/registry-mirrors.sh stop     stop them, keeping the cached blobs
#   deploy/platform/registry-mirrors.sh status   list each mirror and whether it runs
#
# Remove the cache with: docker volume rm $(docker volume ls -q --filter name=k3d-baukit-mirror-)

registry_mirror_image=docker.io/library/registry:3.1.2@sha256:82f6ff4a7f982960c8aa51617fea3aabc485df6e55cce80d22deb1de36f1bc3c
# host=upstream URL=port on 127.0.0.1
registry_mirror_upstreams=(
  docker.io=https://registry-1.docker.io=5701
  ghcr.io=https://ghcr.io=5702
  quay.io=https://quay.io=5703
  registry.k8s.io=https://registry.k8s.io=5705
)
# host=mirrored host whose mirror serves it. reg.kyverno.io fronts ghcr.io/kyverno with the same
# digests, and the registry proxy cannot authenticate against it because its /v2/ answers 405.
registry_mirror_aliases=(
  reg.kyverno.io=ghcr.io
)

# Usage: registry_mirror_name <registry host>. Prints the k3d registry name without the k3d- prefix.
registry_mirror_name() {
  printf 'baukit-mirror-%s\n' "${1//./-}"
}

registry_mirror_fields() {
  local entry=$1
  mirror_host=${entry%%=*}
  mirror_port=${entry##*=}
  mirror_url=${entry#*=}
  mirror_url=${mirror_url%=*}
}

# Starts each mirror that is not running yet.
registry_mirrors_start() {
  local entry name mirror_host mirror_port mirror_url
  for entry in "${registry_mirror_upstreams[@]}"; do
    registry_mirror_fields "$entry"
    name=$(registry_mirror_name "$mirror_host")
    if docker container inspect "k3d-$name" >/dev/null 2>&1; then
      docker start "k3d-$name" >/dev/null
      continue
    fi
    k3d registry create "$name" \
      --image "$registry_mirror_image" \
      --port "127.0.0.1:$mirror_port" \
      --proxy-remote-url "$mirror_url" \
      --volume "k3d-$name:/var/lib/registry" >/dev/null
  done
}

registry_mirrors_stop() {
  local entry mirror_host mirror_port mirror_url
  for entry in "${registry_mirror_upstreams[@]}"; do
    registry_mirror_fields "$entry"
    docker stop "k3d-$(registry_mirror_name "$mirror_host")" >/dev/null 2>&1 || true
  done
}

registry_mirrors_detach() {
  local network=$1 entry mirror_host mirror_port mirror_url
  for entry in "${registry_mirror_upstreams[@]}"; do
    registry_mirror_fields "$entry"
    docker network disconnect "$network" "k3d-$(registry_mirror_name "$mirror_host")" >/dev/null 2>&1 || true
  done
}

registry_mirrors_status() {
  local entry mirror_host mirror_port mirror_url name state
  for entry in "${registry_mirror_upstreams[@]}"; do
    registry_mirror_fields "$entry"
    name=k3d-$(registry_mirror_name "$mirror_host")
    state=$(docker container inspect -f '{{.State.Status}}' "$name" 2>/dev/null || printf 'absent')
    printf '%s 127.0.0.1:%s %s %s\n' "$name" "$mirror_port" "$mirror_host" "$state"
  done
}

# Usage: registry_mirrors_check_extra [host=endpoint ...]. Fails unless each entry names a registry
# host baukit does not mirror itself and an http(s) endpoint, each host once.
registry_mirrors_check_extra() {
  local entry host seen=" " mirror_host mirror_port mirror_url known
  for entry in "$@"; do
    host=${entry%%=*}
    if [[ $entry != *=* || ! $host =~ ^[a-z0-9]([a-z0-9.-]*[a-z0-9])?(:[0-9]+)?$ ||
      ! ${entry#*=} =~ ^https?://[^[:space:]/]+$ ]]; then
      printf 'ERROR: extra registry mirror %s must be host=http(s)://address[:port]\n' "$entry" >&2
      return 2
    fi
    for known in "${registry_mirror_upstreams[@]}" "${registry_mirror_aliases[@]}"; do
      registry_mirror_fields "$known"
      if [[ $mirror_host == "$host" ]]; then
        printf 'ERROR: extra registry mirror %s overrides a baukit mirror\n' "$host" >&2
        return 2
      fi
    done
    if [[ $seen == *" $host "* ]]; then
      printf 'ERROR: extra registry mirror %s is listed twice\n' "$host" >&2
      return 2
    fi
    seen+="$host "
  done
}

# Usage: registry_mirrors_extra_k3s_config [host=endpoint ...]. Prints the mirrors entries for them.
registry_mirrors_extra_k3s_config() {
  local entry
  for entry in "$@"; do
    printf '  %s:\n    endpoint:\n      - %s\n' "${entry%%=*}" "${entry#*=}"
  done
}

# Usage: registry_mirrors_k3s_config [host=endpoint ...]. Prints the k3s registries.yaml that routes
# each upstream to its mirror on the cluster network, plus the extra mirrors.
registry_mirrors_k3s_config() {
  local entry mirror_host mirror_port mirror_url
  printf 'mirrors:\n'
  for entry in "${registry_mirror_upstreams[@]}"; do
    registry_mirror_fields "$entry"
    printf '  %s:\n    endpoint:\n      - http://k3d-%s:5000\n' "$mirror_host" "$(registry_mirror_name "$mirror_host")"
  done
  for entry in "${registry_mirror_aliases[@]}"; do
    printf '  %s:\n    endpoint:\n      - http://k3d-%s:5000\n' "${entry%%=*}" "$(registry_mirror_name "${entry#*=}")"
  done
  registry_mirrors_extra_k3s_config "$@"
}

# Prints one k3d cluster create argument per line.
registry_mirrors_cluster_args() {
  local entry mirror_host mirror_port mirror_url
  for entry in "${registry_mirror_upstreams[@]}"; do
    registry_mirror_fields "$entry"
    printf -- '--registry-use\nk3d-%s:5000\n' "$(registry_mirror_name "$mirror_host")"
  done
}

# Succeeds only when every mirror container is running.
registry_mirrors_running() {
  local entry mirror_host mirror_port mirror_url running
  for entry in "${registry_mirror_upstreams[@]}"; do
    registry_mirror_fields "$entry"
    running=$(docker container inspect -f '{{.State.Running}}' "k3d-$(registry_mirror_name "$mirror_host")" 2>/dev/null) ||
      return 1
    [[ $running == true ]] || return 1
  done
}

# Usage: registry_mirrors_k3d_args <registries.yaml path> [host=endpoint ...]. Writes the k3s
# registry config with the extra mirrors to the path and prints one k3d cluster create argument per
# line. Delete the cluster with registry_mirrors_detach first, or the cluster network stays behind.
registry_mirrors_k3d_args() {
  local config=$1
  shift
  registry_mirrors_k3s_config "$@" >"$config"
  printf -- '--registry-config\n%s\n' "$config"
  registry_mirrors_cluster_args
}

# Usage: registry_mirrors_extra_k3d_args <registries.yaml path> host=endpoint .... The same for the
# extra mirrors alone, when the baukit mirrors are not running.
registry_mirrors_extra_k3d_args() {
  local config=$1
  shift
  { printf 'mirrors:\n'; registry_mirrors_extra_k3s_config "$@"; } >"$config"
  printf -- '--registry-config\n%s\n' "$config"
}

# Prints the trivy config that routes each upstream to its mirror on 127.0.0.1.
registry_mirrors_trivy_config() {
  local entry mirror_host mirror_port mirror_url
  printf 'registry:\n  mirrors:\n'
  for entry in "${registry_mirror_upstreams[@]}"; do
    registry_mirror_fields "$entry"
    # trivy names Docker Hub index.docker.io, k3s names it docker.io.
    [[ $mirror_host == docker.io ]] && mirror_host=index.docker.io
    printf '    %s:\n      - localhost:%s\n' "$mirror_host" "$mirror_port"
  done
  local alias_entry alias_target
  for alias_entry in "${registry_mirror_aliases[@]}"; do
    alias_target=${alias_entry#*=}
    for entry in "${registry_mirror_upstreams[@]}"; do
      registry_mirror_fields "$entry"
      if [[ $mirror_host == "$alias_target" ]]; then
        printf '    %s:\n      - localhost:%s\n' "${alias_entry%%=*}" "$mirror_port"
      fi
    done
  done
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  set -euo pipefail
  repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
  # shellcheck source=deploy/lib/tools.sh
  . "$repo_root/deploy/lib/tools.sh"
  baukit_use_tools "$repo_root" k3d
  case ${1:-} in
    start) registry_mirrors_start ;;
    stop) registry_mirrors_stop ;;
    status) registry_mirrors_status ;;
    *)
      printf 'usage: %s start|stop|status\n' "${0##*/}" >&2
      exit 2
      ;;
  esac
fi

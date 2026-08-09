#!/usr/bin/env bash
set -euo pipefail

platform_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$platform_dir/../.." && pwd)

usage() {
  cat >&2 <<EOF
usage: ${0##*/} [--config FILE] up|down|nuke|recreate|status

Configuration is read from the environment after the optional trusted shell
configuration file. See deploy/platform/README.md for the complete contract.
EOF
  exit 2
}

config_file=
if [[ ${1:-} == --config ]]; then
  [[ $# -ge 3 ]] || usage
  config_file=$2
  shift 2
fi
command=${1:-}
[[ $# -eq 1 ]] || usage

if [[ -n $config_file ]]; then
  [[ -f $config_file ]] || {
    printf 'ERROR: configuration file not found: %s\n' "$config_file" >&2
    exit 2
  }
  # The configuration is a trusted shell fragment so that paths can use
  # parameter expansion. It must contain assignments only.
  # shellcheck disable=SC1090
  source "$config_file"
fi

cluster_name=${PLATFORM_CLUSTER_NAME:-}
gitops_url=${PLATFORM_GITOPS_URL:-}
gitops_branch=${PLATFORM_GITOPS_BRANCH:-main}
gitops_path=${PLATFORM_GITOPS_PATH:-}
gitops_auth_mode=${PLATFORM_GITOPS_AUTH_MODE:-github}
gitops_private_key_file=${PLATFORM_GITOPS_PRIVATE_KEY_FILE:-}
github_owner=${PLATFORM_GITHUB_OWNER:-}
github_repository=${PLATFORM_GITHUB_REPOSITORY:-}
github_owner_type=${PLATFORM_GITHUB_OWNER_TYPE:-organization}
age_key_file=${PLATFORM_AGE_KEY_FILE:-}
baukit_source_mode=${PLATFORM_BAUKIT_SOURCE_MODE:-github-tag}
baukit_checkout=${PLATFORM_BAUKIT_CHECKOUT:-}
k3s_image=${PLATFORM_K3S_IMAGE:-rancher/k3s:v1.34.8-k3s1}
flux_version=${PLATFORM_FLUX_VERSION:-v2.9.4}
git_port=${PLATFORM_SNAPSHOT_PORT:-9418}
state_root=${XDG_STATE_HOME:-${HOME}/.local/state}
state_dir=${PLATFORM_STATE_DIR:-$state_root/baukit-platform/$cluster_name}
snapshot_container=${PLATFORM_SNAPSHOT_CONTAINER:-${cluster_name}-baukit-git}
snapshot_image=${PLATFORM_SNAPSHOT_IMAGE:-${cluster_name}-baukit-git-http}
git_root=$state_dir/git

require_value() {
  local name=$1
  local value=$2
  if [[ -z $value ]]; then
    printf 'ERROR: %s is required\n' "$name" >&2
    exit 2
  fi
}

validate_common_config() {
  require_value PLATFORM_CLUSTER_NAME "$cluster_name"
  if [[ ! $cluster_name =~ ^[a-z0-9][a-z0-9.-]*$ ]]; then
    printf 'ERROR: PLATFORM_CLUSTER_NAME must contain only lowercase letters, digits, dots, or hyphens\n' >&2
    exit 2
  fi
  if [[ ! $git_port =~ ^[0-9]+$ ]] || ((git_port < 1 || git_port > 65535)); then
    printf 'ERROR: PLATFORM_SNAPSHOT_PORT must be a TCP port number\n' >&2
    exit 2
  fi
  case $state_dir in
    ''|/|"$HOME"|"$repo_root"|"$platform_dir")
      printf 'ERROR: unsafe PLATFORM_STATE_DIR: %s\n' "$state_dir" >&2
      exit 2
      ;;
  esac
  case $baukit_source_mode in
    github-tag) ;;
    local-snapshot)
      require_value PLATFORM_BAUKIT_CHECKOUT "$baukit_checkout"
      ;;
    *)
      printf 'ERROR: PLATFORM_BAUKIT_SOURCE_MODE must be github-tag or local-snapshot\n' >&2
      exit 2
      ;;
  esac
}

validate_up_config() {
  validate_common_config
  require_value PLATFORM_GITOPS_URL "$gitops_url"
  require_value PLATFORM_GITOPS_PATH "$gitops_path"
  require_value PLATFORM_AGE_KEY_FILE "$age_key_file"
  [[ -f $age_key_file ]] || {
    printf 'ERROR: SOPS age identity not found: %s\n' "$age_key_file" >&2
    exit 2
  }
  case $gitops_auth_mode in
    github)
      require_value PLATFORM_GITHUB_OWNER "$github_owner"
      require_value PLATFORM_GITHUB_REPOSITORY "$github_repository"
      require_value GITHUB_TOKEN "${GITHUB_TOKEN:-}"
      case $github_owner_type in
        organization|personal) ;;
        *)
          printf 'ERROR: PLATFORM_GITHUB_OWNER_TYPE must be organization or personal\n' >&2
          exit 2
          ;;
      esac
      ;;
    ssh-key)
      require_value PLATFORM_GITOPS_PRIVATE_KEY_FILE "$gitops_private_key_file"
      [[ -f $gitops_private_key_file ]] || {
        printf 'ERROR: Git private key not found: %s\n' "$gitops_private_key_file" >&2
        exit 2
      }
      ;;
    *)
      printf 'ERROR: PLATFORM_GITOPS_AUTH_MODE must be github or ssh-key\n' >&2
      exit 2
      ;;
  esac
}

require_tools() {
  local missing=()
  local tool
  for tool in age docker flux git k3d kubectl sops tar; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  if ((${#missing[@]})); then
    printf 'ERROR: missing required tools: %s\n' "${missing[*]}" >&2
    exit 2
  fi
}

cluster_exists() {
  k3d cluster list --no-headers 2>/dev/null | awk '{print $1}' | grep -Fqx -- "$cluster_name"
}

use_cluster() {
  k3d kubeconfig merge "$cluster_name" --kubeconfig-switch-context >/dev/null
  kubectl config use-context "k3d-$cluster_name" >/dev/null
}

snapshot_exists() {
  docker container inspect "$snapshot_container" >/dev/null 2>&1
}

stop_snapshot() {
  if snapshot_exists && [[ $(docker inspect -f '{{.State.Running}}' "$snapshot_container") == true ]]; then
    docker stop "$snapshot_container" >/dev/null
  fi
}

remove_snapshot() {
  if snapshot_exists; then
    docker rm -f -v "$snapshot_container" >/dev/null
  fi
}

wait_for_snapshot() {
  local snapshot_url="http://127.0.0.1:$git_port/baukit.git"
  local attempt
  for ((attempt = 0; attempt < 100; attempt++)); do
    if git ls-remote "$snapshot_url" HEAD >/dev/null 2>&1; then
      printf 'Baukit snapshot: http://host.k3d.internal:%s/baukit.git\n' "$git_port"
      return
    fi
    sleep 0.1
  done
  printf 'ERROR: snapshot server did not become ready; inspect docker logs %s\n' "$snapshot_container" >&2
  exit 1
}

build_snapshot() {
  [[ -d $baukit_checkout/.git && -d $baukit_checkout/deploy/platform ]] || {
    printf 'ERROR: Baukit checkout not found at %s\n' "$baukit_checkout" >&2
    exit 2
  }
  mkdir -p "$state_dir"
  chmod 0700 "$state_dir"
  local snapshot_tmp=$state_dir/git-snapshot.new
  local git_tmp=$state_dir/git.new
  remove_snapshot
  rm -rf -- "$snapshot_tmp" "$git_tmp"
  mkdir -p "$snapshot_tmp" "$git_tmp"
  (
    cd "$baukit_checkout"
    tar \
      --exclude=.git \
      --exclude=.generated-fixture \
      --exclude=deploy/platform/.local-state \
      --exclude='*/target' \
      --exclude='*/node_modules' \
      --exclude='*/.turbo' \
      -cf - .
  ) | tar -xf - -C "$snapshot_tmp"
  (
    cd "$snapshot_tmp"
    git init -q -b main
    git config user.name 'Baukit local platform'
    git config user.email 'local-platform@invalid'
    git add -A
    GIT_AUTHOR_DATE='2000-01-01T00:00:00Z' \
      GIT_COMMITTER_DATE='2000-01-01T00:00:00Z' \
      git commit -q -m 'Disposable local Baukit snapshot'
  )
  git clone -q --bare "$snapshot_tmp" "$git_tmp/baukit.git"
  rm -rf -- "$state_dir/git-snapshot" "$git_root"
  mv -- "$snapshot_tmp" "$state_dir/git-snapshot"
  mv -- "$git_tmp" "$git_root"

  docker build -q -f "$platform_dir/local-git-http.Dockerfile" \
    -t "$snapshot_image" "$platform_dir" >/dev/null
  docker run -d \
    --name "$snapshot_container" \
    --restart unless-stopped \
    -p "$git_port:$git_port" \
    -v "$git_root:/srv/git:ro" \
    "$snapshot_image" --root /srv/git --port "$git_port" >/dev/null
  wait_for_snapshot
}

ensure_snapshot() {
  if [[ $baukit_source_mode == local-snapshot ]]; then
    build_snapshot
  else
    remove_snapshot
  fi
}

ensure_cluster() {
  if cluster_exists; then
    printf 'Starting existing k3d cluster %s\n' "$cluster_name"
    k3d cluster start "$cluster_name" >/dev/null
  else
    printf 'Creating k3d cluster %s with %s\n' "$cluster_name" "$k3s_image"
    k3d cluster create "$cluster_name" \
      --image "$k3s_image" \
      --servers 1 \
      --agents 0 \
      --k3s-arg '--disable=traefik@server:0' \
      --port '80:80@loadbalancer' \
      --port '443:443@loadbalancer' \
      --wait
  fi
  local containers=()
  mapfile -t containers < <(docker ps -aq --filter "label=k3d.cluster=$cluster_name")
  if ((${#containers[@]})); then
    docker update --restart unless-stopped "${containers[@]}" >/dev/null
  fi
  use_cluster
  kubectl wait node --all --for=condition=Ready --timeout=5m
}

ensure_snapshot_dns() {
  [[ $baukit_source_mode == local-snapshot ]] || return 0
  local gateway
  gateway=$(docker network inspect "k3d-$cluster_name" \
    --format '{{range .IPAM.Config}}{{.Gateway}}{{end}}')
  [[ -n $gateway ]] || {
    printf 'ERROR: cannot determine the k3d Docker network gateway\n' >&2
    exit 1
  }
  local server_block
  server_block=$(printf 'host.k3d.internal:53 {\n  hosts {\n    %s host.k3d.internal\n    fallthrough\n  }\n}' "$gateway")
  kubectl -n kube-system create configmap coredns-custom \
    --from-literal=host.k3d.internal.server="$server_block" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  kubectl -n kube-system rollout restart deployment/coredns >/dev/null
  kubectl -n kube-system rollout status deployment/coredns --timeout=5m >/dev/null
  printf 'Pinned host.k3d.internal to the persistent k3d gateway %s in CoreDNS.\n' "$gateway"
}

install_sops_key() {
  kubectl create namespace flux-system --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  kubectl -n flux-system create secret generic sops-age \
    --from-file=age.agekey="$age_key_file" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  printf 'Installed Secret/flux-system/sops-age from the configured age identity.\n'
}

bootstrap_gitops() {
  printf 'Bootstrapping Flux %s\n' "$flux_version"
  install_sops_key

  case $gitops_auth_mode in
    github)
      local personal_flag=()
      if [[ $github_owner_type == personal ]]; then
        personal_flag+=(--personal)
      fi
      flux bootstrap github \
        --owner="$github_owner" \
        --repository="$github_repository" \
        --branch="$gitops_branch" \
        --path="$gitops_path" \
        --version="$flux_version" \
        --reconcile \
        "${personal_flag[@]}"
      ;;
    ssh-key)
      flux bootstrap git \
        --url="$gitops_url" \
        --branch="$gitops_branch" \
        --path="$gitops_path" \
        --private-key-file="$gitops_private_key_file" \
        --version="$flux_version" \
        --silent
      ;;
  esac
  local reconciled_url
  reconciled_url=$(kubectl -n flux-system get gitrepository.source.toolkit.fluxcd.io flux-system \
    -o jsonpath='{.spec.url}')
  if [[ $reconciled_url != "$gitops_url" ]]; then
    printf 'ERROR: bootstrapped GitRepository URL %s does not match PLATFORM_GITOPS_URL %s\n' \
      "$reconciled_url" "$gitops_url" >&2
    exit 1
  fi
}

print_health() {
  printf '\nK3d cluster:\n'
  k3d cluster list | awk 'NR == 1 || $1 == name' name="$cluster_name"
  printf '\nContainers and restart policies:\n'
  local containers=()
  mapfile -t containers < <(docker ps -aq --filter "label=k3d.cluster=$cluster_name")
  if ((${#containers[@]})); then
    docker inspect "${containers[@]}" \
      --format '{{.Name}} status={{.State.Status}} restart={{.HostConfig.RestartPolicy.Name}}'
  fi
  if snapshot_exists; then
    docker inspect "$snapshot_container" \
      --format '{{.Name}} status={{.State.Status}} restart={{.HostConfig.RestartPolicy.Name}}'
  fi
  printf '\nFlux sources:\n'
  flux get sources git -A || true
  printf '\nFlux Kustomizations:\n'
  flux get kustomizations -A || true
  printf '\nFlux HelmReleases:\n'
  flux get helmreleases -A || true
  printf '\nNon-ready pods:\n'
  kubectl get pods -A --field-selector=status.phase!=Running,status.phase!=Succeeded || true
}

wait_for_reconciliation() {
  flux reconcile kustomization flux-system --namespace flux-system --with-source --timeout=5m
  kubectl -n flux-system wait --for=create \
    gitrepository.source.toolkit.fluxcd.io/baukit --timeout=5m
  flux reconcile source git baukit --namespace flux-system --timeout=5m
  printf 'Waiting for all GitOps Kustomizations and HelmReleases.\n'
  kubectl wait kustomizations.kustomize.toolkit.fluxcd.io -A --all \
    --for=condition=Ready --timeout=45m
  kubectl wait helmreleases.helm.toolkit.fluxcd.io -A --all \
    --for=condition=Ready --timeout=45m
}

up() {
  require_tools
  validate_up_config
  ensure_snapshot
  ensure_cluster
  ensure_snapshot_dns
  if [[ $baukit_source_mode == local-snapshot ]]; then
    docker update --restart unless-stopped "$snapshot_container" >/dev/null
  fi
  bootstrap_gitops
  wait_for_reconciliation
  print_health
}

down() {
  require_tools
  validate_common_config
  if cluster_exists; then
    k3d cluster stop "$cluster_name"
  else
    printf 'Cluster %s is already absent.\n' "$cluster_name"
  fi
  stop_snapshot
  printf 'Stopped cluster and snapshot containers; persistent volumes and snapshot state remain.\n'
}

nuke() {
  require_tools
  validate_common_config
  if cluster_exists; then
    k3d cluster delete "$cluster_name"
  fi
  remove_snapshot

  local resource_ids=()
  mapfile -t resource_ids < <(docker ps -aq --filter "label=k3d.cluster=$cluster_name")
  if ((${#resource_ids[@]})); then
    docker rm -f -v "${resource_ids[@]}" >/dev/null
  fi
  mapfile -t resource_ids < <(docker volume ls -q --filter "label=k3d.cluster=$cluster_name")
  if ((${#resource_ids[@]})); then
    docker volume rm "${resource_ids[@]}" >/dev/null
  fi
  if docker network inspect "k3d-$cluster_name" >/dev/null 2>&1; then
    docker network rm "k3d-$cluster_name" >/dev/null
  fi
  if docker image inspect "$snapshot_image" >/dev/null 2>&1; then
    docker image rm "$snapshot_image" >/dev/null
  fi
  if [[ -d $state_dir ]]; then
    rm -rf -- "$state_dir"
  fi
  printf 'Deleted cluster containers, volumes, network, snapshot container/image, and %s.\n' "$state_dir"
}

recreate() {
  validate_up_config
  nuke
  up
}

status() {
  require_tools
  validate_common_config
  if ! cluster_exists; then
    printf 'Cluster %s: absent\n' "$cluster_name"
    snapshot_exists && docker inspect "$snapshot_container" \
      --format 'Snapshot: {{.State.Status}} (restart={{.HostConfig.RestartPolicy.Name}})'
    return
  fi
  use_cluster
  print_health
}

case $command in
  up) up ;;
  down) down ;;
  nuke) nuke ;;
  recreate) recreate ;;
  status) status ;;
  *) usage ;;
esac

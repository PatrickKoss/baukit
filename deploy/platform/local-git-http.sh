# shellcheck shell=bash
# Helpers shared by platform-up.sh and platform-lifecycle.sh for the local Git
# snapshot that Flux pulls from the host, plus the local k3s image that
# deploy/smoke/k3d-smoke.sh uses too.

# Local clusters run the newest k3s release. Kubernetes 1.35 and later do not
# support cgroup v1, so platform_require_cgroup_v2 stops on a cgroup v1 Docker engine.
platform_k3s_image() {
  printf '%s\n' rancher/k3s:v1.37.2-k3s1@sha256:21ea6e0fac4e3930c2ce8aa40ddd41a8b18cb3e9a0222c030dc005e7e88c4332
}

platform_k3s_config_dir() {
  local here
  here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd) || return 1
  printf '%s/k3s\n' "$here"
}

# k3s server arguments shared by every local cluster and the e2e cluster. The
# files they name come from platform_k3s_config_dir, mounted at
# platform_k3s_config_mount. k3s/server-help-<tag>.txt holds `k3s server --help`
# of the pinned image; a test checks every flag against it.
platform_k3s_config_mount=/etc/baukit/k3s
platform_k3s_create_timeout=${PLATFORM_K3S_CREATE_TIMEOUT:-5m}
platform_k3s_audit_log=/var/log/kubernetes/audit/audit.log
# Local and e2e clusters only, production datastore stays undecided. kine/SQLite dies under host
# starvation; on etcd a lost lease exits the server, so leader election is off and wrangler's widened.
platform_local_etcd_args() {
  printf '%s\n' \
    --k3s-arg --cluster-init@server:0 \
    --k3s-arg --kube-controller-manager-arg=leader-elect=false@server:0 \
    --k3s-arg --kube-cloud-controller-manager-arg=leader-elect=false@server:0 \
    --k3s-arg --kube-scheduler-arg=leader-elect=false@server:0 \
    --env CATTLE_ELECTION_LEASE_DURATION=180s@server:0 \
    --env CATTLE_ELECTION_RENEW_DEADLINE=150s@server:0 \
    --env CATTLE_ELECTION_RETRY_PERIOD=10s@server:0
}

# Local and e2e clusters share the host disk with gate runs, so k3s's percentage thresholds
# (5% free, reclaim to 15%) tainted a 1 TB node until about 160 GB were free. Absolute ones do not.
platform_local_kubelet_args() {
  printf '%s\n' \
    --k3s-arg '--kubelet-arg=eviction-hard=memory.available<100Mi,nodefs.available<20Gi,imagefs.available<20Gi@server:0' \
    --k3s-arg '--kubelet-arg=eviction-minimum-reclaim=nodefs.available=2Gi,imagefs.available=2Gi@server:0'
}

# nodeCIDR of overlays/local/cluster-network. The node-cidr NetworkPolicies only let the API server
# reach webhooks from this range, so every local cluster network must sit inside it.
platform_node_cidr=10.89.0.0/16

# Prints one "<network> <subnet>..." line per Docker network. Other gates create and remove networks
# concurrently; inspect prints the ones still present and fails on a vanished one, which holds no subnet.
platform_docker_subnets() {
  local networks
  networks=$(docker network ls -q) || return 1
  [[ -n $networks ]] || return 0
  xargs docker network inspect --format '{{.Name}}{{range .IPAM.Config}} {{.Subnet}}{{end}}' \
    <<<"$networks" 2>/dev/null || true
}

# Fails unless SUBNET is an IPv4 network inside platform_node_cidr that no Docker network overlaps.
# The optional OWN network is skipped; platform_check_cluster_network vets it.
platform_check_free_subnet() {
  local subnet=$1 own=${2:-} networks
  networks=$(platform_docker_subnets) || return 1
  python3 -c '
import ipaddress, sys
subnet, node_cidr, lines = sys.argv[1], ipaddress.ip_network(sys.argv[2]), sys.argv[3].splitlines()
own = sys.argv[4]
try:
    candidate = ipaddress.ip_network(subnet)
except ValueError as error:
    sys.exit(f"ERROR: PLATFORM_SUBNET {subnet} is not a network: {error}")
if candidate.version != 4 or not candidate.subnet_of(node_cidr):
    sys.exit(f"ERROR: PLATFORM_SUBNET {subnet} is not inside the nodeCIDR {node_cidr}")
clashes = []
for line in lines:
    name, *subnets = line.split()
    if name == own:
        continue
    for other in subnets:
        network = ipaddress.ip_network(other, strict=False)
        if network.version == 4 and candidate.overlaps(network):
            clashes.append(f"{name} ({other})")
listed = ", ".join(clashes)
if clashes:
    sys.exit(f"ERROR: PLATFORM_SUBNET {subnet} overlaps the Docker network(s) {listed}; "
             f"remove them or set PLATFORM_SUBNET to a free /24 inside {node_cidr}")
' "$subnet" "$platform_node_cidr" "$networks" "$own"
}

# Before a create: k3d reuses a Docker network named NETWORK and ignores --subnet, so a leftover one
# must carry exactly SUBNET. It is never removed here; the error prints the command.
platform_check_cluster_network() {
  local network=$1 subnet=$2 found subnets
  found=$(docker network ls -q --filter "name=^${network}\$") || return 1
  [[ -n $found ]] || return 0
  subnets=$(docker network inspect "$network" --format '{{range .IPAM.Config}}{{.Subnet}} {{end}}') || return 1
  python3 -c '
import ipaddress, sys
network, subnet, subnets = sys.argv[1], ipaddress.ip_network(sys.argv[2]), sys.argv[3].split()
ipv4 = [s for s in (ipaddress.ip_network(s, strict=False) for s in subnets) if s.version == 4]
if ipv4 != [subnet]:
    found = " ".join(map(str, ipv4)) or "no IPv4 subnet"
    sys.exit(f"ERROR: a leftover Docker network {network} uses {found}, not PLATFORM_SUBNET {subnet}; "
             f"k3d would reuse it and ignore --subnet. Remove it with: docker network rm {network}")
' "$network" "$subnet" "$subnets"
}

# Fails unless every IPv4 subnet of the Docker network NETWORK sits inside platform_node_cidr.
platform_check_network_in_node_cidr() {
  local network=$1 subnets
  subnets=$(docker network inspect "$network" --format '{{range .IPAM.Config}}{{.Subnet}} {{end}}') || return 1
  python3 -c '
import ipaddress, sys
network, node_cidr, subnets = sys.argv[1], ipaddress.ip_network(sys.argv[2]), sys.argv[3].split()
ipv4 = [ipaddress.ip_network(s, strict=False) for s in subnets]
ipv4 = [s for s in ipv4 if s.version == 4]
outside = [str(s) for s in ipv4 if not s.subnet_of(node_cidr)]
found = " ".join(outside) or "no IPv4 subnet"
if not ipv4 or outside:
    sys.exit(f"ERROR: Docker network {network} uses {found}, outside the "
             f"nodeCIDR {node_cidr}, so the API server cannot reach the admission webhooks; "
             "run recreate to rebuild the cluster on PLATFORM_SUBNET")
' "$network" "$platform_node_cidr" "$subnets"
}

# k3s creates these namespaces without Pod Security labels, which require-psa-labels reports, and
# with token-mounting default ServiceAccounts.
# kube-system matches its exemption in the admission config.
platform_label_system_namespaces() {
  local level namespace
  for namespace in kube-system default kube-public kube-node-lease; do
    level=restricted
    [[ $namespace == kube-system ]] && level=privileged
    kubectl label namespace "$namespace" --overwrite \
      pod-security.kubernetes.io/enforce="$level" \
      pod-security.kubernetes.io/enforce-version=v1.37 \
      pod-security.kubernetes.io/audit="$level" \
      pod-security.kubernetes.io/warn="$level" >/dev/null
    [[ $namespace == kube-system ]] || platform_default_service_account "$namespace" | kubectl apply -f - >/dev/null
  done
}

# CIS 5.1.5: default ServiceAccounts mount no token. kube-system is exempt, as in the benchmark.
platform_default_service_account() {
  cat <<YAML
apiVersion: v1
kind: ServiceAccount
metadata:
  name: default
  namespace: $1
automountServiceAccountToken: false
YAML
}

platform_k3s_server_args() {
  printf '%s\n' \
    --disable=traefik \
    --secrets-encryption \
    --protect-kernel-defaults \
    "--kube-apiserver-arg=admission-control-config-file=$platform_k3s_config_mount/pod-security.yaml" \
    "--kube-apiserver-arg=audit-policy-file=$platform_k3s_config_mount/audit-policy.yaml" \
    "--kube-apiserver-arg=audit-log-path=$platform_k3s_audit_log" \
    --kube-apiserver-arg=audit-log-maxage=7 \
    --kube-apiserver-arg=audit-log-maxbackup=3 \
    --kube-apiserver-arg=audit-log-maxsize=100 \
    "--kube-apiserver-arg=audit-webhook-config-file=$platform_k3s_config_mount/audit-webhook.yaml" \
    --kube-apiserver-arg=audit-webhook-batch-max-wait=5s
}

# The kubelet refuses to start under --protect-kernel-defaults unless the exact
# sysctls already hold the values it would set. The inotify minimums are shared
# by every container of the host user, so parallel clusters exhaust the default
# 128 instances and a k3s server never registers its node. Containers share the
# host values.
platform_require_kernel_defaults() {
  local sysctl_root=${platform_sysctl_root:-/proc/sys}
  local check setting want actual wrong=()
  for check in vm/overcommit_memory=1 vm/panic_on_oom=0 kernel/panic=10 kernel/panic_on_oops=1 \
    'fs/inotify/max_user_instances>=512' 'fs/inotify/max_user_watches>=524288'; do
    setting=${check%%[=>]*}
    want=${check#"$setting"}
    actual=$(cat -- "$sysctl_root/$setting" 2>/dev/null) || actual=unreadable
    if [[ $want == '>='* ]]; then
      [[ $actual =~ ^[0-9]+$ ]] && ((actual >= ${want#>=})) && continue
    elif [[ $actual == "${want#=}" ]]; then
      continue
    fi
    wrong+=("${setting//\//.}$want (now $actual)")
  done
  if ((${#wrong[@]})); then
    printf 'ERROR: local k3s needs these host sysctls (set them with sysctl -w): %s\n' "${wrong[*]}" >&2
    return 1
  fi
}

platform_require_cgroup_v2() {
  local version
  version=$(docker info --format '{{.CgroupVersion}}') || return 1
  if [[ $version != 2 ]]; then
    printf 'ERROR: Docker reports cgroup v%s; k3s v1.35 and later need cgroup v2. On WSL2 set kernelCommandLine = cgroup_no_v1=all under [wsl2] in .wslconfig and run wsl --shutdown.\n' "$version" >&2
    return 1
  fi
}

# Copies the files Git would commit (tracked plus untracked, minus ignored)
# from a checkout into an empty directory.
snapshot_tree() {
  local source=$1
  local destination=$2
  git -C "$source" ls-files -z --cached --others --exclude-standard |
    (
      cd -- "$source" || exit
      while IFS= read -r -d '' path; do
        if [[ -e $path || -L $path ]]; then
          printf '%s\0' "$path"
        fi
      done
    ) |
    tar -C "$source" --null --no-recursion -T - -cf - |
    tar -xf - -C "$destination"
}

# Prints the IPv4 gateway of a k3d cluster network. Pods reach the host there as
# host.k3d.internal, so host services bind to it instead of 0.0.0.0.
k3d_gateway() {
  local cluster=$1
  local gateway
  for gateway in $(docker network inspect "k3d-$cluster" \
    --format '{{range .IPAM.Config}}{{.Gateway}} {{end}}'); do
    if [[ $gateway =~ ^[0-9]+(\.[0-9]+){3}$ ]]; then
      printf '%s\n' "$gateway"
      return 0
    fi
  done
  printf 'ERROR: cannot determine the IPv4 gateway of Docker network k3d-%s\n' "$cluster" >&2
  return 1
}

# Creates a single-node k3d cluster with platform_k3s_server_args. The API and
# the load balancer ports bind 127.0.0.1 only. Arguments after the fifth go to k3d.
k3d_create_local_cluster() {
  local cluster=$1
  local image=$2
  local api_port=$3
  local http_port=$4
  local https_port=$5
  platform_require_cgroup_v2 || return 1
  platform_require_kernel_defaults || return 1
  local config_dir arg k3s_args=()
  config_dir=$(platform_k3s_config_dir) || return 1
  while IFS= read -r arg; do
    k3s_args+=(--k3s-arg "$arg@server:0")
  done < <(platform_k3s_server_args)
  if ! k3d cluster create "$cluster" \
    --image "$image" \
    --timeout "$platform_k3s_create_timeout" \
    --servers 1 \
    --agents 0 \
    --volume "$config_dir:$platform_k3s_config_mount:ro@server:0" \
    "${k3s_args[@]}" \
    --api-port "127.0.0.1:$api_port" \
    --port "127.0.0.1:$http_port:80@loadbalancer" \
    --port "127.0.0.1:$https_port:443@loadbalancer" \
    --wait \
    "${@:6}"; then
    printf 'ERROR: k3d cluster create %s failed or timed out after %s; last server log lines:\n' \
      "$cluster" "$platform_k3s_create_timeout" >&2
    docker logs --tail 30 "k3d-$cluster-server-0" >&2 2>&1 || true
    return 1
  fi
}

# Runs the snapshot image with the bare repositories mounted read-only. The
# port is published on the k3d gateway only, never on 0.0.0.0.
run_snapshot_container() {
  local container=$1
  local image=$2
  local git_root=$3
  local port=$4
  local gateway=$5
  docker run -d \
    --name "$container" \
    --restart unless-stopped \
    --read-only \
    --cap-drop ALL \
    --security-opt no-new-privileges \
    --tmpfs /tmp \
    -p "$gateway:$port:$port" \
    -v "$git_root:/srv/git:ro" \
    "$image" --host 0.0.0.0 --root /srv/git --port "$port"
}

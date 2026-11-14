# shellcheck shell=bash
# Helpers shared by platform-up.sh and platform-lifecycle.sh for the local Git
# snapshot that Flux pulls from the host.

# Prints the k3s image that local and e2e clusters run.
platform_k3s_image() { printf '%s\n' rancher/k3s:v1.34.8-k3s1; }

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

# Creates a single-node k3d cluster without the packaged Traefik. The API and
# the load balancer ports bind 127.0.0.1 only. Arguments after the fifth go to
# k3d unchanged.
k3d_create_local_cluster() {
  local cluster=$1
  local image=$2
  local api_port=$3
  local http_port=$4
  local https_port=$5
  shift 5
  k3d cluster create "$cluster" \
    --image "$image" \
    --servers 1 \
    --agents 0 \
    --k3s-arg '--disable=traefik@server:0' \
    --api-port "127.0.0.1:$api_port" \
    --port "127.0.0.1:$http_port:80@loadbalancer" \
    --port "127.0.0.1:$https_port:443@loadbalancer" \
    "$@" \
    --wait
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

# shellcheck shell=bash
# The k3s image and server flags every local k3d cluster uses: the lifecycle
# cluster and the e2e cluster. Hardening flags (plan S5) belong here.

# Kubernetes 1.35 removed cgroup-v1 support and the WSL2 Docker engine on the
# development host is cgroup-v1, so local clusters stay on the newest 1.34 patch.
# shellcheck disable=SC2034 # read by the scripts that source this file
platform_k3s_image=rancher/k3s:v1.34.8-k3s1

# Prints the k3d --k3s-arg values, one per line.
platform_k3s_args() {
  printf '%s\n' '--disable=traefik@server:0'
}

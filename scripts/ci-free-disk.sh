#!/usr/bin/env bash
set -euo pipefail

[[ "${GITHUB_ACTIONS:-}" == "true" ]] || { echo "ci-free-disk.sh only runs on GitHub-hosted runners" >&2; exit 2; }

unused_toolchains=(
  /usr/share/dotnet
  /usr/share/swift
  /usr/local/.ghcup
  /opt/ghc
  /usr/local/share/boost
  /usr/local/share/powershell
  /usr/local/julia*
  /opt/hostedtoolcache/CodeQL
  /opt/microsoft
)

df -h /
sudo rm -rf "${unused_toolchains[@]}"
sudo docker image prune --all --force >/dev/null
df -h /

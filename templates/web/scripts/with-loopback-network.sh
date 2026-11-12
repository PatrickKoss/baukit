#!/bin/sh
set -eu

if [ $# -eq 0 ]; then
  echo "usage: with-loopback-network.sh COMMAND [ARGUMENT ...]" >&2
  exit 64
fi

if ! unshare --user --map-root-user --net true 2>/dev/null; then
  echo "with-loopback-network.sh: unprivileged user and network namespaces are unavailable" >&2
  exit 69
fi

exec unshare --user --map-root-user --net sh -c 'ip link set lo up && exec "$@"' with-loopback-network "$@"

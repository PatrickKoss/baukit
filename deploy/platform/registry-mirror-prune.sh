#!/bin/sh
# Runs inside a registry mirror, fed on stdin by registry-mirrors.sh. Removes each repository layer
# link whose blob data is gone and prints its repository and digest. The proxy's TTL expiry deletes
# a blob shared by several repositories when the first repository's entry expires, and the others
# then answer 200 with an empty body (distribution/distribution#2367).
cd "${1:-/var/lib/registry}/docker/registry/v2" 2>/dev/null || exit 0
find repositories -path "*/_layers/sha256/*/link" 2>/dev/null | while read -r link; do
  digest=$(cat "$link")
  hex=${digest#sha256:}
  [ -f "blobs/sha256/${hex%"${hex#??}"}/$hex/data" ] && continue
  rm -rf "${link%/link}"
  printf "%s %s\n" "${link%%/_layers/*}" "$digest"
done

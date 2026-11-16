#!/usr/bin/env bash
# Validates syscall rule files with the pinned Falco image. With --k8saudit, the files are audit rules
# and Falco loads the k8saudit plugin that k8saudit-helm-release.yaml pins. Without files, it checks
# every shipped file of both kinds. With --k8saudit-replay EVENTS, Falco reads the audit events in
# EVENTS (one JSON event per line) through the plugin, applies the shipped audit rules and prints the
# name of each rule that fires with the user, namespace and subresource, tab separated, one per line.
set -euo pipefail

falco_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$falco_dir/../../.." && pwd)
k8saudit_release=$falco_dir/k8saudit-helm-release.yaml
plugin_cache=${XDG_CACHE_HOME:-$HOME/.cache}/baukit/falco-plugins

if ! command -v docker >/dev/null 2>&1; then
  printf 'ERROR: falco rule validation needs docker\n' >&2
  exit 2
fi

image=$(python3 -c '
import sys, tomllib
with open(sys.argv[1], "rb") as manifest:
    print(tomllib.load(manifest)["images"]["falco"])
' "$repo_root/deploy/tools.toml")
image="docker.io/${image%%:*}@${image##*@}"

# Prints the single line of the k8saudit release that matches PATTERN.
release_pin() {
  local pin
  pin=$(grep -oE "$1" "$k8saudit_release") || {
    printf 'ERROR: no pin matching %s in %s\n' "$1" "$k8saudit_release" >&2
    return 1
  }
  printf '%s\n' "$pin"
}

# Prints a directory holding libk8saudit.so, downloaded once per plugin digest with falcoctl.
k8saudit_plugin_dir() {
  local reference falcoctl_digest digest target staging
  reference=$(release_pin 'ghcr\.io/falcosecurity/plugins/plugin/k8saudit:[^@"]+@sha256:[a-f0-9]{64}') || return 1
  falcoctl_digest=$(grep -A2 'repository: falcosecurity/falcoctl' "$k8saudit_release" |
    grep -oE 'sha256:[a-f0-9]{64}') || {
    printf 'ERROR: no falcoctl digest in %s\n' "$k8saudit_release" >&2
    return 1
  }
  digest=${reference##*@sha256:}
  target=$plugin_cache/k8saudit-$digest
  if [[ ! -f $target/libk8saudit.so ]]; then
    mkdir -p "$plugin_cache"
    staging=$(mktemp -d "$plugin_cache/.k8saudit.XXXXXX")
    chmod 0777 "$staging"
    if ! docker run --rm --read-only --tmpfs /tmp --tmpfs /etc/falcoctl -e HOME=/tmp --user "$(id -u):$(id -g)" \
      -v "$staging:/plugins" "docker.io/falcosecurity/falcoctl@$falcoctl_digest" \
      artifact install "${reference%%:*}@sha256:$digest" --plugins-dir /plugins \
      --rulesfiles-dir /tmp --allowed-types plugin --resolve-deps=false >&2; then
      rm -rf -- "$staging"
      return 1
    fi
    chmod 0755 "$staging"
    mv -T -- "$staging" "$target" 2>/dev/null || rm -rf -- "$staging"
  fi
  printf '%s\n' "$target"
}

# Runs falco -V on each file after the rules files the image ships, named by the first argument
# (comma separated, may be empty). Extra docker arguments come before the first --.
run_validate() {
  local docker_args=() mounts=() arguments=() index=0 rules_file absolute name builtin
  for builtin in ${1//,/ }; do
    arguments+=(-V "$builtin")
  done
  shift
  while (($#)) && [[ $1 != -- ]]; do
    docker_args+=("$1")
    shift
  done
  shift
  for rules_file in "$@"; do
    absolute=$(cd -- "$(dirname -- "$rules_file")" && pwd)/$(basename -- "$rules_file")
    name=/baukit-rules/$index-$(basename -- "$rules_file")
    mounts+=(-v "$absolute:$name:ro")
    arguments+=(-V "$name")
    ((index += 1))
  done
  docker run --rm --network none "${docker_args[@]}" "${mounts[@]}" "$image" falco "${arguments[@]}"
}

validate_syscall() {
  run_validate /etc/falco/falco_rules.yaml -- "$@"
}

validate_k8saudit() {
  local plugin_dir config
  plugin_dir=$(k8saudit_plugin_dir) || return 1
  config=$(k8saudit_config "")
  trap 'rm -f -- "$config"' RETURN
  run_validate "" -v "$plugin_dir:/baukit-plugins:ro" -v "$config:/etc/falco/config.d/zz-k8saudit.yaml:ro" -- "$@"
}

# Writes a config.d file that loads the k8saudit plugin with OPEN_PARAMS, and prints its path.
k8saudit_config() {
  local config
  config=$(mktemp)
  printf '%s\n' 'load_plugins: [k8saudit]' 'plugins:' '  - name: k8saudit' \
    '    library_path: /baukit-plugins/libk8saudit.so' "    open_params: \"$1\"" >"$config"
  chmod 0644 "$config"
  printf '%s\n' "$config"
}

replay_k8saudit() {
  local events plugin_dir config rules
  events=$(cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")
  plugin_dir=$(k8saudit_plugin_dir) || return 1
  config=$(k8saudit_config /baukit-events/audit.jsonl)
  trap 'rm -f -- "$config"' RETURN
  rules=$falco_dir/k8saudit-rules/baukit.k8saudit.rules.yaml
  docker run --rm --network none -v "$plugin_dir:/baukit-plugins:ro" \
    -v "$config:/etc/falco/config.d/zz-k8saudit.yaml:ro" -v "$events:/baukit-events/audit.jsonl:ro" \
    -v "$rules:/baukit-rules/baukit.k8saudit.rules.yaml:ro" "$image" \
    falco -r /baukit-rules/baukit.k8saudit.rules.yaml -o json_output=true -o stdout_output.enabled=true \
    -o log_stderr=false -o plugins_hostinfo=false --disable-source syscall |
    python3 -c '
import json, sys
for line in sys.stdin:
    if line.startswith("{"):
        alert = json.loads(line)
        fields = alert.get("output_fields", {})
        print("\t".join([alert["rule"], *(str(fields.get(key, "")) for key in sys.argv[1:])]))
' ka.user.name ka.target.namespace ka.target.subresource
}

if (($# == 0)); then
  validate_syscall "$falco_dir"/rules/*.yaml "$falco_dir"/../components/falco-shared-kernel/*.rules.yaml
  validate_k8saudit "$falco_dir"/k8saudit-rules/*.yaml
elif [[ $1 == --k8saudit-replay ]]; then
  replay_k8saudit "$2"
elif [[ $1 == --k8saudit ]]; then
  shift
  validate_k8saudit "$@"
else
  validate_syscall "$@"
fi

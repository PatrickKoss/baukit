#!/usr/bin/env bash
set -euo pipefail

dry_run=false
if [[ ${1:-} == --dry-run && $# == 1 ]]; then
    dry_run=true
elif [[ $# != 0 ]]; then
    printf 'Usage: %s [--dry-run]\n' "$0" >&2
    exit 2
fi

if [[ ${DOCKER_HOST+x} && ${DOCKER_HOST:-} != unix://* ]]; then
    printf 'Skipping cleanup: DOCKER_HOST is not a unix socket; local PIDs cannot identify remote containers.\n' >&2
    exit 0
fi

if [[ ! -d /proc/self ]]; then
    printf 'This script requires the local Linux /proc filesystem.\n' >&2
    exit 1
fi

if ! pid_namespace=$(readlink /proc/self/ns/pid 2>/dev/null) || [[ -z $pid_namespace ]]; then
    printf 'Skipping cleanup: the local PID namespace is unreadable.\n' >&2
    exit 0
fi
if ! hostname=$(cat /proc/sys/kernel/hostname 2>/dev/null) || [[ -z $hostname ]]; then
    if ! hostname=$(cat /etc/hostname 2>/dev/null) || [[ -z $hostname ]]; then
        printf 'Skipping cleanup: the local host name is unreadable.\n' >&2
        exit 0
    fi
fi

is_orphan() {
    local pid=$1 binary=$2 executable
    [[ $pid =~ ^[1-9][0-9]*$ ]] || return 1
    [[ -d /proc/$pid ]] || return 0
    [[ -n $binary ]] || return 1
    if ! executable=$(readlink "/proc/$pid/exe"); then
        [[ ! -d /proc/$pid ]]
        return
    fi
    [[ ${executable##*/} != "$binary" ]] || return 1
    executable=${executable%" (deleted)"}
    [[ ${executable##*/} != "$binary" ]]
}

container_age() {
    local started=$1
    if [[ $started =~ ^[0-9]{1,10}$ ]] && (( now >= 10#$started )); then
        printf '%ss' "$((now - 10#$started))"
    else
        printf 'unknown'
    fi
}

ids=$(docker ps --all --quiet --filter label=dev.baukit.test=true)
[[ -n $ids ]] || exit 0
now=$(date +%s)
format='{{printf "%s\x1f%s\x1f%s\x1f%s\x1f%s\x1f%s\x1f%s\x1f%s" (index .Config.Labels "dev.baukit.test") (index .Config.Labels "dev.baukit.test.pid") (index .Config.Labels "dev.baukit.test.binary") (index .Config.Labels "dev.baukit.test.manifest-dir") (index .Config.Labels "dev.baukit.test.started-at") .Config.Image (index .Config.Labels "dev.baukit.test.pid-ns") (index .Config.Labels "dev.baukit.test.host")}}'

while IFS= read -r id; do
    if ! details=$(docker inspect --format "$format" "$id" 2>&1); then
        remaining=$(docker ps --all --quiet --filter label=dev.baukit.test=true --filter "id=$id")
        [[ -n $remaining ]] || continue
        printf 'Could not inspect test container %s: %s\n' "$id" "$details" >&2
        exit 1
    fi
    IFS=$'\x1f' read -r marker pid binary manifest started image namespace host <<< "$details"
    [[ $marker == true ]] || continue
    [[ $namespace == "$pid_namespace" && $host == "$hostname" ]] || continue
    is_orphan "$pid" "$binary" || continue
    printf '%s image=%s manifest-dir=%s binary=%s age=%s\n' \
        "$id" "$image" "$manifest" "$binary" "$(container_age "$started")"
    if [[ $dry_run == false ]]; then
        if removed=$(docker rm --force --volumes "$id" 2>&1); then
            printf '%s\n' "$removed"
        else
            remaining=$(docker ps --all --quiet --filter label=dev.baukit.test=true --filter "id=$id")
            [[ -n $remaining ]] || continue
            printf 'Could not remove test container %s: %s\n' "$id" "$removed" >&2
            exit 1
        fi
    fi
done <<< "$ids"

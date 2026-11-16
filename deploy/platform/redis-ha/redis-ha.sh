#!/bin/sh
# Usage: redis-ha.sh server|sentinel
set -eu

master_name="${REDIS_MASTER_NAME:-mymaster}"
peer_count="${REDIS_PEER_COUNT:-3}"
headless="${REDIS_HEADLESS:-redis-headless}"
namespace="${POD_NAMESPACE:-platform-redis}"
cluster_domain="${CLUSTER_DOMAIN:-cluster.local}"
sentinel_conf="${SENTINEL_CONF:-/sentinel/sentinel.conf}"
no_answer_attempts="${NO_ANSWER_ATTEMPTS:-3}"
local_wait_attempts="${LOCAL_WAIT_ATTEMPTS:-24}"
retry_seconds="${RETRY_SECONDS:-5}"
down_after_ms=5000
failover_timeout_ms=30000
# Redis Sentinel holds TILT mode for a fixed 30 s after its timer stalls; it neither votes nor fails over then.
sentinel_tilt_ms=30000
# A Sentinel in TILT detects the dead master only when TILT ends, and after a split vote it retries the
# failover only twice the failover timeout after the first try. A restarted master that is still reported
# as master waits for both and the promotion, because starting as master without persistence would make
# the replicas resync to an empty data set.
self_wait_seconds="${SELF_WAIT_SECONDS:-$(((down_after_ms + sentinel_tilt_ms + 3 * failover_timeout_ms) / 1000))}"
poll_seconds=$((retry_seconds > 0 ? retry_seconds : 1))
self_wait_attempts=$((self_wait_seconds / poll_seconds + 1))

pod_name="${POD_NAME:-$(hostname)}"
statefulset="${pod_name%-*}"
ordinal="${pod_name##*-}"

fqdn() {
  printf '%s-%s.%s.%s.svc.%s' "$statefulset" "$1" "$headless" "$namespace" "$cluster_domain"
}

self="$(fqdn "$ordinal")"
bootstrap="$(fqdn 0)"

log() {
  printf 'redis-ha: %s\n' "$*" >&2
}

ask_sentinel() {
  timeout 3 redis-cli -t 2 -h "$1" -p 26379 --raw \
    SENTINEL get-master-addr-by-name "$master_name" 2>/dev/null | head -n 1
}

# Prints one line per distinct master that a peer Sentinel reports.
reported_masters() {
  i=0
  while [ "$i" -lt "$peer_count" ]; do
    if [ "$i" != "$ordinal" ]; then
      answer="$(ask_sentinel "$(fqdn "$i")" || true)"
      case "$answer" in
        "" | *ERR* | *IDONTKNOW*) ;;
        *) printf '%s\n' "$answer" ;;
      esac
    fi
    i=$((i + 1))
  done | sort -u
}

# Prints the master this pod should follow, or "self" to start as master.
# Arguments: attempts while peers report no master, attempts while they report this pod or disagree.
decide_master() {
  attempt=1
  while :; do
    masters="$(reported_masters)"
    count="$(printf '%s' "$masters" | grep -c . || true)"
    if [ "$count" = "0" ] && [ "$attempt" -lt "$1" ]; then
      log "no Sentinel answered yet (attempt $attempt)"
      attempt=$((attempt + 1))
      sleep "$retry_seconds"
      continue
    fi
    if [ "$count" = "0" ]; then
      if [ "$ordinal" = "0" ]; then
        log "no Sentinel answered; bootstrapping as master"
        echo self
      else
        log "no Sentinel answered; following $bootstrap"
        echo "$bootstrap"
      fi
      return 0
    fi
    if [ "$count" = "1" ] && [ "$masters" != "$self" ]; then
      log "Sentinels report master $masters"
      echo "$masters"
      return 0
    fi
    if [ "$attempt" -ge "$2" ]; then
      if [ "$masters" = "$self" ]; then
        log "Sentinels still agree this pod is master; starting as master"
        echo self
        return 0
      fi
      log "Sentinels disagree on the master: $(printf '%s' "$masters" | tr '\n' ' ')"
      return 1
    fi
    log "waiting for Sentinels to agree on a master other than this pod (attempt $attempt)"
    attempt=$((attempt + 1))
    sleep "$retry_seconds"
  done
}

run_server() {
  master="$(decide_master "$no_answer_attempts" "$self_wait_attempts")"
  set -- --save "" --appendonly no --maxmemory 48mb --maxmemory-policy allkeys-lru \
    --replica-announce-ip "$self"
  if [ "$master" != "self" ]; then
    set -- "$@" --replicaof "$master" 6379
  fi
  exec redis-server "$@"
}

local_redis() {
  timeout 3 redis-cli -t 2 -h 127.0.0.1 -p 6379 --raw "$@" 2>/dev/null
}

# Prints the master the local Redis chose once it answers, so Sentinel never monitors a guess.
local_master() {
  attempt=1
  while [ "$attempt" -le "$local_wait_attempts" ]; do
    info="$(local_redis INFO replication || true)"
    case "$info" in
      *role:master*)
        echo "$self"
        return 0
        ;;
      *master_host:*)
        printf '%s\n' "$info" | sed -n 's/^master_host:\([^[:space:]]*\).*/\1/p'
        return 0
        ;;
    esac
    attempt=$((attempt + 1))
    sleep "$retry_seconds"
  done
  log "local Redis did not answer; asking peer Sentinels"
  master="$(decide_master 1 1 || echo "$bootstrap")"
  if [ "$master" = "self" ]; then
    master="$self"
  fi
  echo "$master"
}

run_sentinel() {
  master="$(local_master)"
  printf '%s\n' \
    'port 26379' \
    'dir /tmp' \
    'sentinel resolve-hostnames yes' \
    'sentinel announce-hostnames yes' \
    "sentinel announce-ip $self" \
    "sentinel monitor $master_name $master 6379 2" \
    "sentinel down-after-milliseconds $master_name $down_after_ms" \
    "sentinel failover-timeout $master_name $failover_timeout_ms" \
    "sentinel parallel-syncs $master_name 1" \
    >"$sentinel_conf"
  exec redis-sentinel "$sentinel_conf"
}

case "${1:-}" in
  server) run_server ;;
  sentinel) run_sentinel ;;
  decide) decide_master "$no_answer_attempts" "$self_wait_attempts" ;;
  *)
    log "usage: redis-ha.sh server|sentinel|decide"
    exit 2
    ;;
esac

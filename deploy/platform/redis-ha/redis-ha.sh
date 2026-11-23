#!/bin/sh
# Usage: redis-ha.sh server|sentinel|acl-reload
set -eu

master_name="${REDIS_MASTER_NAME:-mymaster}"
peer_count="${REDIS_PEER_COUNT:-3}"
headless="${REDIS_HEADLESS:-redis-headless}"
namespace="${POD_NAMESPACE:-platform-redis}"
cluster_domain="${CLUSTER_DOMAIN:-cluster.local}"
sentinel_conf="${SENTINEL_CONF:-/sentinel/sentinel.conf}"
# Set by components/redis-ha-acl to the mounted redis-ha-acl Secret; empty keeps Redis unauthenticated.
acl_dir="${REDIS_ACL_DIR:-}"
acl_conf="${REDIS_ACL_CONF:-/tmp/redis-acl.conf}"
# Redis loads this copy of users.acl with the reload user appended; the Secret mount stays read-only.
acl_file="${REDIS_ACL_FILE:-/tmp/redis-users.acl}"
acl_reload_token="${REDIS_ACL_RELOAD_TOKEN:-/tmp/redis-acl-reload.token}"
acl_reload_user=baukit-acl-reload
acl_poll_seconds="${ACL_POLL_SECONDS:-5}"
# A users.acl that Redis rejected is retried this often, so redis_commands_failed_calls_total keeps rising.
acl_retry_seconds="${ACL_RETRY_SECONDS:-60}"
# 0 polls forever; tests set a bound.
acl_poll_limit="${ACL_POLL_LIMIT:-0}"
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

# kube-router rejects a new pod's traffic until it programs the pod IP, which took over 15 s on CI runners.
# redis-0 starting as master while the peers are unreachable would make them resync to an empty data set,
# so it waits longer than the other pods, which only pick a master to follow.
bootstrap_wait_seconds="${BOOTSTRAP_WAIT_SECONDS:-60}"
if [ "$ordinal" = "0" ]; then
  default_no_answer_attempts=$((bootstrap_wait_seconds / poll_seconds + 1))
else
  default_no_answer_attempts=3
fi
no_answer_attempts="${NO_ANSWER_ATTEMPTS:-$default_no_answer_attempts}"

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

acl_secret() {
  cat "$acl_dir/$1"
}

# Creates the pod-local password of the reload user. Only the redis container creates it, at start, on the
# pod's tmp volume, so it survives a container restart and the acl-reload sidecar reads the same one.
ensure_reload_token() {
  [ ! -s "$acl_reload_token" ] || return 0
  (
    umask 077
    head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$acl_reload_token.$$"
  )
  mv "$acl_reload_token.$$" "$acl_reload_token"
}

# Copies users.acl from the Secret and appends the reload user, which may run ACL LOAD and nothing else.
# Appending keeps the line numbers in Redis errors equal to those of the Secret's users.acl.
write_acl_file() {
  digest="$(sha256sum "$acl_reload_token" | cut -d ' ' -f 1)"
  # mktemp, not $$: the redis and acl-reload containers each run this script as PID 1.
  staged="$(mktemp "$acl_file.XXXXXX")"
  {
    awk 1 "$acl_dir/users.acl"
    printf 'user %s on #%s resetkeys resetchannels -@all +acl|load\n' "$acl_reload_user" "$digest"
  } >"$staged"
  mv "$staged" "$acl_file"
}

# Writes the ACL file path and replication credentials to a private config file, so the password never
# appears on the redis-server command line.
write_acl_conf() {
  ensure_reload_token
  write_acl_file
  (
    umask 077
    printf '%s\n' \
      "aclfile $acl_file" \
      'masteruser replication' \
      "masterauth $(acl_secret replication-password)" \
      >"$acl_conf"
  )
}

run_server() {
  master="$(decide_master "$no_answer_attempts" "$self_wait_attempts")"
  set -- --save "" --appendonly no --maxmemory 48mb --maxmemory-policy allkeys-lru \
    --replica-announce-ip "$self"
  if [ "$master" != "self" ]; then
    set -- "$@" --replicaof "$master" 6379
  fi
  if [ -n "$acl_dir" ]; then
    write_acl_conf
    set -- "$acl_conf" "$@"
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
  if [ -n "$acl_dir" ]; then
    printf '%s\n' \
      "sentinel auth-user $master_name sentinel" \
      "sentinel auth-pass $master_name $(acl_secret sentinel-password)" \
      >>"$sentinel_conf"
  fi
  exec redis-sentinel "$sentinel_conf"
}

acl_checksum() {
  sha256sum "$acl_dir/users.acl" | cut -d ' ' -f 1
}

# Redis quotes the offending token in its errors, which may be a password; keep only file, line and reason.
redacted_acl_error() {
  printf '%s' "$1" | tr '\n' ' ' |
    sed -e "s|$acl_file|users.acl|g" -e "s/'[^']*'/'<redacted>'/g" -e 's/ WARNING: .*//'
}

# Returns 0 when Redis loaded the file, 1 when it rejected it and kept the previous users, 2 when it did
# not answer or has not started yet. Sets reply to the redacted answer.
load_acl() {
  reply="redis has not created $acl_reload_token yet"
  [ -s "$acl_reload_token" ] || return 2
  write_acl_file
  answer="$(REDISCLI_AUTH="$(cat "$acl_reload_token")" timeout 5 redis-cli -t 3 -h 127.0.0.1 -p 6379 \
    --no-auth-warning --user "$acl_reload_user" ACL LOAD 2>&1 || true)"
  reply="$(redacted_acl_error "$answer")"
  case "$answer" in
    OK) return 0 ;;
    ERR*) return 1 ;;
    *) return 2 ;;
  esac
}

# Loads users.acl into the local Redis whenever the mounted Secret changes. The kubelet swaps the files of a
# Secret volume in place, unless it is mounted with subPath.
run_acl_reload() {
  if [ -z "$acl_dir" ]; then
    log "acl-reload needs REDIS_ACL_DIR"
    exit 2
  fi
  loaded=""
  attempted=""
  retry_at=0
  unanswered=""
  polls=0
  while [ "$acl_poll_limit" = 0 ] || [ "$polls" -lt "$acl_poll_limit" ]; do
    polls=$((polls + 1))
    sum="$(acl_checksum)"
    now="$(date +%s)"
    if [ "$sum" != "$loaded" ] && { [ "$sum" != "$attempted" ] || [ "$now" -ge "$retry_at" ]; }; then
      status=0
      load_acl || status=$?
      case "$status" in
        0)
          loaded="$sum"
          unanswered=""
          log "loaded users.acl sha256 $(printf '%.12s' "$sum")"
          ;;
        1)
          attempted="$sum"
          retry_at=$((now + acl_retry_seconds))
          log "ACL LOAD rejected users.acl sha256 $(printf '%.12s' "$sum"), Redis keeps the previous users: $reply"
          ;;
        *)
          [ "$reply" = "$unanswered" ] || log "ACL LOAD not done, retrying: $reply"
          unanswered="$reply"
          ;;
      esac
    fi
    sleep "$acl_poll_seconds"
  done
}

case "${1:-}" in
  server) run_server ;;
  sentinel) run_sentinel ;;
  acl-reload) run_acl_reload ;;
  decide) decide_master "$no_answer_attempts" "$self_wait_attempts" ;;
  *)
    log "usage: redis-ha.sh server|sentinel|acl-reload|decide"
    exit 2
    ;;
esac

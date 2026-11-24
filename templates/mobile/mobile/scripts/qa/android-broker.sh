# shellcheck shell=bash
# Sourced by android-env.sh. With BAUKIT_QA_DEVICE_BROKER set to a pruefstand URL, the QA
# environment leases emulators from the broker instead of booting local AVDs. With
# BAUKIT_QA_DEVICE_PORT_FORWARD set to a kube context, the lease runs in the background with
# --port-forward and releases its leases when it gets SIGTERM.

broker_leases="${state_dir:?}/android-leases"
broker_heartbeat_pid="${state_dir:?}/android-heartbeat.pid"
forward_pid="${state_dir:?}/android-forward.pid"
forward_output="${state_dir:?}/android-forward.out"
forward_log="${state_dir:?}/android-forward.log"
forward_watchdog_pid="${state_dir:?}/android-forward-watchdog.pid"
forward_poll_seconds=1
forward_watchdog_seconds=2
forward_release_seconds=60
seconds_per_minute=60
seconds_per_hour=3600

broker_enabled() { [[ -n "${BAUKIT_QA_DEVICE_BROKER:-}" ]]; }

broker_cli() {
  PRUEFSTAND_URL="$BAUKIT_QA_DEVICE_BROKER" pruefstand "$@"
}

broker_lease_ids() {
  [[ -f "$broker_leases" ]] || return 0
  awk '{ print $2 }' "$broker_leases"
}

broker_port_forward() { [[ -n "${BAUKIT_QA_DEVICE_PORT_FORWARD:-}" ]]; }

kill_pid_file() {
  [[ -f "$1" ]] || return 0
  kill "$(<"$1")" 2>/dev/null || true
  rm -f "$1"
}

stop_broker_heartbeat() { kill_pid_file "$broker_heartbeat_pid"; }

process_alive() {
  local state
  state="$(ps -o stat= -p "$1" 2>/dev/null)" || return 1
  [[ "$state" != Z* ]]
}

duration_seconds() {
  [[ "$1" =~ ^([0-9]+)([smh]?)$ ]] || { echo "qa: cannot read the duration $1" >&2; return 2; }
  local count="${BASH_REMATCH[1]}"
  case "${BASH_REMATCH[2]}" in
    m) echo $((count * seconds_per_minute)) ;;
    h) echo $((count * seconds_per_hour)) ;;
    *) echo "$count" ;;
  esac
}

await_process_exit() {
  local deadline=$((SECONDS + forward_release_seconds))
  while process_alive "$1"; do
    ((SECONDS < deadline)) || return 1
    sleep "$forward_poll_seconds"
  done
}

stop_forwarding_lease() {
  local pid
  [[ -f "$forward_pid" ]] || return 0
  pid="$(<"$forward_pid")"
  if ! process_alive "$pid"; then
    echo "qa: the forwarding pruefstand lease had already exited; its leases are released or expire after their TTL"
  else
    kill -TERM "$pid" 2>/dev/null || true
    await_process_exit "$pid" || {
      echo "qa: the forwarding pruefstand lease $pid did not exit within ${forward_release_seconds}s; the broker reclaims its leases after their TTL" >&2
      return 1
    }
    echo "qa: the forwarding pruefstand lease released its devices"
  fi
  rm -f "$forward_pid"
  kill_pid_file "$forward_watchdog_pid"
}

release_by_broker() {
  local -a ids
  mapfile -t ids < <(broker_lease_ids)
  [[ -n "${ids[*]}" ]] || return 0
  broker_cli release "${ids[@]}" || {
    echo "qa: could not release device leases ${ids[*]}; the broker reclaims them after their TTL" >&2
    return 1
  }
  echo "qa: released device leases ${ids[*]}"
}

release_leased_devices() {
  stop_broker_heartbeat
  if [[ -f "$broker_leases" ]]; then
    local serial
    while read -r serial _; do
      adb_bounded disconnect "$serial" >/dev/null 2>&1 || true
    done <"$broker_leases"
  fi
  if [[ -f "$forward_pid" ]]; then
    stop_forwarding_lease || return
  else
    [[ -f "$broker_leases" ]] || return 0
    release_by_broker || return
  fi
  rm -f "$broker_leases"
}

release_leases_on_failure() {
  local status=$?
  trap - EXIT
  ((status == 0)) && return 0
  release_leased_devices || true
  rm -f "$state_dir/android-serial"
  exit "$status"
}

start_broker_heartbeat() {
  local owner="${BAUKIT_QA_DEVICE_OWNER_PID:-$PPID}" interval="${BAUKIT_QA_DEVICE_HEARTBEAT_SECONDS:-60}"
  local -a ids
  [[ "$interval" =~ ^[1-9][0-9]*$ ]] || { echo "qa: BAUKIT_QA_DEVICE_HEARTBEAT_SECONDS must be a positive integer" >&2; return 2; }
  mapfile -t ids < <(broker_lease_ids)
  (
    close_inherited_fds
    while kill -0 "$owner" 2>/dev/null; do
      sleep "$interval"
      broker_cli heartbeat "${ids[@]}" >/dev/null || exit
    done
  ) </dev/null >>"$state_dir/android-heartbeat.log" 2>&1 &
  printf '%s\n' "$!" >"$broker_heartbeat_pid"
}

start_owner_watchdog() {
  local owner="${BAUKIT_QA_DEVICE_OWNER_PID:-$PPID}" lease
  lease="$(<"$forward_pid")"
  (
    close_inherited_fds
    while kill -0 "$lease" 2>/dev/null; do
      kill -0 "$owner" 2>/dev/null || { kill -TERM "$lease" 2>/dev/null; exit; }
      sleep "$forward_watchdog_seconds"
    done
  ) </dev/null >>"$state_dir/android-heartbeat.log" 2>&1 &
  printf '%s\n' "$!" >"$forward_watchdog_pid"
}

start_forwarding_lease() {
  : >"$forward_output"
  (
    close_inherited_fds
    PRUEFSTAND_URL="$BAUKIT_QA_DEVICE_BROKER" exec pruefstand lease "$@" --port-forward "$BAUKIT_QA_DEVICE_PORT_FORWARD"
  ) </dev/null >"$forward_output" 2>"$forward_log" &
  printf '%s\n' "$!" >"$forward_pid"
}

forwarded_serial_count() { grep -c 'ANDROID_SERIAL=' "$forward_output" || true; }

await_forwarded_serials() {
  local pid status=0 limit start="${BAUKIT_QA_DEVICE_PORT_FORWARD_START_SECONDS:-120}"
  pid="$(<"$forward_pid")"
  limit="$(duration_seconds "${BAUKIT_QA_DEVICE_LEASE_WAIT:-10m}")" || exit
  local deadline=$((SECONDS + limit + start))
  while (($(forwarded_serial_count) < emulator_count)); do
    if ! process_alive "$pid"; then
      wait "$pid" || status=$?
      rm -f "$forward_pid"
      cat "$forward_log" >&2
      return "$status"
    fi
    if ((SECONDS >= deadline)); then
      echo "qa: pruefstand lease --port-forward printed no $emulator_count serials within ${limit}s plus ${start}s" >&2
      return 1
    fi
    sleep "$forward_poll_seconds"
  done
}

lease_devices() {
  local output status=0 line serial id leased=0
  local -a extra flags
  read -ra extra <<<"${BAUKIT_QA_DEVICE_BROKER_ARGS:-}"
  mkdir -p "$state_dir"
  trap release_leases_on_failure EXIT
  flags=(
    --api "${BAUKIT_QA_ANDROID_API_LEVEL:-36}"
    --count "${emulator_count:?}"
    --ttl "${BAUKIT_QA_DEVICE_LEASE_TTL:-30m}"
    --wait "${BAUKIT_QA_DEVICE_LEASE_WAIT:-10m}"
    "${extra[@]}"
  )
  if broker_port_forward; then
    start_forwarding_lease "${flags[@]}"
    await_forwarded_serials || status=$?
    output="$(<"$forward_output")"
  else
    output="$(broker_cli lease "${flags[@]}")" || status=$?
  fi
  if ((status == 3)); then
    echo "qa: the device broker had no $emulator_count free emulators within ${BAUKIT_QA_DEVICE_LEASE_WAIT:-10m}" >&2
    broker_port_forward && echo "qa: BAUKIT_QA_DEVICE_PORT_FORWARD=$BAUKIT_QA_DEVICE_PORT_FORWARD leased through kubectl port-forward" >&2
    exit 3
  fi
  ((status == 0)) || { echo "qa: pruefstand lease failed with status $status" >&2; exit "$status"; }
  : >"$broker_leases"
  serials=()
  while read -r line; do
    serial="$(sed -n 's/.*ANDROID_SERIAL=\([^ ]*\).*/\1/p' <<<"$line")"
    id="$(sed -n 's/.*LEASE_ID=\([^ ]*\).*/\1/p' <<<"$line")"
    [[ -n "$serial" && -n "$id" ]] || continue
    printf '%s %s\n' "$serial" "$id" >>"$broker_leases"
    serials+=("$serial")
    leased=$((leased + 1))
  done <<<"$output"
  ((leased == emulator_count)) || {
    echo "qa: pruefstand lease returned $leased devices, expected $emulator_count" >&2
    exit 1
  }
  serial="${serials[0]}"
  printf '%s\n' "${serials[@]}" >"$state_dir/android-serial"
  echo "qa: leased ${serials[*]} from the device broker"
  if broker_port_forward; then
    start_owner_watchdog
  else
    start_broker_heartbeat || exit
  fi
  for line in "${serials[@]}"; do
    adb_checked connect "$line" || exit
    await_emulator "$line"
  done
}

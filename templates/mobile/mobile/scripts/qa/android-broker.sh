# shellcheck shell=bash
# Sourced by android-env.sh. With BAUKIT_QA_DEVICE_BROKER set to a pruefstand URL, the QA
# environment leases emulators from the broker instead of booting local AVDs.

broker_leases="${state_dir:?}/android-leases"
broker_heartbeat_pid="${state_dir:?}/android-heartbeat.pid"

broker_enabled() { [[ -n "${BAUKIT_QA_DEVICE_BROKER:-}" ]]; }

broker_cli() {
  PRUEFSTAND_URL="$BAUKIT_QA_DEVICE_BROKER" pruefstand "$@"
}

broker_lease_ids() {
  [[ -f "$broker_leases" ]] || return 0
  awk '{ print $2 }' "$broker_leases"
}

stop_broker_heartbeat() {
  [[ -f "$broker_heartbeat_pid" ]] || return 0
  kill "$(<"$broker_heartbeat_pid")" 2>/dev/null || true
  rm -f "$broker_heartbeat_pid"
}

release_leased_devices() {
  local -a ids
  stop_broker_heartbeat
  [[ -f "$broker_leases" ]] || return 0
  mapfile -t ids < <(broker_lease_ids)
  local serial
  while read -r serial _; do
    adb_bounded disconnect "$serial" >/dev/null 2>&1 || true
  done <"$broker_leases"
  if [[ -n "${ids[*]}" ]]; then
    broker_cli release "${ids[@]}" || {
      echo "qa: could not release device leases ${ids[*]}; the broker reclaims them after their TTL" >&2
      return 1
    }
    echo "qa: released device leases ${ids[*]}"
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

lease_devices() {
  local output status=0 line serial id leased=0
  local -a extra
  read -ra extra <<<"${BAUKIT_QA_DEVICE_BROKER_ARGS:-}"
  mkdir -p "$state_dir"
  trap release_leases_on_failure EXIT
  output="$(broker_cli lease \
    --api "${BAUKIT_QA_ANDROID_API_LEVEL:-36}" \
    --count "${emulator_count:?}" \
    --ttl "${BAUKIT_QA_DEVICE_LEASE_TTL:-30m}" \
    --wait "${BAUKIT_QA_DEVICE_LEASE_WAIT:-10m}" \
    "${extra[@]}")" || status=$?
  if ((status == 3)); then
    echo "qa: the device broker had no $emulator_count free emulators within ${BAUKIT_QA_DEVICE_LEASE_WAIT:-10m}" >&2
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
  start_broker_heartbeat || exit
  for line in "${serials[@]}"; do
    adb_checked connect "$line" || exit
    await_emulator "$line"
  done
}

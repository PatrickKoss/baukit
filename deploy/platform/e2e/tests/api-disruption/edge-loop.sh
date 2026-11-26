#!/bin/sh
# Runs inside the loader pod. Requests EDGE_HOST through Traefik once per interval for LOAD_SECONDS,
# after the first success, and prints one line per failed request and a final summary line.
set -u

url=https://$EDGE_HOST/
expected_body='baukit e2e echo'
request_timeout_seconds=2
interval_seconds=0.1

request() {
  wget -q -T "$request_timeout_seconds" -O - --no-check-certificate "$url" 2>&1
}

warm_deadline=$(($(date +%s) + WARM_SECONDS))
until body=$(request) && [ "$body" = "$expected_body" ]; do
  if [ "$(date +%s)" -ge "$warm_deadline" ]; then
    echo "never reached $url: $body"
    exit 1
  fi
  sleep 1
done
echo warm

ok=0
failed=0
end=$(($(date +%s) + LOAD_SECONDS))
while [ "$(date +%s)" -lt "$end" ]; do
  if body=$(request) && [ "$body" = "$expected_body" ]; then
    ok=$((ok + 1))
  else
    failed=$((failed + 1))
    echo "$(date +%T) failed: $body"
  fi
  sleep "$interval_seconds"
done
echo "summary ok=$ok failed=$failed"

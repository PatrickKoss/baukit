#!/bin/sh
# Serves a built web client with `vite preview` and checks its security headers.
# vite runs without the pnpm wrapper, which does not forward signals, so the EXIT trap stops it.
set -eu

web=${1:?usage: check-web-headers.sh WEB_DIR}
port=${WEB_HEADERS_PORT:-4173}
headers=$(mktemp)
log=$(mktemp)
(cd "$web" && exec node node_modules/vite/bin/vite.js preview --port "$port" --strictPort) >"$log" 2>&1 &
preview=$!
trap 'kill "$preview" 2>/dev/null || true; rm -f "$headers" "$log"' EXIT

attempt=0
until curl -sfI "http://localhost:$port/" >"$headers"; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 100 ] || ! kill -0 "$preview" 2>/dev/null; then
        cat "$log" >&2
        echo "vite preview did not serve $web" >&2
        exit 1
    fi
    sleep 0.2
done

policy=$(tr -d '\r' <"$headers" | sed -n 's/^[Cc]ontent-[Ss]ecurity-[Pp]olicy: //p')
expected=$(tr -d '\n' <"$web/dist/csp.txt")
fail() {
    cat "$headers" >&2
    echo "$1" >&2
    exit 1
}
[ -n "$policy" ] || fail "no Content-Security-Policy header"
[ "$policy" = "$expected" ] || fail "served policy differs from dist/csp.txt"
script_src=$(printf '%s' "$policy" | tr ';' '\n' | sed -n 's/^ *script-src //p')
case " $script_src " in
    *" 'self' "*) ;;
    *) fail "script-src lacks 'self'" ;;
esac
case "$script_src" in
    *unsafe-inline* | *unsafe-eval*) fail "script-src allows unsafe sources" ;;
esac
tr -d '\r' <"$headers" | grep -qi '^x-content-type-options: nosniff$' || fail "no X-Content-Type-Options: nosniff"
echo "web headers: $policy"

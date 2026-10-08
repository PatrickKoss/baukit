#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"

tree=$(cargo tree --manifest-path rust/Cargo.toml -p baukit-suite --no-default-features -e normal --prefix none)
if grep -E '^(sqlx(-[^ ]+)?|tokio(-[^ ]+)?|reqwest|axum(-[^ ]+)?|utoipa(-[^ ]+)?|baukit-(jobs|http|egress|ratelimit|runtime|erasure|credential-vault)) v' <<< "$tree"; then
  echo "baukit-suite domain build includes runtime dependencies" >&2
  exit 1
fi

for feature in none postgres runtime delivery http jobs all; do
  args=(--manifest-path rust/Cargo.toml -p baukit-suite --lib --no-default-features)
  case "$feature" in
    none) ;;
    all) args+=(--all-features) ;;
    *) args+=(--features "$feature") ;;
  esac
  cargo check "${args[@]}"
  cargo clippy "${args[@]}" -- -D warnings
done

cargo check --manifest-path rust/Cargo.toml -p baukit-test --features suite --lib

# A separate consumer keeps the suite's test adapters from enabling runtime features.
consumer=$(mktemp -d)
cleanup() {
  if [[ -f "$consumer/Cargo.toml" ]]; then
    cargo clean --manifest-path "$consumer/Cargo.toml" --target-dir "$repo_root/rust/target" -p suite-domain-consumer
  fi
  rm -rf "$consumer"
}
trap cleanup EXIT
mkdir "$consumer/src"
cp scripts/fixtures/suite-domain.rs "$consumer/src/lib.rs"
python3 - "$repo_root" "$consumer" <<'PYTHON'
import json
import sys
from pathlib import Path

root, consumer = map(Path, sys.argv[1:])
(consumer / "Cargo.toml").write_text(
    '[package]\nname = "suite-domain-consumer"\nversion = "0.0.0"\nedition = "2024"\n'
    '[workspace]\n[dependencies]\n'
    f'baukit-suite = {{ path = {json.dumps(str(root / "rust/crates/baukit-suite"))}, default-features = false }}\n'
    'serde = { version = "1", features = ["derive"] }\nserde_json = "1"\n'
)
PYTHON
cp rust/Cargo.lock "$consumer/Cargo.lock"
cargo test --manifest-path "$consumer/Cargo.toml" --target-dir "$repo_root/rust/target" --offline
cargo clippy --manifest-path "$consumer/Cargo.toml" --target-dir "$repo_root/rust/target" --offline --all-targets -- -D warnings

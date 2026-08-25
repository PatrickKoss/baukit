#!/bin/sh
set -eu

cargo run --manifest-path backend/Cargo.toml -p {{ context.app_name }}-bin --bin openapi -- backend/openapi.json

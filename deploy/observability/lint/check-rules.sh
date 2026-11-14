#!/bin/sh
set -eu

observability=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
repo_root=$(CDPATH='' cd -- "$observability/../.." && pwd)
. "$repo_root/deploy/lib/tools.sh"
baukit_use_tools "$repo_root" promtool

promtool check rules "$observability/recording-rules/baukit-red.rules.yml" "$observability/alerts/baukit.rules.yml"
promtool test rules "$observability/tests/worker.rules.test.yml"

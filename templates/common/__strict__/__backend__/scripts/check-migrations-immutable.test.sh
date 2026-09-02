#!/bin/sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
guard="$repository_root/scripts/check-migrations-immutable.sh"
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT

create_repository() {
  case_name=$1
  case_repository="$test_root/$case_name"
  mkdir -p "$case_repository/backend/migrations"
  git -C "$case_repository" init --quiet
  git -C "$case_repository" config user.email "migration-guard@example.invalid"
  git -C "$case_repository" config user.name "Migration guard test"
  printf '%s\n' 'CREATE TABLE example (id integer);' \
    > "$case_repository/backend/migrations/20260101000000_example.sql"
  git -C "$case_repository" add backend/migrations
  git -C "$case_repository" commit --quiet -m "add migration"
  case_base=$(git -C "$case_repository" rev-parse HEAD)
}

expect_failure() {
  expected_path=$1
  failure_repository=$2
  failure_base=$3
  if failure_output=$(cd "$failure_repository" && sh "$guard" "$failure_base" HEAD 2>&1); then
    echo "expected migration guard to fail for $expected_path" >&2
    exit 1
  fi
  case "$failure_output" in
    *"$expected_path"*) ;;
    *)
      echo "migration guard did not identify $expected_path" >&2
      printf '%s\n' "$failure_output" >&2
      exit 1
      ;;
  esac
}

create_repository unchanged
(
  cd "$case_repository"
  sh "$guard" "$case_base" HEAD >/dev/null
)

create_repository added
printf '%s\n' 'CREATE TABLE another_example (id integer);' \
  > "$case_repository/backend/migrations/20260102000000_another_example.sql"
git -C "$case_repository" add backend/migrations
git -C "$case_repository" commit --quiet -m "add forward migration"
(
  cd "$case_repository"
  sh "$guard" "$case_base" HEAD >/dev/null
)

create_repository modified
migration_path=backend/migrations/20260101000000_example.sql
printf '%s\n' 'CREATE TABLE example (id bigint);' > "$case_repository/$migration_path"
git -C "$case_repository" add "$migration_path"
git -C "$case_repository" commit --quiet -m "modify migration"
expect_failure "$migration_path" "$case_repository" "$case_base"

create_repository deleted
migration_path=backend/migrations/20260101000000_example.sql
rm "$case_repository/$migration_path"
git -C "$case_repository" add "$migration_path"
git -C "$case_repository" commit --quiet -m "delete migration"
expect_failure "$migration_path" "$case_repository" "$case_base"

echo "migration immutability tests: passed"

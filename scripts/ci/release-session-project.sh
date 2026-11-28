#!/usr/bin/env bash
set -euo pipefail

# Keep the production kernel's release optimization and disabled debug assertions.
# Only the dev-only App fixture dependency needs its release guard enabled.
cargo_test=(cargo
  --config profile.release.package.chariox-kernel.codegen-units=256
  --config profile.release.package.chariox-app-runtime.debug-assertions=true
  test --release --locked -p chariox-kernel --test session_project_release)

phase() {
  local name=$1
  shift
  local started=$SECONDS status=0
  printf 'Release regression %s started at %s\n' "$name" "$(date -u +%FT%TZ)"
  /usr/bin/time -v "$@" || status=$?
  printf 'Release regression %s finished: status=%s elapsed=%ss\n' "$name" "$status" "$((SECONDS - started))"
  return "$status"
}

phase compile "${cargo_test[@]}" --no-run
# Compilation has already completed; this deadline applies only to test execution.
phase test timeout 60 "${cargo_test[@]}" -- --exact visible_session_project_assignment_survives_release_builds --nocapture

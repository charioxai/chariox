#!/bin/sh
# Runs a test command with a private TMPDIR and fails when the tests leave
# anything in it: every test removes the temp state it creates.
#   scripts/check-test-temp.sh cargo test -p chariox-kernel --lib <filter>
# CHARIOX_KEEP_TEST_TEMP=1 keeps test directories for debugging (the check then fails).
set -u
[ "$#" -gt 0 ] || { echo "usage: $0 <test command...>" >&2; exit 2; }
tmp=$(mktemp -d "${CHARIOX_TEST_TEMP_PARENT:-/var/tmp}/chariox-test-temp.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
TMPDIR=$tmp "$@"
status=$?
left=$(find "$tmp" -mindepth 1 -maxdepth 2 ! \( -path "$tmp/chariox-tests" -empty \) | head -50)
if [ -n "$left" ]; then
  echo "tests left state in TMPDIR (first 50 entries):" >&2
  echo "$left" | sed "s#^$tmp/##" >&2
  exit 1
fi
exit $status

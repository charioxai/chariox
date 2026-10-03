#!/usr/bin/env bash
# Focused private-state drill. Fixed-libc workers require no shared enrollment.
set -euo pipefail
[[ "$(uname -s)" == Linux ]] || { echo 'Run this drill on the Linux builder.' >&2; exit 1; }
command -v slot-run >/dev/null || { echo 'The builder slot-run admission helper is required.' >&2; exit 1; }
exec slot-run cargo +1.88.0 test --locked -p chariox-kernel --lib saved_snapshot_restore -- --test-threads=1

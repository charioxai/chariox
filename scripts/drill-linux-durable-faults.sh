#!/usr/bin/env bash
# Run prebuilt component tests as an ordinary user on a private <=128 MiB
# filesystem. Compile through builder admission separately; this script never
# starts a kernel/helper, mounts anything, or borrows installed App state.
set -euo pipefail
[[ $# == 4 && "$(id -u)" != 0 ]]
fault_binary="$(realpath -e "$1")"
fault_root="$(realpath -e "$2")"
fault_key="$(realpath -e "$3")"
fault_evidence="$(realpath -e "$4")"
[[ -f "$fault_binary" && -x "$fault_binary" && ! -L "$3" && "$(stat -c %a "$fault_key")" == 600 && "$(stat -c %s "$fault_key")" == 32 ]]
[[ "$fault_root" != "$fault_evidence" && "$fault_evidence/" != "$fault_root/"* && -z "$(ls -A "$fault_root")" ]]
export CHARIOX_FAULT_ROOT="$fault_root" CHARIOX_FAULT_KEY="$fault_key"
status=0
for fault_test in process_kill_matrix enospc_without_restart removed_route_keeps_accepted_occurrence; do
  set +e
  "$fault_binary" --ignored --exact "$fault_test" --nocapture --test-threads=1 > "$fault_evidence/$fault_test.log" 2>&1
  fault_exit=$?
  set -e
  printf '%s exit=%s\n' "$fault_test" "$fault_exit" | tee -a "$fault_evidence/results.txt"
  if [[ $fault_exit != 0 ]]; then status=1; fi
done
exit "$status"

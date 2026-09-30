#!/usr/bin/env bash
# ExecStartPost of chariox-kernel.service, as the kernel's ordinary user. The user
# counterpart of `chariox-app-storage --prepare-managed-domain`: enables cpu,
# memory and pids below the delegated unit and creates the empty App subtree
# `apps` (mode 0700) that /etc/chariox/app-storage.json names as the owner's
# cgroup root.
set -euo pipefail
self=$(sed -n 's/^0:://p' /proc/self/cgroup)
[[ "$self" == */chariox-kernel.service/.control ]] \
  || { echo "prepare-app-domain: expected the unit's .control cgroup, got $self" >&2; exit 1; }
unit=/sys/fs/cgroup${self%/.control}
[[ -O "$unit/cgroup.subtree_control" ]] || { echo "prepare-app-domain: $unit is not delegated to $(id -un)" >&2; exit 1; }
echo "+cpu +memory +pids" > "$unit/cgroup.subtree_control"
if [[ -d "$unit/apps" ]]; then
  grep -qx 'populated 0' "$unit/apps/cgroup.events" \
    || { echo "prepare-app-domain: $unit/apps still has processes" >&2; exit 1; }
  # Leaves an unclean stop left behind hold no processes; remove them (best
  # effort) so the kernel starts from a clean subtree.
  for leaf in "$unit/apps"/app-*; do [[ -d "$leaf" ]] && rmdir "$leaf" 2>/dev/null || true; done
else
  mkdir "$unit/apps"
fi
chmod 0700 "$unit/apps"
echo "+cpu +memory +pids" > "$unit/apps/cgroup.subtree_control"
echo "prepare-app-domain: $unit/apps ready ($(cat "$unit/apps/cgroup.subtree_control"))"

#!/usr/bin/env bash
# ExecStart of chariox-kernel.service, as the kernel's ordinary user:
#   start-kernel.sh KERNEL
# Prepares the App cgroup domain, then execs KERNEL in its place, so the domain
# exists before the kernel can start any App (it starts enabled Apps at once).
# The user counterpart of `chariox-app-storage --prepare-managed-domain`: enables
# cpu, memory and pids below the delegated unit and creates the empty App subtree
# `apps` (mode 0700) that /etc/chariox/app-storage.json names as the owner's
# cgroup root. systemd has already placed this process in the unit's `supervisor`
# subgroup, so the unit cgroup has no processes of its own. Enabling controllers
# there from an ExecStartPre instead makes systemd 259 fail to spawn the main
# process (EBUSY).
set -euo pipefail
[[ $# -eq 1 ]] || { echo "usage: start-kernel.sh KERNEL" >&2; exit 2; }
self=$(sed -n 's/^0:://p' /proc/self/cgroup)
[[ "$self" == */chariox-kernel.service/supervisor ]] \
  || { echo "start-kernel: expected the unit's supervisor cgroup, got $self" >&2; exit 1; }
unit=/sys/fs/cgroup${self%/supervisor}
[[ -O "$unit/cgroup.subtree_control" ]] || { echo "start-kernel: $unit is not delegated to $(id -un)" >&2; exit 1; }
echo "+cpu +memory +pids" > "$unit/cgroup.subtree_control"
if [[ -d "$unit/apps" ]]; then
  grep -qx 'populated 0' "$unit/apps/cgroup.events" \
    || { echo "start-kernel: $unit/apps still has processes" >&2; exit 1; }
  # Leaves an unclean stop left behind hold no processes; remove them (best
  # effort) so the kernel starts from a clean subtree.
  for leaf in "$unit/apps"/app-*; do [[ -d "$leaf" ]] && rmdir "$leaf" 2>/dev/null || true; done
else
  mkdir "$unit/apps"
fi
chmod 0700 "$unit/apps"
echo "+cpu +memory +pids" > "$unit/apps/cgroup.subtree_control"
echo "start-kernel: $unit/apps ready ($(cat "$unit/apps/cgroup.subtree_control"))"
exec "$1"

# Kernel resource protection

Each machine stores three independent switches in its kernel configuration:

```
/config set kernel.resource_protection.memory false
/config set kernel.resource_protection.disk false
/config set kernel.resource_protection.cpu false
/config show
```

Use `true` to enable a switch again, or `/config unset <path>` to restore its
on-by-default value. `/config schema` lists the settings. Changes apply to the
running kernel and persist across restarts. They apply to every kernel-owned
provider and terminal on that machine.

Protection is a last resort. Memory admission, pauses and stops require physical
memory to approach exhaustion on the host or an ancestor cgroup. The reserve is
1% of the workload ceiling, bounded to 64–512 MiB (and at most 10% on small
machines). Predicted peaks and a private workload accounting total cannot create
pressure on a healthy machine. Accounting cgroups impose no memory maximum,
memory throttle, swap prohibition or CPU weighting.

Disk admission requires the receiving volume to have less than its reserve:
1% of volume size, bounded to 64–512 MiB. Stopping a writer requires less than
half that reserve and observed writes to that exact volume. A prediction alone
does not stop a writer while gigabytes remain free.

When memory or disk protection is off, the kernel releases existing waits for
that resource and issues no new proactive pause or stop for it. Memory off also
disables the explicit process memory cap. Disabling one resource leaves the
other settings alone. The OS OOM killer, ancestor limits and explicit user Stop
commands remain effective. The CPU switch records permission for CPU starvation
intervention; the kernel currently has no proactive CPU pause/stop controller.

The focused native drill is `apps/cli/scripts/live-resource-protection-drill.mjs`.
It uses an own kernel, real relay and official low-effort Codex run, then checks
memory pressure inside an owned 4 GiB cgroup and disk pressure on a private ext4
loop volume. It requires delegated cgroups and loop-mount permission. Pass its
binary, profile, state and evidence paths through the `RESSET_*` environment
variables named at the top of the script; keep state and evidence outside Git.

Jobs that declare memory needs reserve capacity before spawning a process and
wait when that declared capacity is unavailable. This admission does not pause
or stop healthy running commands. Memory off and explicit Start now also apply
to these job reservations.

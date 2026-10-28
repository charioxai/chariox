# Process memory and session ownership

MP-08 / MP-09 / MP-10 / MP-11: providers and Room terminals share the kernel's
process resource policy on ordinary and managed machines.

Each launched process tree defaults to **16 GiB**. Set
`CHARIOX_PROCESS_MEMORY_LIMIT_MIB` on the kernel to change that limit for every
provider and terminal it subsequently launches. The value must be an integer of
at least 16 MiB; invalid values refuse the launch with an explanation. For
example, `CHARIOX_PROCESS_MEMORY_LIMIT_MIB=8192` sets an 8 GiB limit. There are no
separate provider or terminal settings. The default accommodates kernel builds
that need 8–12 GiB; a larger workload can use this override.

On Linux, when the existing cgroup-v2 hierarchy permits delegation, each tree
joins a separate child cgroup before executing its workload. `memory.max` bounds
its memory, swap is disabled for that tree, and `memory.oom.group` ends the whole
tree together. Workload children clear inherited OOM protection. The kernel stays
outside the workload cgroups. Their shared parent budget leaves 10% of physical
memory (at least 512 MiB) outside the workloads, respecting a finite parent limit.

A resident-memory watchdog covers Linux without delegation and macOS. It samples
owned roots and descendants every 250 ms and checks the same aggregate reserve;
Linux additionally checks available host memory. This sampled watchdog is a
best-effort limit and can overshoot between samples. It retains process start
identities across exec and checks them before signalling, so a recycled PID does
not authorize a signal. A single cached process-table sample is shared between
watchdogs. App workers retain their existing 512 MiB sandbox resource domains.

A memory termination produces the typed `memory_limit` provider termination or
terminal outcome. Terminal watches also receive the typed memory-limit event,
including the byte limit, instead of remaining pending after the process exits.

`/processes` in the TUI opens a live read-only panel showing the current session's launch-owned processes:
agent or terminal owner, PID, executable name, resident bytes, CPU and age. CPU is
`ps`'s lifetime average expressed as a percentage of one core. Arguments and
environment variables are excluded. Samples reach clients through the existing
session subscription projection, reconciled every two seconds; clients discard
samples older than five seconds. This adds no process termination command.

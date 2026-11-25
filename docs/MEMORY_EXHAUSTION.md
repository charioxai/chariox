# Memory exhaustion: the kernel stays up, agents use the rest

Decided by Miguel 2026-11-23 after b3's 19:55–20:05Z incident (a 66 GB test
process; 20A7 offline for ~2 min). Every number is one named constant in
`apps/kernel/src/runtime/process_resources/exhaustion.rs`.

1. **The kernel is protected, with no manual step.** Its own cgroup carries
   `memory.min`: 2 GiB by default, or a quarter more than the kernel has
   measurably used (its own `VmHWM`, rounded up to 256 MiB) when that is larger.
   Only memory the kernel actually uses is protected, so a small kernel on a
   small machine costs what it uses. Three layers, none a hand-run command:
   the shipped unit (`deploy/local-linux/chariox-kernel.service`) and the machine
   unit Setup writes (`apps/setup/machine/remote.mjs`) carry `MemoryMin=2G`; at
   start and every minute the kernel compares its cgroup's protection with what
   it needs and, when short, raises it itself through its service manager
   (`systemctl [--user] set-property --runtime <unit> MemoryMin=…`, which carries
   it up to the slices above, and the kernel writes its own cgroup and any level
   above it that it may write, since systemd does not set the slices: protection
   holds only as far as every level above carries it, and it is read back from
   the whole chain, not assumed; a user install's `user.slice`, owner slice and
   `user@UID.service` are set by `install-root.sh`); a documented setting
   (`CHARIOX_KERNEL_MEMORY_RESERVATION_MIB`) sets a larger floor. The kernel logs
   `process_resources.kernel_protection` saying which happened, and the warning
   with the command only when the manager refused. The executor kernel's measured
   peak (3.6 GB) makes it raise itself to 4.5 GiB; the L14 deploy also sets
   `CHARIOX_KERNEL_MEMORY_RESERVATION_MIB=4096` so it starts there.
2. **Agents use everything else.** Pool = machine memory − the kernel's own use
   − 512 MiB for the OS. No per-agent share: one agent may use nearly all of it.
3. **Only real exhaustion acts**: memory PSI "some" avg10 ≥ 20 % for ≥ 5 s AND
   pool use ≥ 70 %. (b3, 20:00Z: MemAvailable ~30 GB, reclaim thrash. The old
   512 MiB free-memory line could not fire.) The pressure is read from the
   tightest limited cgroup above the workloads when there is one, else the host.
4. **The one command that grew most over 10 s (≥ 64 MiB, no oldest exemption)**:
   throttle at the trigger (its owner's unit `memory.high` = what it holds), pause
   3 s later, stop 13 s after the trigger, each only while pressure is ≥ 10 %.
   Pressure below 10 % for 10 s lets it go. A command paused before that grows
   into exhaustion again is stopped at once.
5. **Every step is told twice**: to the command's own stderr (so a stopped
   command's tool result carries the reason and numbers) and as a `resource_notice`
   in its agent's inbox plus a status summary in the room, with no card.
6. An owner cap (`CHARIOX_PROCESS_MEMORY_LIMIT_MIB`) still stops at its line.

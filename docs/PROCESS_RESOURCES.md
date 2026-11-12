# Process memory and session ownership

MP-08 / MP-09 / MP-10 / MP-11: providers and Room terminals share the kernel's
process resource policy on ordinary and managed machines.

Resource pauses and stops are a last resort near exhaustion. Each machine has
independent memory, disk and CPU permissions in
`kernel.resource_protection`, all on by default. Turning a resource off releases
its existing waits and prevents proactive pauses/stops for it, including an
explicit memory cap. OS OOM handling and explicit user Stop commands remain in
effect. See [Kernel resource protection](RESOURCE_PROTECTION.md) for CLI commands.

All agents and terminals share memory accounting: the machine's memory (or its
bounding cgroup limit) minus 10% (at least 512 MiB). This ceiling helps select
work under real pressure; it is not a private allocation limit. Everything one
agent starts is one owner; each Room terminal is another. Starting an owner or
predicting a large command never interrupts healthy work.

On Linux, memory intervention requires host `MemAvailable` or physical headroom
in a bounding ancestor cgroup below the reserve: 1% of the accounting ceiling,
bounded to 64–512 MiB and at most 10% on small machines. PSI alone and the
kernel's own accounting total cannot create an emergency. On macOS, only critical
system memory pressure permits intervention. Linux admission and resumption use
physical headroom, so shared pages counted in multiple RSS totals cannot strand
work. When physical memory recovers, memory waits are released.

Under that pressure, a growing command pauses when remaining headroom is less
than the larger of the margin and one second of current aggregate growth.
After a successful pause, the latest measured growth of running work must
still be positive before any pressure stop: a decaying startup rate is not
ongoing allocation. When no command can pause and running work is still growing,
it stops ONE
command: a command growing at
128 MiB/s or more (a runaway) first; otherwise the largest command of the owner
furthest above its fair share (ceiling / owners, recomputed at every start and
stop). A command is one subtree under the provider or terminal process, that
process itself, or a job it left running. It is killed at once, without a grace
period in which a runaway keeps growing, and no second command is stopped for
the same cause within one second. The owner keeps running.

A command that admission paused (it grew toward a full pool) continues when its
room frees. Its growth estimate is the median of samples over the last 60 s
before the pause, so a short burst does not require room for repeated bursts.
Samples while stopped never lower that estimate. A steady fast grower therefore
stays paused until enough room returns, rather than cycling through resume and
pause. If a paused command or a command held before growing has waited 60 s and
still cannot continue in the room there is, the waiting command ends with the
memory-stop outcome, with the reason on its error stream. When memory keeps it:
what it still needs to continue, the room that was left once what is in use and reserved is
counted, whether the pool's ceiling or the machine's free memory set that room,
and the command's own size. When the memory room covers what it needs but not growing
on (everything running, it included, grows so fast that the pool would be short of
memory again within seconds): the rate everything grows by, what staying clear of
short memory takes at that rate, and the room that was left. When the disk keeps it (the writes it is known to
make do not fit above the floor): what it still needs of disk and what is free
on the volume it writes to. A pause never lasts for ever. A command that could
continue is never ended, however long it waited: the resume pass continues it.
Jobs a tool shell ran at once stay separate commands across a hot reload, so a paused one is
still found and continued.

`CHARIOX_PROCESS_MEMORY_LIMIT_MIB` sets an optional owner-set hard cap per agent
and per terminal (an integer of at least 16 MiB; invalid values refuse the launch
with an explanation). Over its cap, one command of that owner is stopped the same
way. Unset, no cap applies.

On Linux, when the existing cgroup-v2 hierarchy permits delegation, the kernel
creates an accounting cgroup `chariox-<pid>-<id>` beside its own, an owner
`unit-*` below it, and a `process-*` leaf per workload. These groups impose no
`memory.max`, `memory.high`, swap prohibition, CPU weighting or per-owner memory
reservation. Ancestor limits and OS OOM handling remain authoritative. Explicit
owner caps use the same monitor as the portable watchdog. Workload children clear
inherited OOM protection; the kernel stays outside workload cgroups. Empty owned
cgroups are removed after their workloads end.

A resident-memory watchdog covers Linux without delegation and macOS with the
same physical-pressure prerequisite and command selection from resident memory.
It samples owned roots and descendants every 250 ms and can overshoot between samples. It retains process
start identities across exec and checks them before signalling, so a recycled
PID does not authorize a signal. A single cached process-table sample is shared
between watchdogs. App workers retain their existing 512 MiB sandbox resource
domains.

A stopped command learns why: the kernel writes one line to its error stream
(when that is a pipe or terminal it can write to), for example
`chariox: stopped this command under memory pressure: 3100 MiB resident, 3300 MiB
for its owner, fair share 1400 MiB of the 8600 MiB shared ceiling`, so the agent
reads the reason in its tool output; the kernel logs the same record
(`process_resources.memory_stop`).
When the command is the workload's own process (a terminal command, a watched
process, the provider process itself), the workload ends with the typed
`memory_limit` outcome, whose `limit_bytes` is the owner-set cap or the fair share
at the stop.

That `memory_limit` is the provider termination or terminal outcome. Terminal watches also receive the typed memory-limit event,
including the byte limit, instead of remaining pending after the process exits.

`/processes` in the TUI opens a live read-only panel showing the current session's launch-owned processes:
agent or terminal owner, PID, executable name, resident bytes, CPU and age. CPU is
`ps`'s lifetime average expressed as a percentage of one core. Arguments and
environment variables are excluded. Samples reach clients through the existing
session subscription projection, reconciled every two seconds; clients discard
samples older than five seconds.

`/provider processes list` opens the same view. To stop work, use
`/provider processes kill agent <agent-id>` (one confirmation) or
`/provider processes kill process <process-id>` (no confirmation). The process
IDs are issued by the kernel and shown below each TUI row; a raw PID is refused.
The controller rechecks session access and launch ownership, sends TERM, then
KILL after a bounded grace period. Controller stops use the typed `outside_end`
outcome, preserving `memory_limit` for a resource limit. Individual child stops
leave the owning provider and unrelated processes running.

The Web Health view groups the same session projections by connected machine
and kernel, with Project - Room - Agent/Terminal owners and expandable totals.
Its collapse/expand state belongs to the viewer; process authority remains in
the kernel.

The resource settings are versioned at local **1017**, peer **455**. The process
shape fixture is `session-processes-954.json`; regenerate protocol consumers with
`node scripts/generate-protocol-versions.mjs` when the coordinator renumbers it.
Focused checks include `resource_protection_protocol_1017_455_snapshot_and_hash`
and `local_daemon_protocol_session_process_memory_outcomes_are_versioned`.

For the real control drill, start a kernel and client from the same combined
head, check their advertised version pair, and run two agents with long-lived
children. In the TUI, list processes, kill one child's opaque process id, then
kill the owning agent with confirmation. Verify `outside_end`, absence of the
selected tree, and the second agent's nonempty process set before and after.
Repeat through Health on the real Web bundle at DPR1 and DPR2; record the
source heads and screenshots separately from any earlier protocol's results.

MP-08 / MP-09 / MP-10 / MP-11: all kernel images install `procps` and run
`scripts/check-process-sampler.mjs` during the image build. The check launches
an owned shell process and reads the exact `ps` columns the monitor needs.
The publication runtime, lab runtime and Docker slice are covered; the native
managed image also installs `procps` explicitly and checks those columns.
The egress gateway, local Docker broker, dummy App and browser-only drill image
do not run a kernel and do not need the kernel sampler (the browser drill
already installs it for its own diagnostics).

On macOS, resource ownership uses Darwin's exec-stable process unique ID and
UID; kernel-access admission retains its stricter exec-image identity.
Run these focused regressions on a real Mac with Python 3 installed:

```sh
cargo test -p chariox-kernel --lib shell_exec_keeps_owned_root_and_subsequent_children_addressable
cargo test -p chariox-kernel --lib watchdog_covers_children_created_after_shell_exec
```

The first launches a shell, records its opaque process ID, execs another shell,
then creates a child; both remain addressable and stop together. On Darwin it
also asserts that exec changed the OS image version. The second crosses the
same exec boundary before creating a child that exceeds the portable RSS cap.
These focused regressions supplement PC-04's real client/provider drill.

The Claude stream launch path has an independent containment regression:
`cargo test -p chariox-kernel --lib claude_process_resources_end_only_the_overweight_stream_and_name_the_limit`.
It uses the real Claude runtime initializer and event drain with a bounded
stream fixture, checks prompt settlement and the named memory limit, and leaves
an unrelated stream alive. It needs no account and supplements the real Claude
provider leg; it does not certify that provider's authentication or CLI behavior.

## Linux OOM protection and controller termination

The kernel can retain its service's `oom_score_adj=-1000`. Every command it
launches uses the shared killable command constructor, including watched agent
commands, Room exec, browser controllers, connectors, scripts, probes and helper
commands. The owned standard-process and PTY spawn boundaries also raise negative
adjustments to zero. Every entry preserves a nonnegative inherited adjustment:
a positive service/container policy can be the minimum an unprivileged child
is allowed to keep. The native App domain entry applies the same rule before
joining its domain or executing bubblewrap. Failure to read or raise a negative
adjustment refuses the launch. Bootstrap kernel
replacement launches retain service policy; they are kernels, not workloads.

An owner-set cap is enforced by the resource monitor while memory protection is
on. Cgroup charge or portable resident-memory samples decide which owned command
to stop. It does not install a private cgroup throttle or hard limit, and memory
protection off disables this intervention too.

Regressions:
- `spawned_trees_are_killable_when_the_kernel_is_oom_protected` runs an isolated
  kernel test process at -1000 and checks standard, async, owned and PTY children,
  including their children and the resource-domain shell, at zero while the
  parent remains protected. It is an ignored privileged drill because setting
  -1000 requires Linux `CAP_SYS_RESOURCE`; the ordinary workspace suite never
  attempts that setup.
- `spawned_trees_preserve_a_positive_oom_policy` checks the same boundaries at
  +350. A root launcher establishes the positive minimum, then drops to nobody
  before the child runs; ordinary users also exercise preservation without
  requiring privilege.
- `linux_controller_ends_a_runaway_before_the_oom_killer` requires the owner-set
  typed limit, actual controller termination, zero `oom`/`oom_kill` events and
  complete cgroup cleanup. Run with `--ignored` on a delegated Linux host.
- `process_resources::pressure_tests` covers the shared ceiling (PC-06, PC-07) in
  watchdog mode and, with `--ignored` on a delegated host, in cgroup mode: a lone
  owner at 83% of the ceiling untouched, six owners starting without interrupting
  one above its new share, one command of the owner furthest above its share
  stopped under pressure, a 512 MiB/s runaway stopped first with zero `oom_kill`,
  and an owner-set cap stopping one command.
- `apps/app-worker/tests/oom-protection.test.mjs` executes the real native App
  admission entry under inherited protection and verifies it clears that policy
  even when admission subsequently refuses invalid descriptors. Its positive
  policy case drops privilege and verifies admission is reached without lowering
  the inherited adjustment.

Run the protected-parent drills explicitly as a Linux user with effective
`CAP_SYS_RESOURCE` (for example, root on the delegated builder):

```sh
cargo test -p chariox-kernel --lib \
  process_spawn::tests::spawned_trees_are_killable_when_the_kernel_is_oom_protected \
  -- --exact --ignored --nocapture
node --test apps/app-worker/tests/oom-protection.test.mjs
```

The native protected-parent case reports a plain capability skip when
`CAP_SYS_RESOURCE` is absent. Its positive-policy case always runs on Linux.
A root execution of both positive-policy probes drops to nobody after setting
the inherited minimum, so privilege cannot hide a forbidden decrease.

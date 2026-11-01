# Process memory and session ownership

MP-08 / MP-09 / MP-10 / MP-11: providers and Room terminals share the kernel's
process resource policy on ordinary and managed machines.

All agents and terminals share **one memory ceiling**: the machine's memory (or
the finite limit of the cgroup the kernel's workloads live in) minus a reserve of
10% (at least 512 MiB) for the kernel, the OS and the user's own apps. There is no
default per-agent limit: one agent alone can use nearly all of the ceiling, and
nothing is stopped while it has room, however far one owner is above its fair
share. Launching an agent never interrupts a running one. Everything one agent
starts (its provider runs, the jobs they leave running and the processes it starts
through `chariox.events.process`) is one owner; each Room terminal is another.

Only under real memory pressure does the kernel act: when less than a margin
(5% of the ceiling, at least min(512 MiB, 10%)) is left under the ceiling, when
the host's own available memory falls below that margin (user apps took it;
macOS: critical memory pressure), or when workloads stall on memory (Linux PSI
`full avg10` of 10% or more). It then stops ONE command: a command growing at
128 MiB/s or more (a runaway) first; otherwise the largest command of the owner
furthest above its fair share (ceiling / owners, recomputed at every start and
stop). A command is one subtree under the provider or terminal process, that
process itself, or a job it left running. It is killed at once, without a grace
period in which a runaway keeps growing, and no second command is stopped for
the same cause within one second. The owner keeps running.

`CHARIOX_PROCESS_MEMORY_LIMIT_MIB` sets an optional owner-set hard cap per agent
and per terminal (an integer of at least 16 MiB; invalid values refuse the launch
with an explanation). Over its cap, one command of that owner is stopped the same
way. Unset, no cap applies.

On Linux, when the existing cgroup-v2 hierarchy permits delegation, the kernel
creates a budget cgroup `chariox-<pid>-<id>` beside its own: `memory.max` is the
ceiling (a backstop the controller acts before), `memory.high` sits half a margin
below it to slow allocations near the line, swap is disabled, and with a delegated
`cpu` controller `cpu.weight` is 50 so the kernel and the user's apps keep more
CPU (CPU is never a reason to stop anything). Each owner is a `unit-*` cgroup
with `memory.low` set to its fair share; an owner-set cap adds `memory.high` (the
cap) and `memory.max` (the cap plus the greater of 25% or 64 MiB). Each workload
is a `process-*` leaf. There is no `memory.oom.group`. Workload children clear
inherited OOM protection; the kernel stays outside the workload cgroups. A new
budget removes the empty budgets of kernels whose process is gone, and a budget
removes its empty descendants when its last workload ends.

A resident-memory watchdog covers Linux without delegation and macOS with the
same decision on the sum of resident memory. It samples owned roots and
descendants every 250 ms and can overshoot between samples. It retains process
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

The process protocol lands in the union local **911**, peer **362**. Local **913**,
peer **364** adds the same rows to the one-shot `SessionState` response (`processes`). Its shape
fixture is `session-processes-800.json`; regenerate protocol consumers with
`node scripts/generate-protocol-versions.mjs` when the coordinator renumbers it.
The focused shape check is
`cargo test -p chariox-kernel --lib local_daemon_protocol_session_process_memory_outcomes_are_versioned`.
Run it through the lane's Cargo slot. `node --test scripts/protocol-versions.test.mjs`
also checks the process-control fixture against both generated versions.

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

An owner-set cap throttles at the cap using `memory.high`; the monitor observes
the unit's charged memory and stops one command there. `memory.max` adds the
greater of 25 percent or 64 MiB as a killable burst backstop. The kernel and its
other domains stay outside that tree. The watchdog remains the enforcement path
when cgroups are unavailable.

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

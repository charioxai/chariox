# Agent wakes (MP-08 / MP-09 / MP-10 / MP-11 A02, A03, A10)

The kernel owns the one durable answer to "what is this agent waiting for, and
who wakes it?". An agent registers a kernel-owned wake source, ends its turn
waiting on it with a finite deadline (`chariox.events.yield`), and the kernel
starts a new turn when the source occurs. Nothing in this path is silent: every
state is stored, shown to every client, and has a visible failure.

## Parts

| Part | Job | Where |
| --- | --- | --- |
| Ledger | Tasks, obligations, registrations, wakes, inbox, receipts, the settle rules. One writer, new tables only. | `durable_state/agent_lifecycle*` |
| Scheduler | Fires time-driven wakes, sweeps every 30 s, reports proof of life. | `runtime/state/agent_wake_scheduler.rs` |
| Delivery | Turns an inbox event into a provider turn or a steer, and records the exact receipt. | `runtime/state/agent_inbox_delivery.rs` |
| Driver | Owns one `kind` of source and only decides *when* it occurs. | `runtime/state/wake_driver.rs` |
| Tools | `chariox.events.{inbox,ack,yield,blocked,timer,cancel_wake,wakes}` | `tool_dispatch/agent_events.rs`, `agent_wakes.rs` |

A plain turn only gets a task row and the task id in its hidden context. It is
never held to a wait, never corrected, and a plain failure is cancelled exactly
as it is without the ledger.

Workflow turns use their workflow turn envelope for completion. The event tools
are withheld from workflow provider discovery, and direct calls are refused
before any task or wake mutation. Workflow turns receive no task-wait instruction.
Task waits remain unavailable there until wake and correction continuations retain
the original workflow run and node identity (MP-08/MP-10/MP-11).

The workflow boundary drill starts a real workflow through the TUI, confirms
that its provider sees no event tools, and completes the workflow's normal
output envelope. Forced calls to all seven event tools must fail without
changing tasks, wakes or workflow nodes; ordinary timer/yield/wake completion
must still work (MP-08/MP-10/MP-11).

## Task states

`working` → `waiting` (a yield with live registrations and a deadline) →
`working` (woken) → `done`. `blocked` means the owner must act and always has
one owner interaction; `cancelled` is a stop. A task with an open obligation
that ends its turn without a valid wait or answer gets one correction turn,
then blocks.

## The driver interface

A new source kind (the terminal, a watcher) implements `WakeDriver` and
registers it with `register_wake_driver`. It never touches the ledger.

```rust
trait WakeDriver: Send + Sync {
    fn kind(&self) -> &'static str;
    fn arm(&self, wake: &AgentWake) -> Result<(), DaemonError>;      // after the ledger committed
    fn cancel(&self, wake: &AgentWake) -> Result<Cancel, DaemonError>; // Done | Pending
    fn recover(&self, wake: &AgentWake) -> Recovery;                 // Reattached | Lost(reason)
    fn liveness(&self, wake: &AgentWake) -> Liveness;                // Live | Lost(reason)
}
```

- A tool arms a source with `KernelRuntimeState::arm_wake_source`: the ledger
  commits obligation, registration and wake first, then calls `arm`. An unknown
  kind is refused before anything is stored; an `arm` that fails leaves the wake
  `cancelled`.
- The driver reports through `wake_sink(kind)`. `verified(wake_id)` is the
  proof of life for a `starting` wake. `occurred(WakeOccurrence)` reports an
  occurrence: a unique `occurrence_id` (a replay is one admission), a `kind`
  (the ledger reserves its own occurrence kinds and refuses them), a redacted
  `payload`, an updated `summary` and `end` (`Succeeded`, `Failed`, `Lost`) when
  the source is over. A sink only accepts wakes of its own kind.
- Drivers are synchronous. A driver that needs I/O spawns its own task and
  reports through its sink.
- A source that cannot be observed is reported `Lost` and is never relaunched.

## No silent dormancy

- Every armed wake has proof of life (`verified_at_ms`); an unverified wake is
  shown as `UNVERIFIED` and reported as such by `chariox.events.wakes`.
- The scheduler writes a tick; a tick older than 15 s marks the scheduler
  stalled in the `wakes` health and in the alert a creation reports. The 30 s
  sweep fires overdue wakes and reads `liveness` for every running wake.
- Delivery is recorded per occurrence (`accepted`, `pending`, `uncertain`,
  `rejected`); a wake not delivered within 60 s raises a visible notice, and a
  wedged delivery blocks the task within 2 minutes with an owner interaction and
  no second provider turn.
- Three handled wakes without progress block the task; Resume continues it,
  Cancel retires its wakes.
- A deadline that lapses with healthy sources is one `deadline_reached` wake; a
  new wait needs a fresh deadline. A lost source is one `source_lost` wake.

## What clients see

`RuntimeSession.agent_tasks` / `agent_wakes` and the `agent_task_updated` event
(PROTOCOL.md, local 561 / peer 169). The TUI footer shows a task's wait or block
and `/agent wakes [agent]` lists wakes with next fire and last receipt. Clients
only display; every decision is the kernel's.

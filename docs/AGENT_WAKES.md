# Agent wakes (MP-08 / MP-09 / MP-10 / MP-11 A02, A03, A10)

The kernel owns the one durable answer to "what is this agent waiting for, and
who wakes it?". An agent registers a kernel-owned wake source, ends its turn
waiting on it (`chariox.events.yield`; a deadline is optional and required only for a goal), and the kernel
starts a new turn when the source occurs. Nothing in this path is silent: every
state is stored, shown to every client, and has a visible failure.

## Asking without stopping work

MP-08 / MP-09 / MP-10 / MP-11: use `chariox.request_popup` with `wait:false`
for a question whose answer can arrive later. Keep working after its immediate
`interaction_id` and `registration_id` response. The same question card appears
on every attached terminal; authorized agents and granted external followers
answer through the existing interaction response path.

A blocking `request_popup` (the default) ends its card after at most 280 seconds
with status `timed_out`: the provider cuts a waiting tool call at 300 seconds
without telling the kernel, and a card nobody can answer must not stay open.
A step a person has to do (a CAPTCHA, a login, an approval) uses `wait:false`
and `chariox.events.yield`. The Vault unlock card an agent's tool call raises
stays open 280 seconds for the same reason; a provider launch keeps 60 seconds.

```json
{"wait":false,"title":"Choose the next check","message":"Which check should run next?","choices":[{"id":"focused","label":"Focused checks","reply":"Run the focused checks."}],"timeout_sec":300}
```

Include `registration_id` when yielding on several sources. If questions are
your only remaining obligations, finishing this turn retains their wait. The
answer wakes the idle agent through its event inbox. Read the exact choice,
reply and answerer in `payload.public_answer`, handle it, then acknowledge the
event. Deadline defaults keep `status:"timed_out"`; without a default there is
no reply. Questions and deadlines survive kernel restart. Stop/cancel/delete
withdraws pending questions, without choosing a default. Treat `source_lost`,
including the existing `public_answer.cancelled:true` receipt, as withdrawal
rather than an answer. An ended asking task does not start another provider turn.

### Person cards (to-dos, conditions, one action)

A card is the shared interaction card with up to three extras. All are the
kernel's: the clients only show the card and send the answer.

- **To-do**: a `wait:false` card whose only choice is Done, for a step only the
  person can do. The agent keeps working; Done wakes it. The card survives a
  page reload and a kernel restart like any `wait:false` question.
- **Completion condition** (`until: {"wake_id": ...}`, `wait:false` only): the
  `wake_id` of a source the same agent armed (`events.timer`, `events.process`,
  `events.terminal`). When that source occurs or ends, the card closes on every
  client within the wake delivery (about a second) and the agent receives the
  card's answer as `status:"answered"`, `choice_id:"condition_met"` and a
  kernel-written `reply` ("Wake source 'x' occurred: ..."). No answer is needed.
  An unknown, foreign or already ended source refuses the card.
- **One action** (`action: {"request": <kernel request>}`): the card shows the
  kernel's own description of exactly that request ("<agent> asks to run
  AliasSession (alias: ..., session_id: ...)"); the agent's title and message are
  ignored. It is a passkey decision: Approve needs the Chariox passkey and runs
  the request at once as the owner; Cancel (or an expired or abandoned card)
  runs nothing. The call blocks until decided and returns `ran` and the result.
  What `/sudo` refuses (approvals, access, secrets, the passkey, pairing) is
  refused here too, and a second answer to the same card finds it gone, so the
  request runs once.

Use the default `wait:true` only when work cannot continue without the answer.
That mode keeps the existing blocking result. `events.blocked` is for an owner
action that actually prevents further work.

| Tool | When to use it | Result |
| --- | --- | --- |
| `request_popup` with `wait:false` | Keep working while awaiting input | Interaction and wake registration IDs; attributed answer in inbox |
| `request_popup` with `wait:true` or omitted | Work needs the answer before continuing | Blocking answer or timeout |
| `events.yield` | Await admitted sources after useful work | End the provider turn; kernel wakes it for a source or deadline |
| `events.blocked` | Work requires an owner action | Existing owner decision and supervised task |

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
- A helper's work that can no longer complete ends with a definitive outcome and
  wakes whoever waits on it (A10-S04). The helper's task ends `cancelled` with
  its obligations `failed`, and the waiter's `source_lost` wake carries
  `public_answer`: the typed turn outcome (`outcome.kind`, `retryable`, `reason`,
  the same one the turn's event, history and state carry), `recovery` and, for a
  leased helper, its `host` (the slice or worker kernel). Three causes are
  covered: the provider process is killed (`provider_crashed` with the exit or
  signal), the relay reports the slice or worker kernel offline for 20 s
  (`provider_crashed`, the turn is never replayed), and the turn's workspace
  (a git worktree the sweep saw earlier) is removed. A task that owes ledger work
  stays `blocked` for the owner instead of cancelled.

## What clients see

`RuntimeSession.agent_tasks` / `agent_wakes` and the `agent_task_updated` event
(PROTOCOL.md, local 561 / peer 169). The TUI footer shows a task's wait or block
and `/agent wakes [agent]` lists wakes with next fire and last receipt. Clients
only display; every decision is the kernel's.

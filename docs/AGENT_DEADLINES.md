# Agent deadlines (MP-08 / MP-09 / MP-10 / MP-11)

A person or external agent can arm an unconditional deadline on an agent's
executing kernel before submitting its first prompt:

```json
{"SetAgentDeadline":{"session_id":"ROOM","agent_id":"AGENT","deadline_ms":1792800000000}}
```

`deadline_ms` is a required field: an absolute UTC epoch millisecond to set,
extend or shorten, or JSON `null` to clear. An omitted field is refused.
One `SetAgentDeadline` request returns the resulting durable receipt;
`deadline:null` confirms there is no deadline. A reached deadline ended that
run, not the agent: once its cleanup has finished (`stopped_at_ms` is set), the owner, or a parent that
may edit it, sets a new time or `none`, and the agent takes prompts again with its context. A deadline
bounds only the agent that carries it: its sub-agents are neither stopped nor limited by it, so they
keep working. When a limit ends, whether the child is idle, running a turn or waiting,
the PARENT agent receives one urgent `child_time_limit_ended` event that
names the extend command: the parent extends the limit and prompts the agent again, or lets it
stay stopped. No card reaches the person; a time limit is the parent agent's setting, never the
person's to answer. A top-level agent has no parent to ask: the Room's stop notice is the whole
story, and the owner's external agent may extend or clear the limit itself the same way. A
refused edit names who can make it. Edits during pending cleanup are refused
with `agent_deadline_cleanup_pending`; retry shortly.

The owner can use `chariox agent deadline <agent> <90s|30m|2h|1d|none>`
(optionally `--session ROOM`, `--kernel KERNEL`, `--socket PATH`). Durations start
at the time of the request. Agents use the Room command
`agent deadline <agent-ref> <duration|none> [--room <room>]` or read with
`agent deadline get <agent-ref> [--room <room>]`; an agent may read its own
limit with `agent deadline get` on itself, so it can plan and warn its parent.
A parent can edit descendants; an owner's
coordinator can edit a peer of the same owner when its authority covers the
peer, including an ordinary agent in another room. An agent cannot edit its own
deadline, an ancestor's, a hidden sub-agent in another room, or another owner's
agent. A new sub-agent starts with no limit unless `agent spawn --deadline` gives it its own.

An edit is serialized with deadline cleanup, so an old scheduler snapshot
cannot stop a run after an accepted extension or clear. A limit bounds only the agent that carries it: an
ended agent's cleanup never stops a sub-agent, so a sub-agent revived under an ended parent keeps the work it
starts next, and the owner's new limit for it is the only limit it has.
Deadline records commit in one durable transaction.
`GetAgentDeadline {session_id,agent_id}` returns `AgentDeadline {deadline}`;
`deadline:null` means none was armed. A record contains the Room and agent ids,
the deadline, `state:"armed"|"reached"`, `reached_at_ms` and `stopped_at_ms`.

The durable state writer, scheduler and lifecycle sweep enforce the deadline.
It does not start a second timer service or send an advisory prompt that the
agent could ignore. The scheduler
checks wall-clock deadlines every second, including when Room agent tools are
disabled. The existing lifecycle sweep is a second enforcement path.

Agent runs with a reached deadline receive their durable reached state before the scheduler awaits
physical cleanup. The admission fence also checks the deadline itself, so a
late scheduler cannot admit another prompt. Cleanup stops the kernel-owned
provider process/connector, cancels the active and queued prompts, retires
wakes/tasks and releases normal prompt claims. Unfinished cleanup is retried;
`stopped_at_ms` appears only after cleanup succeeds. The same transaction publishes one agent event
with `reason:"deadline_reached"` to its outside followers, and the Room shows a notice that says what stopped and which sub-agents go on. Ten minutes and one minute before a deadline, the Room shows a warning and the agent, if it has work, receives one too. Completed idle agent runs do
not cause repeated deadline writes. No process that armed the deadline or open watch is
needed. After a kernel restart, an overdue stored deadline is enforced during
the first scheduler tick; the original reached/stop receipts are retained.

A leased agent runs on a worker, so the worker enforces its deadline. The home
sends `SetLeasedAgentDeadline` (relay peer 442) to the worker first and records
its own deadline only after the worker confirms; a worker that refuses or does not
answer leaves no deadline on the home, and a spawn that cannot be armed removes
the child it created. The worker arms the deadline on the agent that carries the
leased work, so its own scheduler tick stops the provider run even when the home
is offline; the home also cancels the leased turn when its own tick reaches the
deadline. Moving a deadline-bound agent to a worker stays refused with
`agent_deadline_requires_executor`: move the agent first, then arm the deadline.
A leased agent that ended at its worker's deadline stays ended there: neither an edit nor the shift a home restart gives back revives it (`agent_deadline_reached`); start a new agent for new work. A prompt that reaches the worker after the deadline is rejected before any provider run
starts or prompt input is taken (`agent_deadline_reached`).
This is an optional general facility for managing an agent. It does not replace
managed-host automatic shutdown policy (MP-09).

The requests and response were introduced in local protocol **585**. The deadline-edit
protocol pair is recorded in `fixtures/protocol-versions.json`; the local and
leased-worker requests both carry a required nullable `deadline_ms`. The deadline carries
no credentials and is visible through the ordinary authorized Room/gateway
request path. Refusals use constant remediation text; provider diagnostics are
not copied into external errors.

The focused live drill is `scripts/drills/agent-deadline-edit.mjs`: DEADLINE-01
extends a running agent past its old deadline; DEADLINE-02 clears it and observes
work finish past the old deadline; DEADLINE-03 shortens it, checks the stopped
receipt and refuses later work; DEADLINE-04 checks the edited deadline and a
cleared deadline after restart and admits work past the old time. Both require the real CLI, kernels, relay and a
product-linked real provider. Storage, protocol and runtime fixture tests are
supplementary regression checks.

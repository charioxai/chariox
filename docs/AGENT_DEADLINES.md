# Agent deadlines (MP-08 / MP-09 / MP-10 / MP-11)

A person or external agent can arm an unconditional deadline on an agent's
executing kernel before submitting its first prompt:

```json
{"SetAgentDeadline":{"session_id":"ROOM","agent_id":"AGENT","deadline_ms":1792800000000}}
```

Use a command id with the generic CLI request. `deadline_ms` is an absolute UTC
epoch millisecond. A deadline may be shortened or retried, but cannot be
extended or reset after it fires. New work after a deadline needs a new agent.
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
with `reason:"deadline_reached"` to its outside followers. Completed idle agent runs do
not cause repeated deadline writes. No process that armed the deadline or open watch is
needed. After a kernel restart, an overdue stored deadline is enforced during
the first scheduler tick; the original reached/stop receipts are retained.

A leased agent runs on a worker, so the worker enforces its deadline. The home
sends `SetLeasedAgentDeadline` (relay peer 442, Landing 8) to the worker first and records
its own deadline only after the worker confirms; a worker that refuses or does not
answer leaves no deadline on the home, and a spawn that cannot be armed removes
the child it created. The worker arms the deadline on the agent that carries the
leased work, so its own scheduler tick stops the provider run even when the home
is offline; the home also cancels the leased turn when its own tick reaches the
deadline. A deadline of a parent shortens the deadlines of its remote descendants
the same way. Moving a deadline-bound agent to a worker stays refused with
`agent_deadline_requires_executor`: move the agent first, then arm the deadline.
A prompt that reaches the worker after the deadline is rejected before any provider run
starts or prompt input is taken (`agent_deadline_reached`).
This is an optional general facility for managing an agent. It does not replace
managed-host automatic shutdown policy (MP-09).

The requests and response were introduced in local protocol **585**, without
a relay protocol change. The deadline carries
no credentials and is visible through the ordinary authorized Room/gateway
request path. Refusals use constant remediation text; provider diagnostics are
not copied into external errors.

Required live scenarios are an active official-provider turn stopped at its
deadline, and a deadline that becomes overdue while its kernel is down and is
enforced on restart. Both require the real CLI, kernels, relay and a
product-linked real provider. Storage, protocol and runtime fixture tests are
supplementary regression checks.

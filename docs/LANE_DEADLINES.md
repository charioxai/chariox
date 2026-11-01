# Lane deadlines (MP-08 / MP-09 / MP-10 / MP-11)

The outside coordinator can arm an unconditional deadline on an agent's
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

This reuses agent-model A02's single writer and A03's scheduler and sweep on
the union with `amint/c2` at `4bea22d44`. It does not start a second timer
service or send an advisory prompt that the agent could ignore. The scheduler
checks wall-clock deadlines every second, including when Room agent tools are
disabled. The existing lifecycle sweep is a second enforcement path.

Due lanes receive their durable reached state before the scheduler awaits
physical cleanup. The admission fence also checks the deadline itself, so a
late scheduler cannot admit another prompt. Cleanup stops the kernel-owned
provider process/connector, cancels the active and queued prompts, retires
wakes/tasks and releases normal prompt claims. Unfinished cleanup is retried;
`stopped_at_ms` appears only after cleanup succeeds. Completed idle lanes do
not cause repeated deadline writes. No coordinator process or open watch is
needed. After a kernel restart, an overdue stored deadline is enforced during
the first scheduler tick; the original reached/stop receipts are retained.

A home kernel cannot promise this for a disconnected leased worker. Arming a
deadline on a remote-backed agent, or moving a deadline-bound agent to a worker,
is refused with `agent_deadline_requires_executor`. This is an ordinary local
executor facility; it does not establish A10 leased-deadline acceptance or
replace managed-host automatic shutdown policy (MP-09).

The additive requests and response share **allocated local 585** with outside
event subscriptions on the agent-model union. The unused provisional 584 is
retired. Relay peer 181 is inherited from that union. The deadline carries
no credentials and is visible through the ordinary authorized Room/gateway
request path. Refusals use constant remediation text; provider diagnostics are
not copied into external errors.

Required live scenarios are an active official-provider turn stopped at its
deadline, and a deadline that becomes overdue while its kernel is down and is
enforced on restart. Both require the real CLI, kernels, hosted relay and a
product-linked real provider. Storage, protocol and runtime fixture tests are
supplementary regression checks.

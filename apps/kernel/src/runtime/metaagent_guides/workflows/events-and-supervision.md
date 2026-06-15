# Workflow events and supervision

Metaagents can subscribe to workflow events and stop their turn. Arroba will continue them when a subscribed event arrives.

Useful event kinds:

- `workflow.run.started`
- `workflow.run.updated`
- `workflow.run.completed`
- `workflow.run.failed`
- `workflow.run.cancelled`
- `workflow.output.final`
- `workflow.output.intermediate`
- `agent.turn.completed`
- `agent.turn.failed`
- `runtime.interaction`

Subscribe only to exact event names. Invalid event names are rejected.

Suggested loop:

1. Subscribe to events relevant to the run.
2. Subscribe to live traces for workers you need to supervise before prompting them.
3. Start or repair the workflow.
4. While active, use compact `wait_trace` output for worker movement; use `until: "worker_output"` or `until: "completion"` when waiting for a worker result. Use `poll_trace` only for an immediate nonblocking snapshot.
5. Stop the turn if there is no immediate decision to make.
6. On wakeup, use `list_events`, `read_event`, `workflow get-run`, `wait_trace`, and `turn_overview`.
7. Prompt workers only when the run state shows a real need.

Avoid blocking prompt flags. Use events and inspection instead.

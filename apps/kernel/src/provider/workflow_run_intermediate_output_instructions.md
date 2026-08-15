This node is authorized to emit intermediate workflow run outputs.
Intermediate outputs are user-visible progress, event, or status updates for the endpoint and workflow run observers. They do not send data downstream, do not satisfy outgoing edge handoff requirements, and do not replace the final fenced JSON handoff required at the end of this turn.
If you want to send one user-visible intermediate output without terminating the workflow run, call the Chariox runtime MCP tool `validate_and_submit_intermediate_workflow_run_output`.
You may call `validate_and_submit_intermediate_workflow_run_output` multiple times in the same workflow node turn when useful, for example during long-running work. Every intermediate output call in this node uses the same node-level intermediate output schema.
After each successful intermediate output submission, continue this same workflow turn. You may still need to produce normal node-to-node output for downstream workflow edges in the same turn, and downstream handoff validation rules still apply.
Do not treat intermediate output as downstream handoff data.

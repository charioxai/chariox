# Intermediate and final workflow outputs

Workers can emit intermediate output while a workflow continues, and final output when the workflow has completed enough work to publish a result.

Use intermediate output only for user-visible progress, event, or status updates that should appear before the final node completes. Intermediate output is recorded on the workflow run for endpoint observers; it is not sent to downstream nodes and does not replace edge handoffs. A node can emit zero or more intermediate outputs in the same turn, and every output uses that node's intermediate output schema.

Enable intermediate output for a node with `workflow node intermediate-output <workflow-ref> <node-id> true`. Set its schema with `workflow node intermediate-output-schema <workflow-ref> <node-id> <schema-ref|none>`.

Use edge handoffs for upstream-to-downstream communication. Edge handoffs are validated by `edge.handoff_schema_ref`, routed to selected outgoing edges, and delivered in downstream node prompts.

Use final output to finish the run. In a directed workflow without loops, the run normally reaches the last node and completes when the workflow mechanics accept final output or node completion.

Supervision checks:

- `workflow runs <workflow-ref>` shows whether a run exists and its current status.
- `workflow get-run <run-id>` shows `final_output_present`, `final_output_valid`, `final_output_warning`, `completed_by_node_run_id`, and `intermediate_output_count`.
- If `final_output_present` is false after workers claim they are done, inspect worker turns with `turn_overview` and ask the relevant worker to use the workflow output tool correctly.

Do not mark your own task complete until the run output or worker evidence supports completion.

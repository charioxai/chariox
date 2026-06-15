# Partial and final workflow outputs

Workers can emit intermediate output while a workflow continues, and final output when the workflow has completed enough work to publish a result.

Use intermediate output for progress that should be visible before the final node completes. Enable it for a node with `workflow node intermediate-output <workflow-ref> <node-id> true`.

Use final output to finish the run. In a directed workflow without loops, the run normally reaches the last node and completes when the workflow mechanics accept final output or node completion.

Supervision checks:

- `workflow runs <workflow-ref>` shows whether a run exists and its current status.
- `workflow get-run <run-id>` shows `final_output_present`, `final_output_valid`, `final_output_warning`, `completed_by_node_run_id`, and `intermediate_output_count`.
- If `final_output_present` is false after workers claim they are done, inspect worker turns with `turn_overview` and ask the relevant worker to use the workflow output tool correctly.

Do not mark your own task complete until the run output or worker evidence supports completion.

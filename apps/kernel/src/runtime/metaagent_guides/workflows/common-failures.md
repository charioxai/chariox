# Common workflow failures

Workflow object exists but nothing runs:

- Check that an endpoint exists.
- If not, create one with `workflow endpoint new <workflow-ref> <entry-node-id> [alias]`.
- Run with `workflow run <workflow-ref> <endpoint-ref> [prompt]`.

Multiple nodes exist but only one node runs:

- Check `workflow resolve <workflow-ref>` for missing edges.
- Add edges with `workflow edge add`.

Run exists but the metaagent cannot tell what happened:

- Use `workflow get-run <run-id>`.
- Inspect `active_node_run`, `node_runs`, `messages`, `failure_events`, and output fields.
- Use `turn_overview` for the worker when run state alone is insufficient.

Event subscription does not wake the metaagent:

- Verify the exact event kind with `list_subscriptions`.
- Subscribe only to documented event names.

Worker says the task is done but workflow output is missing:

- Inspect `final_output_present` and `final_output_warning`.
- Prompt the worker to submit the required workflow output rather than starting unrelated extra work.

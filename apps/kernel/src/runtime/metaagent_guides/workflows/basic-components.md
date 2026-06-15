# Workflow basic components

A runnable workflow needs four pieces:

1. A workflow definition created with `workflow new`.
2. One or more nodes created with `workflow node add`, each backed by an owned regular agent.
3. Edges created with `workflow edge add` when work must move from one node to another.
4. An endpoint created with `workflow endpoint new`, pointing at the entry node.

Creating only the workflow object is not enough. A workflow that has nodes but no endpoint cannot be triggered. A workflow that has multiple nodes but no edges will not pass work downstream.

Before building, use `search_commands` or `command_docs` for the exact command syntax. After building, use `workflow resolve <workflow-ref>` and verify:

- `nodes` contains every worker that should participate.
- `edges` connects the intended handoff path.
- `endpoints` contains the trigger the run will use.

Trigger with `workflow run <workflow-ref> <endpoint-ref> [prompt]`. Inspect progress with `workflow runs <workflow-ref>` or `workflow get-run <run-id>`.

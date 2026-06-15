# Workflow edges and handoffs

Edges define where completed node work goes next. If a downstream worker should receive the result of an upstream worker, connect the nodes with `workflow edge add <workflow-ref> <from-node-id> <to-node-id>`.

Use `workflow resolve <workflow-ref>` after editing. A complete handoff path should show:

- both source and target nodes,
- an edge whose `from_node_id` is the source node,
- an edge whose `to_node_id` is the target node,
- an endpoint that starts at the intended entry node.

If a workflow run stays on the first node or never reaches a reviewer, inspect `workflow get-run <run-id>`:

- `node_runs` shows which nodes actually ran,
- `messages` shows handoff messages,
- `unconsumed_message_count` shows messages not yet consumed,
- `failure_events` explains failed or missing handoffs.

Missing edges are fixed by adding the edge and starting a new run, or resuming if the run is paused and repairable.

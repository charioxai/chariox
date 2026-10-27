# Minimal directed workflow

Use a directed workflow when the task has a natural handoff, such as implementation followed by review.

Typical shape:

1. Spawn or select an implementation worker.
2. Spawn or select a review worker.
3. Create the workflow.
4. Add one node for each worker.
5. Add an edge from implementation node to review node.
6. Create an endpoint on the implementation node.
7. Run the endpoint with the task prompt.

Example command sequence:

```text
agent spawn implementer
agent spawn reviewer
workflow new app-build
workflow node add app-build implementer
workflow node add app-build reviewer
workflow edge add app-build <implementer-node-id> <reviewer-node-id>
workflow endpoint new app-build <implementer-node-id> default
workflow run app-build default Build the requested app and hand off for review.
```

Do not implement directly. Your job as a metaagent is to build and supervise the execution path.

For parallel branches that converge into one reviewer or aggregator, add all branch edges into that node and then run `workflow node wait-for-all-inputs <workflow-ref> <join-node-id> true`.

This node is authorized to complete the workflow run.
If you consider that the workflow is complete and the run should stop, or will stop by design at this node, generate final workflow run output and submit it by calling the Chariox runtime MCP tool `validate_and_submit_workflow_run_output`.
When you are generating final workflow run output, normal node-to-node output is not necessary and does not need `validate_workflow_handoff`.
Do not finalize the turn until `validate_and_submit_workflow_run_output` returns `valid: true` with no warning.

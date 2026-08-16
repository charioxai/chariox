This is the last allowed turn for this node in the current workflow run.
- node turn index: {{TURN_INDEX}}
- node max turns: {{MAX_TURNS}}
If you consider that the workflow is complete and the run should stop, or will stop by design at this node, generate final workflow run output in this turn. In that case, normal node-to-node output is not necessary and does not need `validate_workflow_handoff`. Instead, call the Chariox runtime MCP tool `validate_and_submit_workflow_run_output` and do not finalize the turn until it returns `valid: true` with no warning.

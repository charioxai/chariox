# Generate an agent app

Use this recipe when the user asks for an app and expects the metaagent to coordinate work.

Start by writing or updating your plan. Identify the app goal, acceptance checks, likely files, and workers needed.

Choose either direct delegation or a workflow:

- Direct delegation is simpler for one implementer plus optional reviewer.
- A workflow is useful when the task prompt asks for one or when the work has clear stages.

For direct delegation:

1. Spawn regular worker agents.
2. Give each worker one explicit subtask.
3. Use `turn_overview` to inspect worker results.
4. Ask for fixes only when evidence shows a gap.
5. Mark complete only after implementation and validation evidence exists.

For workflow delegation:

1. Read the workflow guides.
2. Create nodes, edges, and an endpoint.
3. Run the workflow with the task prompt.
4. Subscribe to workflow and worker events.
5. Inspect `workflow get-run` and worker turns before deciding completion.

Do not implement directly. Do not keep spawning workers after the app is built and validated unless there is a concrete missing requirement.

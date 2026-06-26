# Workflow-Code Authoring

Use workflow-code when the task is to generate a complete workflow shape as a portable script. Use manual workflow commands when incremental interactive editing is simpler. The script is a workflow generator: every apply creates a new workflow and new node, edge, endpoint, queue, watchdog, and generated-agent ids. Handles inside the script only connect script components to each other; after apply, the kernel returns handle-to-id maps in the apply report.

Workflow-code scripts run in the kernel compiler with a single builder named `workflow`. JavaScript is the default language. Pass `language: "javascript"` or `language: "typescript"` in `create`, `update`, `validate`, `apply`, or `run` when the source language needs to be explicit; legacy `java_script` and `type_script` decode for compatibility, but new scripts and tool calls should use `javascript` or `typescript`. Do not invent attributes. The compiler rejects unknown fields after it exports the builder state into the kernel workflow-code schema.

## Builder API

- `workflow.define(options)` sets workflow-level properties:
  - `alias`
  - `prompt` (default invocation prompt used by `workflow_code.run` when no prompt is passed)
  - `flushAgentContextBeforeRun`
  - `maxConcurrent`
  - `runOutputSchema`
  - `intermediateOutputSchema`

- `workflow.schema(options)` defines an inline JSON Schema and returns a schema handle:
  - `handle`
  - `alias`
  - `description`
  - `schema`

- `workflow.schemaFromFile(options)` loads a JSON Schema file from the session schema import root and returns a schema handle:
  - `handle`
  - `path`
  - `alias`
  - `description`

- `workflow.newAgent(options)` defines a generated agent for a node:
  - `alias`
  - `provider`
  - `model`
  - `effort`
  - `accountProfile`

- `workflow.existingAgent(agentRef)` binds a node to an already spawned session agent. In metaagent apply/run, the existing agent must be controlled by that metaagent. Provider rebindings cannot target existing-agent nodes.

- `workflow.node(options)` defines one workflow node and returns a node handle:
  - `handle`
  - `agent`
  - `publicLabel`
  - `instructions`
  - `canCompleteWorkflowRun`
  - `canEmitIntermediateRunOutput`
  - `waitForAllInputs`
  - `intermediateOutputSchema`
  - `maxTurns`
  - `extensions`
  - `canvas`

- `workflow.edge(fromNode, toNode, options)` defines a directed edge and returns an edge handle:
  - `handle`
  - `sourceSide`
  - `targetSide`
  - `handoffSchema`
  - `validationPolicy` (`"warn"` or `"halt"`)
  - `canvas`

- `workflow.endpoint(entryNode, options)` defines an invocation endpoint and returns an endpoint handle:
  - `handle`
  - `alias`
  - `canvas`

- `workflow.queue(options)` defines a prompt queue and returns a queue handle:
  - `handle`
  - `alias`
  - `priority`
  - `enabled`

- `workflow.watchdog(endpoint, options)` defines endpoint wakeups:
  - `handle`
  - `queue`
  - `enabled`
  - `intervalSeconds`
  - `invocationPrompt`
  - `policy` (`"skip"` or `"queue"`)
  - `maxWakeups`

Canvas points are optional. Use `{ x, y }` for nodes/endpoints. Use `{ points: [{ x, y }] }` for edge waypoints. If canvas coordinates are absent, the kernel applies the session canvas auto-layout service during apply.

## Canvas Contract

Use coordinate space `workflow-canvas-v1`. Workflow nodes are `232 x 96`, endpoints are `180 x 78`, and generated exit markers are `120 x 72` at `node.x + 268`, `node.y + 28` for nodes that can complete the run. Keep at least `36` canvas units between nodes, endpoints, and exit markers. A safe node grid is `320` horizontally and `160` vertically; place entry endpoints at `node.x - 220`, `node.y` unless you have a reason to custom-place them. Call `arroba.meta.workflow_code.canvas_contract` when you need the authoritative current dimensions programmatically.

If no queues are defined, the kernel creates the workflow default prompt queue and returns it in `queue_ids`. Define queues only when the workflow needs named priorities or disabled queues. Watchdogs reference endpoint and optional queue handles, not runtime ids. Use `queue: "default"` when a watchdog should target the implicit default queue while the script also defines other queues. The queue handle `default` is reserved for the kernel default queue; only use that handle when the queue alias also normalizes to `default`.

Workflow, endpoint, and queue aliases use the same rules as manual workflow commands: after trimming and lowercase normalization, aliases must contain only lowercase letters, digits, `-`, or `_`. Queue aliases must be unique after normalization; define at most one queue with alias `default` when you need to configure the implicit default queue.

The kernel validates workflow-code against TOML-backed limits before apply: `workflow.code.max_concurrent`, `max_nodes`, `max_agents`, `max_edges`, `max_endpoints`, `max_queues`, `max_watchdogs`, `max_schema_bytes`, and `max_generated_prompt_bytes`.

Workflow-code scripts only define workflow structure. They do not call `run` or `enqueue` during compilation. To invoke the generated workflow, call `arroba.meta.workflow_code.run` with `endpoint`, optional `queue`, and `prompt`; `endpoint` and `queue` may be script handles returned by `workflow.endpoint` and `workflow.queue`, and the kernel maps them to generated runtime ids before normal workflow scheduling.

## Schemas and Outputs

Define workflow schemas directly in the script so the artifact is portable. Use `workflow.schema` handles for:

- `workflow.define({ runOutputSchema })` for final output.
- `workflow.define({ intermediateOutputSchema })` for workflow-level intermediate output.
- `workflow.node({ intermediateOutputSchema })` for a node-specific intermediate output.
- `workflow.edge(..., { handoffSchema })` for edge handoff validation.

Use `workflow.schemaFromFile({ handle, path, alias, description })` only when the JSON file is part of the workspace or imported package context available to the compiler. The file path must be relative, stay inside the approved import root, end in `.json`, and obey the configured schema byte limit.

Agents complete routed fan-out by emitting final fenced JSON containing `workflow_handoffs`. Each item selects an edge by real `edge_id` or a target by real `to_node_id`. The kernel resolves real ids during apply/run and exposes them in runtime context and apply reports.

For multi-edge nodes, the runtime prompt lists outgoing edge contracts with real `edge_id`, `to_node_id`, target public label, any target node instruction excerpt as `target_instructions`, handoff schema ref, and validation policy. Use that contract instead of guessing ids or reading workflow metadata from the workspace.

## Extensions

Use node `extensions` when the node's agent needs MCP, skill, script, connector, credential-backed access, or other extension grants supported by Arroba extension definitions. Keep extension grants on the node that needs them; generated agents receive those grants during apply, and existing agents receive them when the binding is authorized.

Extension grant shape:

```js
extensions: [
  { kind: "skill", name: "workflow-code-skill" },
  { kind: "mcp", name: "repo-tools" },
  { kind: "script", name: "release-script", environment: "node" },
  { kind: "connector", name: "linear", credential: "linear-api", maxSafety: "read" },
]
```

Supported `kind` values are `"mcp"`, `"skill"`, `"script"`, and `"connector"`. Script grants must include `environment`. Connector grants may include `credential` and `maxSafety`; the kernel rejects unavailable or invalid extension requirements before applying the workflow.

## Existing Agents and Portability

Prefer `workflow.newAgent` when a script should be portable across users and kernels. Use `workflow.existingAgent(agentRef)` only when the caller intentionally binds a session-local agent; exported scripts with existing-agent refs are not portable unless the target kernel has a compatible agent ref and authorization. In metaagent apply/run, the bound existing agent must already be controlled by that metaagent.

Provider/model choices on `workflow.newAgent` are preferences. When importing or applying on another kernel, use `provider_rebindings` keyed by node handle:

```json
[
  { "node": "planner", "provider": "dev-stub", "model": "default" },
  { "node": "reviewer", "provider": "codex", "model": "gpt-5", "effort": "high" }
]
```

Do not include runtime ids in provider rebindings. Do not rebind existing-agent nodes.

## Metaagent Tool Flow

1. Author the script.
2. Call `arroba.meta.workflow_code.validate` with `source`, or save it with `arroba.meta.workflow_code.create` and validate by `name`. Include `language: "typescript"` when the inline or saved source uses TypeScript syntax.
3. Call `arroba.meta.workflow_code.apply` to add the generated workflow to the session, or `arroba.meta.workflow_code.run` to apply and invoke an endpoint in one step. `run` may pass `endpoint`, `queue`, and `prompt`; endpoint and queue values may be script handles, and when it omits or blanks `prompt`, the script-level `workflow.define({ prompt })` value is used.
4. Inspect the apply report. It contains `workflow_id`, `schema_refs`, `node_ids`, `edge_ids`, `endpoint_ids`, `queue_ids`, `watchdog_ids`, and `agent_ids`.
5. Use `arroba.meta.workflow_code.package_export` and `arroba.meta.workflow_code.package_import` to exchange portable workflow-code packages across kernels. Use `arroba.meta.workflow_code.source_export` for a single source file and `arroba.meta.workflow_code.source_export_directory` for a source-directory package with `workflow.js`, `schemas/*.json`, and `manifest.json`. `source_export_dir` remains a compatibility alias.

Use `provider_rebindings` with apply/run when a generated-agent provider, model, effort, or account profile is unavailable or should be replaced in the target kernel. Rebindings target node handles, not generated runtime ids, and can only rebind nodes using `workflow.newAgent`.

Exported workflow-code packages include `source_sha256` and `definition_sha256`. Import verifies both before saving the artifact, then validates the compiled definition against the target kernel limits and feature version. Source-directory export includes file hashes in `manifest.json`; validate the directory before applying it on another kernel. This preserves portability for embedded and external schema files while rejecting tampered or inconsistent packages.

Generated runtime ids are never authored in the script. The script provides stable handles; the apply/run result returns the generated `workflow_id`, `schema_refs`, `node_ids`, `edge_ids`, `endpoint_ids`, `queue_ids`, `watchdog_ids`, and `agent_ids` maps keyed by those handles.

## Registered Workflows

Use the workflow registry when the user asks for a known reusable workflow, a standard topology, or a workflow that should be shared by name. Registry lookup order is workspace, then user, then builtin. Builtin entries include the canonical dynamic workflow patterns such as `prompt-chaining`, `routing`, `fan-out-synthesize`, `parallelization`, `adversarial-verification`, `generate-filter`, `tournament`, `loop-until-done`, `orchestrator-workers`, and `evaluator-optimizer`.

Preferred registry flow:

1. Call `arroba.meta.workflow_registry.list` or `arroba.meta.workflow_registry.get` to check whether a suitable workflow already exists.
2. If it fits, call `arroba.meta.workflow_registry.load` to add it to the current session, or `arroba.meta.workflow_registry.run` to load and invoke it in one step. Use `provider_rebindings` when the registered workflow names a provider/model/account profile unavailable in this kernel.
3. If no entry fits, author workflow-code, validate it, and either apply/run it directly or register it with `arroba.meta.workflow_registry.add` for reuse.
4. Use `arroba.meta.workflow_registry.add_from_workflow` only after a live workflow is already correct and should become reusable. It exports portable generated-agent bindings by default; use `agent_mode: "existing_agents"` only for intentionally non-portable local workflows.

Registry scripts are still generators. Every load/run creates a fresh workflow with fresh graph ids and generated agents unless the registered source intentionally uses authorized existing-agent refs. Do not author runtime ids in registry source.

## Small Routing Example

This script creates a 1-2-1 workflow: a router fans out to two workers, both converge into a synthesizer, and the synthesizer can loop back to the router or produce final output.

```js
workflow.define({
  alias: "toy-router-loop",
  maxConcurrent: 32,
});

const handoff = workflow.schema({
  handle: "handoff",
  alias: "Handoff",
  schema: {
    type: "object",
    required: ["summary"],
    properties: {
      summary: { type: "string" },
      confidence: { type: "number" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Final output",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
      evidence: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const router = workflow.node({
  handle: "router",
  agent: workflow.newAgent({ alias: "router", provider: "codex", model: "default" }),
  publicLabel: "Router",
  instructions: "Route the task to the best worker, or both workers when useful. Emit workflow_handoffs selecting the intended outgoing edge ids.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 120 },
});

const workerA = workflow.node({
  handle: "worker_a",
  agent: workflow.newAgent({ alias: "worker-a", provider: "claude", model: "default" }),
  publicLabel: "Worker A",
  instructions: "Solve the task from angle A and hand off concise findings.",
  canvas: { x: 280, y: 40 },
});

const workerB = workflow.node({
  handle: "worker_b",
  agent: workflow.newAgent({ alias: "worker-b", provider: "opencode", model: "default" }),
  publicLabel: "Worker B",
  instructions: "Solve the task from angle B and hand off concise findings.",
  canvas: { x: 280, y: 200 },
});

const synth = workflow.node({
  handle: "synth",
  agent: workflow.newAgent({ alias: "synth", provider: "codex", model: "default" }),
  publicLabel: "Synthesizer",
  instructions: "Synthesize worker findings. If more routing is needed, hand off to the router loop edge; otherwise submit final output matching the final_output schema.",
  canCompleteWorkflowRun: true,
  waitForAllInputs: true,
  canvas: { x: 560, y: 120 },
});

workflow.edge(router, workerA, { handle: "router_to_a", handoffSchema: handoff });
workflow.edge(router, workerB, { handle: "router_to_b", handoffSchema: handoff });
workflow.edge(workerA, synth, { handle: "a_to_synth", handoffSchema: handoff });
workflow.edge(workerB, synth, { handle: "b_to_synth", handoffSchema: handoff });
workflow.edge(synth, router, { handle: "synth_loop", handoffSchema: handoff });

workflow.endpoint(router, {
  handle: "entry",
  alias: "entry",
  canvas: { x: -220, y: 120 },
});
```

## Dynamic Pattern Starters

Read `workflows/workflow-code-patterns` for canonical, kernel-compiled scripts covering prompt chaining, routing, parallelization, fan-out and synthesize, adversarial verification, generate and filter, tournament, loop until done, orchestrator-workers, and evaluator-optimizer.

Keep toy drills small. For prompt chaining use two generated agents and one edge. For routing use one router and two workers. For parallel fan-out use one planner, two workers, and one synthesizer. For adversarial workflows use one proposer and one critic with a loop edge plus a finalizer when needed. For tournament workflows use a seeder, two contestants, and one judge when a single endpoint must start both branches. For evaluator-optimizer workflows use one optimizer and one evaluator with a loop edge. For orchestrator-worker workflows use one orchestrator, one or two workers, and one synthesizer.

Use a mixture of providers in drills that have at least three generated nodes. Apply-time `provider_rebindings` should be tested with the same script on a kernel where one provider/model choice is unavailable.

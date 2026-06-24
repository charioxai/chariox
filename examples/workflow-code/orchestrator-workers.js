workflow.define({
  alias: "pattern-orchestrator-workers",
  maxConcurrent: 32,
});

const assignment = workflow.schema({
  handle: "assignment",
  alias: "Assignment",
  schema: {
    type: "object",
    required: ["subtask"],
    properties: {
      subtask: { type: "string" },
      acceptance_criteria: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const result = workflow.schema({
  handle: "result",
  alias: "Worker result",
  schema: {
    type: "object",
    required: ["result"],
    properties: {
      result: { type: "string" },
      open_questions: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Orchestrated output",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
      delegated: { type: "boolean" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const orchestrator = workflow.node({
  handle: "orchestrator",
  agent: workflow.newAgent({ alias: "orchestrator", provider: "codex", model: "default" }),
  publicLabel: "Orchestrator",
  instructions: "Decompose the task and hand a focused assignment to the worker.",
  canvas: { x: 0, y: 100 },
});

const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "orchestrated-worker", provider: "opencode", model: "default" }),
  publicLabel: "Worker",
  instructions: "Complete the assigned subtask and hand the result to the synthesizer.",
  canvas: { x: 280, y: 100 },
});

const synthesizer = workflow.node({
  handle: "synthesizer",
  agent: workflow.newAgent({ alias: "orchestrated-synthesizer", provider: "claude", model: "default" }),
  publicLabel: "Synthesizer",
  instructions: "Combine the orchestration context and worker result into final output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 560, y: 100 },
});

workflow.edge(orchestrator, worker, { handle: "orchestrator_to_worker", handoffSchema: assignment });
workflow.edge(worker, synthesizer, { handle: "worker_to_synthesizer", handoffSchema: result });
workflow.endpoint(orchestrator, { handle: "entry", alias: "entry", canvas: { x: -180, y: 100 } });

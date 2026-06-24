workflow.define({
  alias: "pattern-loop-until-done",
  maxConcurrent: 32,
});

const workProduct = workflow.schema({
  handle: "work_product",
  alias: "Work product",
  schema: {
    type: "object",
    required: ["artifact"],
    properties: {
      artifact: { type: "string" },
      iteration: { type: "integer" },
    },
    additionalProperties: false,
  },
});

const feedback = workflow.schema({
  handle: "feedback",
  alias: "Checker feedback",
  schema: {
    type: "object",
    required: ["status", "notes"],
    properties: {
      status: { enum: ["revise", "done"] },
      notes: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Accepted result",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
      iterations: { type: "integer" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "loop-worker", provider: "codex", model: "default" }),
  publicLabel: "Worker",
  instructions: "Create or revise the artifact, then hand it to the checker.",
  maxTurns: 6,
  canvas: { x: 0, y: 100 },
});

const checker = workflow.node({
  handle: "checker",
  agent: workflow.newAgent({ alias: "checker", provider: "claude", model: "default" }),
  publicLabel: "Checker",
  instructions: "If work is insufficient, route feedback back to the worker. If accepted, submit final output.",
  canCompleteWorkflowRun: true,
  maxTurns: 6,
  canvas: { x: 300, y: 100 },
});

workflow.edge(worker, checker, { handle: "work_to_checker", handoffSchema: workProduct, validationPolicy: "halt" });
workflow.edge(checker, worker, { handle: "revise_loop", handoffSchema: feedback, validationPolicy: "warn" });
workflow.endpoint(worker, { handle: "entry", alias: "entry", canvas: { x: -180, y: 100 } });

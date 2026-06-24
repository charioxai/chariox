workflow.define({
  alias: "pattern-evaluator-optimizer",
  maxConcurrent: 32,
});

const candidate = workflow.schema({
  handle: "candidate",
  alias: "Candidate",
  schema: {
    type: "object",
    required: ["candidate"],
    properties: {
      candidate: { type: "string" },
      version: { type: "integer" },
    },
    additionalProperties: false,
  },
});

const evaluation = workflow.schema({
  handle: "evaluation",
  alias: "Evaluation",
  schema: {
    type: "object",
    required: ["decision", "feedback"],
    properties: {
      decision: { enum: ["accept", "revise"] },
      feedback: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Optimized answer",
  schema: {
    type: "object",
    required: ["answer", "accepted"],
    properties: {
      answer: { type: "string" },
      accepted: { type: "boolean" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const optimizer = workflow.node({
  handle: "optimizer",
  agent: workflow.newAgent({ alias: "optimizer", provider: "codex", model: "default" }),
  publicLabel: "Optimizer",
  instructions: "Produce an improved candidate and hand it to the evaluator.",
  maxTurns: 6,
  canvas: { x: 0, y: 100 },
});

const evaluator = workflow.node({
  handle: "evaluator",
  agent: workflow.newAgent({ alias: "evaluator", provider: "claude", model: "default" }),
  publicLabel: "Evaluator",
  instructions: "Evaluate the candidate. Route feedback back for revision or submit final output when accepted.",
  canCompleteWorkflowRun: true,
  maxTurns: 6,
  canvas: { x: 300, y: 100 },
});

workflow.edge(optimizer, evaluator, { handle: "candidate_to_evaluator", handoffSchema: candidate, validationPolicy: "halt" });
workflow.edge(evaluator, optimizer, { handle: "revision_loop", handoffSchema: evaluation, validationPolicy: "warn" });
workflow.endpoint(optimizer, { handle: "entry", alias: "entry", canvas: { x: -180, y: 100 } });

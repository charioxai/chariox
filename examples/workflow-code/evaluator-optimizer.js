const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      optimizer_count: {
        type: "integer",
        minimum: 1,
        maximum: 12,
        default: 1,
        title: "Optimizer count",
        description: "Number of optimizer candidates evaluated in parallel.",
      },
      max_iterations: {
        type: "integer",
        minimum: 1,
        maximum: 24,
        default: 6,
        title: "Max iterations",
        description: "Maximum optimizer/evaluator revision loop budget.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-evaluator-optimizer",
  maxConcurrent: Math.max(32, params.optimizer_count + 1),
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

function pad2(value) {
  return String(value).padStart(2, "0");
}

function branchY(index, count) {
  return (index - (count - 1) / 2) * 160 + 120;
}

const centerY = 120;
const coordinator = workflow.node({
  handle: "coordinator",
  agent: workflow.newAgent({ alias: "evaluator-optimizer-coordinator", provider: "codex", model: "default" }),
  publicLabel: "Coordinator",
  instructions: `Start ${params.optimizer_count} optimizer candidate stream${params.optimizer_count === 1 ? "" : "s"} for evaluation.`,
  canvas: { x: 0, y: centerY },
});

const evaluator = workflow.node({
  handle: "evaluator",
  agent: workflow.newAgent({ alias: "evaluator", provider: "claude", model: "sonnet" }),
  publicLabel: "Evaluator",
  instructions: `Evaluate ${params.optimizer_count} candidate stream${params.optimizer_count === 1 ? "" : "s"}. Route feedback back for revision or submit final output when accepted.`,
  canCompleteWorkflowRun: true,
  waitForAllInputs: params.optimizer_count > 1,
  maxTurns: params.max_iterations,
  canvas: { x: 620, y: centerY },
});

for (let index = 0; index < params.optimizer_count; index += 1) {
  const number = index + 1;
  const handle = params.optimizer_count === 1 ? "optimizer" : `optimizer_${pad2(number)}`;
  const optimizer = workflow.node({
    handle,
    agent: workflow.newAgent({ alias: handle, provider: "codex", model: "default" }),
    publicLabel: params.optimizer_count === 1 ? "Optimizer" : `Optimizer ${number}`,
    instructions: "Produce an improved candidate and hand it to the evaluator.",
    maxTurns: params.max_iterations,
    canvas: { x: 300, y: branchY(index, params.optimizer_count) },
  });
  workflow.edge(coordinator, optimizer, { handle: `coordinator_to_${handle}` });
  workflow.edge(optimizer, evaluator, { handle: `${handle}_to_evaluator`, handoffSchema: candidate, validationPolicy: "halt" });
  workflow.edge(evaluator, optimizer, { handle: `revision_loop_${handle}`, handoffSchema: evaluation, validationPolicy: "warn" });
}

workflow.endpoint(coordinator, { handle: "entry", alias: "entry", canvas: { x: -220, y: centerY } });

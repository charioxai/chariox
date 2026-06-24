workflow.define({
  alias: "pattern-fan-out-synthesize",
  maxConcurrent: 32,
});

const assignment = workflow.schema({
  handle: "assignment",
  alias: "Worker assignment",
  schema: {
    type: "object",
    required: ["question", "angle"],
    properties: {
      question: { type: "string" },
      angle: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finding = workflow.schema({
  handle: "finding",
  alias: "Worker finding",
  schema: {
    type: "object",
    required: ["finding"],
    properties: {
      finding: { type: "string" },
      evidence: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Synthesis",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
      source_count: { type: "integer" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "fanout-planner", provider: "codex", model: "default" }),
  publicLabel: "Planner",
  instructions: "Split the request into two complementary worker assignments and hand off to both outgoing worker edges.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 120 },
});

const workerA = workflow.node({
  handle: "worker_a",
  agent: workflow.newAgent({ alias: "worker-a", provider: "claude", model: "default" }),
  publicLabel: "Worker A",
  instructions: "Work your assigned angle and hand findings to the synthesizer.",
  canvas: { x: 260, y: 40 },
});

const workerB = workflow.node({
  handle: "worker_b",
  agent: workflow.newAgent({ alias: "worker-b", provider: "opencode", model: "default" }),
  publicLabel: "Worker B",
  instructions: "Work your assigned angle and hand findings to the synthesizer.",
  canvas: { x: 260, y: 200 },
});

const synthesizer = workflow.node({
  handle: "synthesizer",
  agent: workflow.newAgent({ alias: "synthesizer", provider: "codex", model: "default" }),
  publicLabel: "Synthesizer",
  instructions: "Wait for all worker inputs, synthesize them, and submit final output.",
  canCompleteWorkflowRun: true,
  waitForAllInputs: true,
  canvas: { x: 560, y: 120 },
});

workflow.edge(planner, workerA, { handle: "planner_to_a", handoffSchema: assignment });
workflow.edge(planner, workerB, { handle: "planner_to_b", handoffSchema: assignment });
workflow.edge(workerA, synthesizer, { handle: "a_to_synth", handoffSchema: finding });
workflow.edge(workerB, synthesizer, { handle: "b_to_synth", handoffSchema: finding });
workflow.endpoint(planner, { handle: "entry", alias: "entry", canvas: { x: -180, y: 120 } });

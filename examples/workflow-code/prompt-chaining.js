workflow.define({
  alias: "pattern-prompt-chaining",
  maxConcurrent: 32,
});

const handoff = workflow.schema({
  handle: "handoff",
  alias: "Draft handoff",
  schema: {
    type: "object",
    required: ["draft"],
    properties: {
      draft: { type: "string" },
      notes: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Final answer",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
      changes: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const drafter = workflow.node({
  handle: "drafter",
  agent: workflow.newAgent({ alias: "drafter", provider: "codex", model: "default" }),
  publicLabel: "Drafter",
  instructions: "Create a first complete draft and hand it to the refiner.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 80 },
});

const refiner = workflow.node({
  handle: "refiner",
  agent: workflow.newAgent({ alias: "refiner", provider: "claude", model: "default" }),
  publicLabel: "Refiner",
  instructions: "Refine the draft and submit final output that matches final_output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 280, y: 80 },
});

workflow.edge(drafter, refiner, { handle: "draft_to_refiner", handoffSchema: handoff });
workflow.endpoint(drafter, { handle: "entry", alias: "entry", canvas: { x: -220, y: 80 } });

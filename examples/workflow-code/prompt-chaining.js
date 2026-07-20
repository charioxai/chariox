const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      refinement_passes: {
        type: "integer",
        minimum: 1,
        maximum: 12,
        default: 1,
        title: "Refinement passes",
        description: "Number of sequential refinement nodes after the drafter.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-prompt-chaining",
  maxConcurrent: Math.max(32, params.refinement_passes + 1),
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

function pad2(value) {
  return String(value).padStart(2, "0");
}

const y = 80;
const drafter = workflow.node({
  handle: "drafter",
  agent: workflow.newAgent({ alias: "drafter", provider: "codex", model: "default" }),
  publicLabel: "Drafter",
  instructions: "Create a first complete draft and hand it to the refinement chain.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y },
});

let previous = drafter;
for (let index = 0; index < params.refinement_passes; index += 1) {
  const pass = index + 1;
  const isFinal = pass === params.refinement_passes;
  const handle = params.refinement_passes === 1 ? "refiner" : `refiner_${pad2(pass)}`;
  const refiner = workflow.node({
    handle,
    agent: workflow.newAgent({
      alias: handle.replaceAll("_", "-"),
      provider: index % 2 === 0 ? "claude" : "opencode",
      model: index % 2 === 0 ? "haiku" : "deepseek-v4-pro",
    }),
    publicLabel: params.refinement_passes === 1 ? "Refiner" : `Refiner ${pass}`,
    instructions: isFinal
      ? "Refine the draft and submit final output that matches final_output."
      : "Refine the draft and hand it to the next refiner.",
    canCompleteWorkflowRun: isFinal,
    canvas: { x: 280 + index * 280, y },
  });
  workflow.edge(previous, refiner, { handle: `${previous.handle}_to_${handle}`, handoffSchema: handoff });
  previous = refiner;
}

workflow.endpoint(drafter, { handle: "entry", alias: "entry", canvas: { x: -220, y } });

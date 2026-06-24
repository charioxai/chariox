workflow.define({
  alias: "pattern-generate-filter",
  maxConcurrent: 32,
});

const candidates = workflow.schema({
  handle: "candidates",
  alias: "Candidates",
  schema: {
    type: "object",
    required: ["items"],
    properties: {
      items: {
        type: "array",
        items: {
          type: "object",
          required: ["value"],
          properties: { value: { type: "string" }, score: { type: "number" } },
          additionalProperties: false,
        },
      },
    },
    additionalProperties: false,
  },
});

const filtered = workflow.schema({
  handle: "filtered",
  alias: "Filtered candidates",
  schema: {
    type: "object",
    required: ["selected"],
    properties: {
      selected: { type: "array", items: { type: "string" } },
      rationale: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Filtered result",
  schema: {
    type: "object",
    required: ["result"],
    properties: {
      result: { type: "string" },
      selected_count: { type: "integer" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const generator = workflow.node({
  handle: "generator",
  agent: workflow.newAgent({ alias: "generator", provider: "codex", model: "default" }),
  publicLabel: "Generator",
  instructions: "Generate diverse candidate solutions and hand them to the filter.",
  canvas: { x: 0, y: 100 },
});

const filter = workflow.node({
  handle: "filter",
  agent: workflow.newAgent({ alias: "filter", provider: "claude", model: "default" }),
  publicLabel: "Filter",
  instructions: "Filter the candidate list and hand the selected options to the finisher.",
  canvas: { x: 280, y: 100 },
});

const finisher = workflow.node({
  handle: "finisher",
  agent: workflow.newAgent({ alias: "finisher", provider: "opencode", model: "default" }),
  publicLabel: "Finisher",
  instructions: "Turn the filtered candidates into the final result.",
  canCompleteWorkflowRun: true,
  canvas: { x: 560, y: 100 },
});

workflow.edge(generator, filter, { handle: "generated_candidates", handoffSchema: candidates });
workflow.edge(filter, finisher, { handle: "filtered_candidates", handoffSchema: filtered });
workflow.endpoint(generator, { handle: "entry", alias: "entry", canvas: { x: -180, y: 100 } });

const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      generator_count: {
        type: "integer",
        minimum: 1,
        maximum: 12,
        default: 1,
        title: "Generator count",
        description: "Number of candidate generators before filtering.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-generate-filter",
  maxConcurrent: Math.max(32, params.generator_count + 2),
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

function pad2(value) {
  return String(value).padStart(2, "0");
}

function branchY(index, count) {
  return (index - (count - 1) / 2) * 160 + 120;
}

const centerY = 120;
const coordinator = workflow.node({
  handle: "coordinator",
  agent: workflow.newAgent({ alias: "generate-filter-coordinator", provider: "codex", model: "default" }),
  publicLabel: "Coordinator",
  instructions: `Route this request through every outgoing workflow edge to the ${params.generator_count} preconfigured candidate generator${params.generator_count === 1 ? "" : "s"}. Do not create or spawn provider-native agents.`,
  canvas: { x: 0, y: centerY },
});

const filter = workflow.node({
  handle: "filter",
  agent: workflow.newAgent({ alias: "filter", provider: "claude", model: "haiku" }),
  publicLabel: "Filter",
  instructions: `Filter candidate lists from ${params.generator_count} generator${params.generator_count === 1 ? "" : "s"} and hand selected options to the finisher.`,
  waitForAllInputs: params.generator_count > 1,
  canvas: { x: 600, y: centerY },
});

const finisher = workflow.node({
  handle: "finisher",
  agent: workflow.newAgent({ alias: "finisher", provider: "opencode", model: "default" }),
  publicLabel: "Finisher",
  instructions: "Turn the filtered candidates into the final result.",
  canCompleteWorkflowRun: true,
  canvas: { x: 900, y: centerY },
});

for (let index = 0; index < params.generator_count; index += 1) {
  const number = index + 1;
  const handle = params.generator_count === 1 ? "generator" : `generator_${pad2(number)}`;
  const generator = workflow.node({
    handle,
    agent: workflow.newAgent({ alias: handle, provider: "codex", model: "default" }),
    publicLabel: params.generator_count === 1 ? "Generator" : `Generator ${number}`,
    instructions: "Generate diverse candidate solutions and hand them to the filter.",
    canvas: { x: 300, y: branchY(index, params.generator_count) },
  });
  workflow.edge(coordinator, generator, { handle: `coordinator_to_${handle}` });
  workflow.edge(generator, filter, { handle: `${handle}_candidates`, handoffSchema: candidates });
}

workflow.edge(filter, finisher, { handle: "filtered_candidates", handoffSchema: filtered });
workflow.endpoint(coordinator, { handle: "entry", alias: "entry", canvas: { x: -220, y: centerY } });

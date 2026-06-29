const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      worker_count: {
        type: "integer",
        minimum: 2,
        maximum: 12,
        default: 2,
        title: "Worker count",
        description: "Number of parallel research workers before synthesis.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-fan-out-synthesize",
  maxConcurrent: Math.max(32, params.worker_count + 2),
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

function pad2(value) {
  return String(value).padStart(2, "0");
}

function branchY(index, count) {
  return (index - (count - 1) / 2) * 160 + 160;
}

const centerY = 160;
const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "fanout-planner", provider: "codex", model: "default" }),
  publicLabel: "Planner",
  instructions: `Split the request into ${params.worker_count} complementary worker assignments and hand off to every worker edge.`,
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: centerY },
});

const synthesizer = workflow.node({
  handle: "synthesizer",
  agent: workflow.newAgent({ alias: "synthesizer", provider: "codex", model: "default" }),
  publicLabel: "Synthesizer",
  instructions: `Wait for all ${params.worker_count} worker inputs, synthesize them, and submit final output.`,
  canCompleteWorkflowRun: true,
  waitForAllInputs: true,
  canvas: { x: 620, y: centerY },
});

for (let index = 0; index < params.worker_count; index += 1) {
  const number = index + 1;
  const defaultHandles = ["worker_a", "worker_b"];
  const handle = params.worker_count === 2 ? defaultHandles[index] : `worker_${pad2(number)}`;
  const worker = workflow.node({
    handle,
    agent: workflow.newAgent({ alias: handle.replaceAll("_", "-"), provider: index % 2 === 0 ? "claude" : "opencode", model: "default" }),
    publicLabel: params.worker_count === 2 ? `Worker ${number === 1 ? "A" : "B"}` : `Worker ${number}`,
    instructions: "Work your assigned angle and hand findings to the synthesizer.",
    canvas: { x: 300, y: branchY(index, params.worker_count) },
  });
  workflow.edge(planner, worker, { handle: `planner_to_${handle}`, handoffSchema: assignment });
  workflow.edge(worker, synthesizer, { handle: `${handle}_to_synthesizer`, handoffSchema: finding });
}

workflow.endpoint(planner, { handle: "entry", alias: "entry", canvas: { x: -220, y: centerY } });

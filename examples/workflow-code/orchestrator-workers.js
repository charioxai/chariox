const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      worker_count: {
        type: "integer",
        minimum: 1,
        maximum: 12,
        default: 1,
        title: "Worker count",
        description: "Number of delegated workers before synthesis.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-orchestrator-workers",
  maxConcurrent: Math.max(32, params.worker_count + 2),
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

function pad2(value) {
  return String(value).padStart(2, "0");
}

function branchY(index, count) {
  return (index - (count - 1) / 2) * 160 + 140;
}

const centerY = 140;
const orchestrator = workflow.node({
  handle: "orchestrator",
  agent: workflow.newAgent({ alias: "orchestrator", provider: "codex", model: "default" }),
  publicLabel: "Orchestrator",
  instructions: `Decompose the task and hand focused assignments to ${params.worker_count} worker node${params.worker_count === 1 ? "" : "s"}.`,
  canvas: { x: 0, y: centerY },
});

const synthesizer = workflow.node({
  handle: "synthesizer",
  agent: workflow.newAgent({ alias: "orchestrated-synthesizer", provider: "claude", model: "sonnet" }),
  publicLabel: "Synthesizer",
  instructions: `Combine orchestration context and all ${params.worker_count} worker result${params.worker_count === 1 ? "" : "s"} into final output.`,
  canCompleteWorkflowRun: true,
  waitForAllInputs: params.worker_count > 1,
  canvas: { x: 620, y: centerY },
});

for (let index = 0; index < params.worker_count; index += 1) {
  const number = index + 1;
  const handle = params.worker_count === 1 ? "worker" : `worker_${pad2(number)}`;
  const worker = workflow.node({
    handle,
    agent: workflow.newAgent({ alias: handle === "worker" ? "orchestrated-worker" : handle.replaceAll("_", "-"), provider: "opencode", model: "default" }),
    publicLabel: params.worker_count === 1 ? "Worker" : `Worker ${number}`,
    instructions: "Complete the assigned subtask and hand the result to the synthesizer.",
    canvas: { x: 300, y: branchY(index, params.worker_count) },
  });
  workflow.edge(orchestrator, worker, { handle: `orchestrator_to_${handle}`, handoffSchema: assignment });
  workflow.edge(worker, synthesizer, { handle: `${handle}_to_synthesizer`, handoffSchema: result });
}

workflow.endpoint(orchestrator, { handle: "entry", alias: "entry", canvas: { x: -220, y: centerY } });

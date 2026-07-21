const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      specialist_count: {
        type: "integer",
        minimum: 2,
        maximum: 12,
        default: 2,
        title: "Specialist count",
        description: "Number of specialist branches the classifier can route to.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-routing",
  maxConcurrent: Math.max(32, params.specialist_count + 1),
});

const routeTask = workflow.schema({
  handle: "route_task",
  alias: "Routed task",
  schema: {
    type: "object",
    required: ["task", "reason", "specialist"],
    properties: {
      task: { type: "string" },
      reason: { type: "string" },
      specialist: { type: "integer" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Specialist answer",
  schema: {
    type: "object",
    required: ["answer", "specialist"],
    properties: {
      answer: { type: "string" },
      specialist: { type: "integer" },
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
const classifier = workflow.node({
  handle: "classifier",
  agent: workflow.newAgent({ alias: "classifier", provider: "codex", model: "default" }),
  publicLabel: "Classifier",
  instructions: `Classify the request and emit workflow_handoffs to exactly one of ${params.specialist_count} specialist edges.`,
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: centerY },
});

for (let index = 0; index < params.specialist_count; index += 1) {
  const number = index + 1;
  const defaultHandles = ["code_specialist", "research_specialist"];
  const defaultLabels = ["Code specialist", "Research specialist"];
  const handle = params.specialist_count === 2 ? defaultHandles[index] : `specialist_${pad2(number)}`;
  const specialist = workflow.node({
    handle,
    agent: workflow.newAgent({
      alias: handle.replaceAll("_", "-"),
      provider: index % 2 === 0 ? "opencode" : "claude",
      model: index % 2 === 0 ? "kimi-k2.6" : "sonnet",
    }),
    publicLabel: params.specialist_count === 2 ? defaultLabels[index] : `Specialist ${number}`,
    instructions: `Answer tasks routed to specialist branch ${number} and submit final output.`,
    canCompleteWorkflowRun: true,
    canvas: { x: 320, y: branchY(index, params.specialist_count) },
  });
  workflow.edge(classifier, specialist, {
    handle: `to_${handle}`,
    handoffSchema: routeTask,
    validationPolicy: "halt",
  });
}

workflow.endpoint(classifier, { handle: "entry", alias: "entry", canvas: { x: -220, y: centerY } });

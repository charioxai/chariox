const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      reviewer_count: {
        type: "integer",
        minimum: 2,
        maximum: 12,
        default: 2,
        title: "Reviewer count",
        description: "Number of independent reviewers before aggregation.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-parallelization",
  maxConcurrent: Math.max(32, params.reviewer_count + 2),
});

const reviewTask = workflow.schema({
  handle: "review_task",
  alias: "Parallel review task",
  schema: {
    type: "object",
    required: ["subject", "criteria"],
    properties: {
      subject: { type: "string" },
      criteria: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const reviewResult = workflow.schema({
  handle: "review_result",
  alias: "Parallel review result",
  schema: {
    type: "object",
    required: ["verdict", "notes"],
    properties: {
      verdict: { enum: ["pass", "fail", "needs_review"] },
      notes: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Aggregated parallel decision",
  schema: {
    type: "object",
    required: ["decision", "rationale"],
    properties: {
      decision: { enum: ["pass", "fail", "needs_review"] },
      rationale: { type: "string" },
      reviewer_count: { type: "integer" },
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
const dispatcher = workflow.node({
  handle: "dispatcher",
  agent: workflow.newAgent({ alias: "parallel-dispatcher", provider: "codex", model: "default" }),
  publicLabel: "Dispatcher",
  instructions: `Send the same review task to ${params.reviewer_count} independent reviewer edges.`,
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: centerY },
});

const aggregator = workflow.node({
  handle: "aggregator",
  agent: workflow.newAgent({ alias: "parallel-aggregator", provider: "codex", model: "default" }),
  publicLabel: "Aggregator",
  instructions: `Wait for all ${params.reviewer_count} reviewer results, aggregate their votes, and submit final output.`,
  canCompleteWorkflowRun: true,
  waitForAllInputs: true,
  canvas: { x: 620, y: centerY },
});

for (let index = 0; index < params.reviewer_count; index += 1) {
  const number = index + 1;
  const handle = `reviewer_${pad2(number)}`;
  const provider = index % 2 === 0 ? "claude" : "opencode";
  const reviewer = workflow.node({
    handle,
    agent: workflow.newAgent({ alias: handle.replaceAll("_", "-"), provider, model: provider === "claude" ? "haiku" : "deepseek-v4-pro" }),
    publicLabel: `Reviewer ${number}`,
    instructions: "Review the task independently, then hand structured notes to the aggregator.",
    canvas: { x: 300, y: branchY(index, params.reviewer_count) },
  });
  workflow.edge(dispatcher, reviewer, { handle: `dispatcher_to_${handle}`, handoffSchema: reviewTask });
  workflow.edge(reviewer, aggregator, { handle: `${handle}_to_aggregator`, handoffSchema: reviewResult });
}

workflow.endpoint(dispatcher, { handle: "entry", alias: "entry", canvas: { x: -220, y: centerY } });

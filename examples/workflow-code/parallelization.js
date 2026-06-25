workflow.define({
  alias: "pattern-parallelization",
  maxConcurrent: 32,
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

const dispatcher = workflow.node({
  handle: "dispatcher",
  agent: workflow.newAgent({ alias: "parallel-dispatcher", provider: "codex", model: "default" }),
  publicLabel: "Dispatcher",
  instructions: "Send the same review task to both independent reviewer edges.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 120 },
});

const policyReviewer = workflow.node({
  handle: "policy_reviewer",
  agent: workflow.newAgent({ alias: "policy-reviewer", provider: "claude", model: "default" }),
  publicLabel: "Policy reviewer",
  instructions: "Review the task from the policy and constraints angle, then hand structured notes to the aggregator.",
  canvas: { x: 280, y: 40 },
});

const qualityReviewer = workflow.node({
  handle: "quality_reviewer",
  agent: workflow.newAgent({ alias: "quality-reviewer", provider: "opencode", model: "default" }),
  publicLabel: "Quality reviewer",
  instructions: "Review the task from the quality and completeness angle, then hand structured notes to the aggregator.",
  canvas: { x: 280, y: 200 },
});

const aggregator = workflow.node({
  handle: "aggregator",
  agent: workflow.newAgent({ alias: "parallel-aggregator", provider: "codex", model: "default" }),
  publicLabel: "Aggregator",
  instructions: "Wait for both reviewer results, aggregate their votes, and submit final output.",
  canCompleteWorkflowRun: true,
  waitForAllInputs: true,
  canvas: { x: 600, y: 120 },
});

workflow.edge(dispatcher, policyReviewer, { handle: "to_policy", handoffSchema: reviewTask });
workflow.edge(dispatcher, qualityReviewer, { handle: "to_quality", handoffSchema: reviewTask });
workflow.edge(policyReviewer, aggregator, { handle: "policy_to_aggregator", handoffSchema: reviewResult });
workflow.edge(qualityReviewer, aggregator, { handle: "quality_to_aggregator", handoffSchema: reviewResult });
workflow.endpoint(dispatcher, { handle: "entry", alias: "entry", canvas: { x: -180, y: 120 } });

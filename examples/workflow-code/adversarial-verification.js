workflow.define({
  alias: "pattern-adversarial-verification",
  maxConcurrent: 32,
});

const proposal = workflow.schema({
  handle: "proposal",
  alias: "Proposal",
  schema: {
    type: "object",
    required: ["claim"],
    properties: {
      claim: { type: "string" },
      evidence: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const critique = workflow.schema({
  handle: "critique",
  alias: "Critique",
  schema: {
    type: "object",
    required: ["issues", "recommendation"],
    properties: {
      issues: { type: "array", items: { type: "string" } },
      recommendation: { enum: ["revise", "judge"] },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Judgment",
  schema: {
    type: "object",
    required: ["decision", "rationale"],
    properties: {
      decision: { enum: ["accept", "reject", "revise"] },
      rationale: { type: "string" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const proposer = workflow.node({
  handle: "proposer",
  agent: workflow.newAgent({ alias: "proposer", provider: "codex", model: "default" }),
  publicLabel: "Proposer",
  instructions: "Produce a proposal with evidence and hand it to the critic.",
  maxTurns: 4,
  canvas: { x: 0, y: 120 },
});

const critic = workflow.node({
  handle: "critic",
  agent: workflow.newAgent({ alias: "critic", provider: "claude", model: "default" }),
  publicLabel: "Critic",
  instructions: "Find flaws. Route back to proposer for revision or forward to judge when ready.",
  canvas: { x: 280, y: 120 },
});

const judge = workflow.node({
  handle: "judge",
  agent: workflow.newAgent({ alias: "judge", provider: "opencode", model: "default" }),
  publicLabel: "Judge",
  instructions: "Decide whether the proposal survives critique and submit final output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 560, y: 120 },
});

workflow.edge(proposer, critic, { handle: "proposal_to_critic", handoffSchema: proposal });
workflow.edge(critic, proposer, { handle: "critic_loop", handoffSchema: critique, validationPolicy: "warn" });
workflow.edge(critic, judge, { handle: "critic_to_judge", handoffSchema: critique, validationPolicy: "halt" });
workflow.endpoint(proposer, { handle: "entry", alias: "entry", canvas: { x: -180, y: 120 } });

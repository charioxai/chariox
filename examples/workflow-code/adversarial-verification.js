const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      critic_count: {
        type: "integer",
        minimum: 1,
        maximum: 12,
        default: 1,
        title: "Critic count",
        description: "Number of independent critics reviewing the proposal before judgment.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-adversarial-verification",
  maxConcurrent: Math.max(32, params.critic_count + 2),
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

function pad2(value) {
  return String(value).padStart(2, "0");
}

function branchY(index, count) {
  return (index - (count - 1) / 2) * 160 + 140;
}

const centerY = 140;
const proposer = workflow.node({
  handle: "proposer",
  agent: workflow.newAgent({ alias: "proposer", provider: "codex", model: "default" }),
  publicLabel: "Proposer",
  instructions: `Produce a proposal with evidence and hand it to ${params.critic_count} critic${params.critic_count === 1 ? "" : "s"}.`,
  canvas: { x: 0, y: centerY },
});

const judge = workflow.node({
  handle: "judge",
  agent: workflow.newAgent({ alias: "judge", provider: "opencode", model: "kimi-k2.6" }),
  publicLabel: "Judge",
  instructions: `Decide whether the proposal survives critique from ${params.critic_count} critic${params.critic_count === 1 ? "" : "s"} and submit final output.`,
  canCompleteWorkflowRun: true,
  waitForAllInputs: params.critic_count > 1,
  canvas: { x: 620, y: centerY },
});

for (let index = 0; index < params.critic_count; index += 1) {
  const number = index + 1;
  const handle = params.critic_count === 1 ? "critic" : `critic_${pad2(number)}`;
  const critic = workflow.node({
    handle,
    agent: workflow.newAgent({ alias: handle, provider: "claude", model: "sonnet" }),
    publicLabel: params.critic_count === 1 ? "Critic" : `Critic ${number}`,
    instructions: "Find flaws, set recommendation to revise or judge, and hand the critique to Judge.",
    canvas: { x: 300, y: branchY(index, params.critic_count) },
  });
  workflow.edge(proposer, critic, { handle: `proposal_to_${handle}`, handoffSchema: proposal });
  workflow.edge(critic, judge, { handle: `${handle}_to_judge`, handoffSchema: critique, validationPolicy: "halt" });
}

workflow.endpoint(proposer, { handle: "entry", alias: "entry", canvas: { x: -220, y: centerY } });

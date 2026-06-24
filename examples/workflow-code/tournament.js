workflow.define({
  alias: "pattern-tournament",
  maxConcurrent: 32,
});

const contestPrompt = workflow.schema({
  handle: "contest_prompt",
  alias: "Contest prompt",
  schema: {
    type: "object",
    required: ["task", "slot"],
    properties: {
      task: { type: "string" },
      slot: { enum: ["a", "b"] },
    },
    additionalProperties: false,
  },
});

const entry = workflow.schema({
  handle: "entry",
  alias: "Contest entry",
  schema: {
    type: "object",
    required: ["answer", "strategy"],
    properties: {
      answer: { type: "string" },
      strategy: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Tournament result",
  schema: {
    type: "object",
    required: ["winner", "reason"],
    properties: {
      winner: { enum: ["a", "b", "tie"] },
      reason: { type: "string" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const seeder = workflow.node({
  handle: "seeder",
  agent: workflow.newAgent({ alias: "seeder", provider: "codex", model: "default" }),
  publicLabel: "Seeder",
  instructions: "Send the same task to both contestants with distinct slots.",
  canvas: { x: 0, y: 120 },
});

const contestantA = workflow.node({
  handle: "contestant_a",
  agent: workflow.newAgent({ alias: "contestant-a", provider: "claude", model: "default" }),
  publicLabel: "Contestant A",
  instructions: "Produce a tournament entry for slot a.",
  canvas: { x: 280, y: 40 },
});

const contestantB = workflow.node({
  handle: "contestant_b",
  agent: workflow.newAgent({ alias: "contestant-b", provider: "opencode", model: "default" }),
  publicLabel: "Contestant B",
  instructions: "Produce a tournament entry for slot b.",
  canvas: { x: 280, y: 200 },
});

const judge = workflow.node({
  handle: "judge",
  agent: workflow.newAgent({ alias: "tournament-judge", provider: "codex", model: "default" }),
  publicLabel: "Judge",
  instructions: "Wait for both entries, compare them, and submit final output.",
  canCompleteWorkflowRun: true,
  waitForAllInputs: true,
  canvas: { x: 560, y: 120 },
});

workflow.edge(seeder, contestantA, { handle: "seed_a", handoffSchema: contestPrompt });
workflow.edge(seeder, contestantB, { handle: "seed_b", handoffSchema: contestPrompt });
workflow.edge(contestantA, judge, { handle: "entry_a", handoffSchema: entry });
workflow.edge(contestantB, judge, { handle: "entry_b", handoffSchema: entry });
workflow.endpoint(seeder, { handle: "entry", alias: "entry", canvas: { x: -180, y: 120 } });

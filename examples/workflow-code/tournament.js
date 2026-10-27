const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      bracket_size: {
        type: "integer",
        minimum: 2,
        xPowerOfTwo: true,
        default: 2,
        title: "Bracket size",
        description: "Number of contestants. Must be a power of two; graph size is still bounded by kernel workflow limits.",
      },
    },
    additionalProperties: false,
  },
});

const bracketSize = params.bracket_size;

workflow.define({
  alias: "pattern-tournament",
  maxConcurrent: Math.max(32, bracketSize),
});

const contestPrompt = workflow.schema({
  handle: "contest_prompt",
  alias: "Contest prompt",
  schema: {
    type: "object",
    required: ["task", "slot"],
    properties: {
      task: { type: "string" },
      slot: { type: "integer" },
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
      winner: { type: "string" },
      reason: { type: "string" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

function pad2(value) {
  return String(value).padStart(2, "0");
}

function rowY(index) {
  return index * 160;
}

const seederY = ((bracketSize - 1) * 160) / 2;
const seeder = workflow.node({
  handle: "seeder",
  agent: workflow.newAgent({ alias: "seeder", provider: "codex", model: "default" }),
  publicLabel: "Seeder",
  instructions: `Send the same task to ${bracketSize} contestants with numbered slots.`,
  canvas: { x: 0, y: seederY },
});

let currentRound = [];
for (let index = 0; index < bracketSize; index += 1) {
  const slot = index + 1;
  const handle = `contestant_${pad2(slot)}`;
  const contestant = workflow.node({
    handle,
    agent: workflow.newAgent({
      alias: handle.replaceAll("_", "-"),
      provider: index % 2 === 0 ? "claude" : "opencode",
      model: index % 2 === 0 ? "sonnet" : "kimi-k2.6",
    }),
    publicLabel: `Contestant ${slot}`,
    instructions: `Produce a tournament entry for slot ${slot}.`,
    canvas: { x: 300, y: rowY(index) },
  });
  workflow.edge(seeder, contestant, { handle: `seed_${pad2(slot)}`, handoffSchema: contestPrompt });
  currentRound.push({ node: contestant, y: rowY(index), label: `slot ${slot}` });
}

let round = 1;
while (currentRound.length > 1) {
  const nextRound = [];
  const isFinalRound = currentRound.length === 2;
  for (let matchIndex = 0; matchIndex < currentRound.length; matchIndex += 2) {
    const matchNumber = matchIndex / 2 + 1;
    const left = currentRound[matchIndex];
    const right = currentRound[matchIndex + 1];
    const y = (left.y + right.y) / 2;
    const handle = isFinalRound ? "final_judge" : `round_${round}_judge_${pad2(matchNumber)}`;
    const judge = workflow.node({
      handle,
      agent: workflow.newAgent({ alias: handle.replaceAll("_", "-"), provider: "codex", model: "default" }),
      publicLabel: isFinalRound ? "Final judge" : `Round ${round} judge ${matchNumber}`,
      instructions: isFinalRound
        ? "Wait for both finalist entries, select the tournament winner, and submit final output."
        : "Wait for both entries, select the winner for the next round, and hand that result forward.",
      canCompleteWorkflowRun: isFinalRound,
      waitForAllInputs: true,
      canvas: { x: 620 + (round - 1) * 320, y },
    });
    workflow.edge(left.node, judge, { handle: `${left.node.handle}_to_${handle}`, handoffSchema: entry });
    workflow.edge(right.node, judge, { handle: `${right.node.handle}_to_${handle}`, handoffSchema: entry });
    nextRound.push({ node: judge, y, label: `${left.label} vs ${right.label}` });
  }
  currentRound = nextRound;
  round += 1;
}

workflow.endpoint(seeder, { handle: "entry", alias: "entry", canvas: { x: -220, y: seederY } });

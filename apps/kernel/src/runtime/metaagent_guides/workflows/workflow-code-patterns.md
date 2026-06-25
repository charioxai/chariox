# Workflow-Code Pattern Examples

This guide exposes the canonical workflow-code scripts for the dynamic workflow pattern suite. The kernel compiles every script in this guide in the workflow-code unit tests, so these examples are the preferred starting points for metaagents that need to create portable workflows.

Use these as templates, then validate the edited source with `arroba.meta.workflow_code.validate` before applying or running it. If a target kernel lacks a requested provider or model, use apply/run `provider_rebindings` keyed by node handle.

The suite maps the Anthropic workflow vocabulary to Arroba workflow-code primitives: prompt chaining, routing, parallelization, orchestrator-workers, and evaluator-optimizer come from Anthropic's "Building effective agents"; adversarial verification, tournament, generate/filter, and loop-until-done cover the Claude Code dynamic-workflow shapes used for wide parallel work, independent verification, comparison, and iterative convergence.

All examples follow `workflow-canvas-v1`: nodes are `232 x 96`, endpoints are `180 x 78`, generated exit markers are `120 x 72` at `node.x + 268`, `node.y + 28`, and explicit boxes keep at least `36` canvas units of separation. Use `arroba.meta.workflow_code.canvas_contract` for the authoritative runtime contract before designing custom layouts.

References:
- https://www.anthropic.com/engineering/building-effective-agents
- https://code.claude.com/docs/en/workflows

## Prompt Chaining

Path: `examples/workflow-code/prompt-chaining.js`

```js
workflow.define({
  alias: "pattern-prompt-chaining",
  maxConcurrent: 32,
});

const handoff = workflow.schema({
  handle: "handoff",
  alias: "Draft handoff",
  schema: {
    type: "object",
    required: ["draft"],
    properties: {
      draft: { type: "string" },
      notes: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Final answer",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
      changes: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const drafter = workflow.node({
  handle: "drafter",
  agent: workflow.newAgent({ alias: "drafter", provider: "codex", model: "default" }),
  publicLabel: "Drafter",
  instructions: "Create a first complete draft and hand it to the refiner.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 80 },
});

const refiner = workflow.node({
  handle: "refiner",
  agent: workflow.newAgent({ alias: "refiner", provider: "claude", model: "default" }),
  publicLabel: "Refiner",
  instructions: "Refine the draft and submit final output that matches final_output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 280, y: 80 },
});

workflow.edge(drafter, refiner, { handle: "draft_to_refiner", handoffSchema: handoff });
workflow.endpoint(drafter, { handle: "entry", alias: "entry", canvas: { x: -220, y: 80 } });
```

## Routing

Path: `examples/workflow-code/routing.js`

```js
workflow.define({
  alias: "pattern-routing",
  maxConcurrent: 32,
});

const routeTask = workflow.schema({
  handle: "route_task",
  alias: "Routed task",
  schema: {
    type: "object",
    required: ["task", "reason"],
    properties: {
      task: { type: "string" },
      reason: { type: "string" },
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
      specialist: { enum: ["code", "research"] },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const classifier = workflow.node({
  handle: "classifier",
  agent: workflow.newAgent({ alias: "classifier", provider: "codex", model: "default" }),
  publicLabel: "Classifier",
  instructions: "Classify the request and emit workflow_handoffs to exactly one specialist edge.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 120 },
});

const codeSpecialist = workflow.node({
  handle: "code_specialist",
  agent: workflow.newAgent({ alias: "code-specialist", provider: "opencode", model: "default" }),
  publicLabel: "Code specialist",
  instructions: "Answer code, build, repository, or implementation tasks and submit final output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 300, y: 40 },
});

const researchSpecialist = workflow.node({
  handle: "research_specialist",
  agent: workflow.newAgent({ alias: "research-specialist", provider: "claude", model: "default" }),
  publicLabel: "Research specialist",
  instructions: "Answer analysis, summarization, and research tasks and submit final output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 300, y: 200 },
});

workflow.edge(classifier, codeSpecialist, {
  handle: "to_code",
  handoffSchema: routeTask,
  validationPolicy: "halt",
});
workflow.edge(classifier, researchSpecialist, {
  handle: "to_research",
  handoffSchema: routeTask,
  validationPolicy: "halt",
});
workflow.endpoint(classifier, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
```

## Fan-Out And Synthesize

Path: `examples/workflow-code/fan-out-synthesize.js`

```js
workflow.define({
  alias: "pattern-fan-out-synthesize",
  maxConcurrent: 32,
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

const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "fanout-planner", provider: "codex", model: "default" }),
  publicLabel: "Planner",
  instructions: "Split the request into two complementary worker assignments and hand off to both outgoing worker edges.",
  canCompleteWorkflowRun: false,
  canvas: { x: 0, y: 120 },
});

const workerA = workflow.node({
  handle: "worker_a",
  agent: workflow.newAgent({ alias: "worker-a", provider: "claude", model: "default" }),
  publicLabel: "Worker A",
  instructions: "Work your assigned angle and hand findings to the synthesizer.",
  canvas: { x: 280, y: 40 },
});

const workerB = workflow.node({
  handle: "worker_b",
  agent: workflow.newAgent({ alias: "worker-b", provider: "opencode", model: "default" }),
  publicLabel: "Worker B",
  instructions: "Work your assigned angle and hand findings to the synthesizer.",
  canvas: { x: 280, y: 200 },
});

const synthesizer = workflow.node({
  handle: "synthesizer",
  agent: workflow.newAgent({ alias: "synthesizer", provider: "codex", model: "default" }),
  publicLabel: "Synthesizer",
  instructions: "Wait for all worker inputs, synthesize them, and submit final output.",
  canCompleteWorkflowRun: true,
  waitForAllInputs: true,
  canvas: { x: 560, y: 120 },
});

workflow.edge(planner, workerA, { handle: "planner_to_a", handoffSchema: assignment });
workflow.edge(planner, workerB, { handle: "planner_to_b", handoffSchema: assignment });
workflow.edge(workerA, synthesizer, { handle: "a_to_synth", handoffSchema: finding });
workflow.edge(workerB, synthesizer, { handle: "b_to_synth", handoffSchema: finding });
workflow.endpoint(planner, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
```

## Parallelization

Path: `examples/workflow-code/parallelization.js`

```js
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
workflow.endpoint(dispatcher, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
```

## Adversarial Verification

Path: `examples/workflow-code/adversarial-verification.js`

```js
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
workflow.endpoint(proposer, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
```

## Generate And Filter

Path: `examples/workflow-code/generate-filter.js`

```js
workflow.define({
  alias: "pattern-generate-filter",
  maxConcurrent: 32,
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

const generator = workflow.node({
  handle: "generator",
  agent: workflow.newAgent({ alias: "generator", provider: "codex", model: "default" }),
  publicLabel: "Generator",
  instructions: "Generate diverse candidate solutions and hand them to the filter.",
  canvas: { x: 0, y: 100 },
});

const filter = workflow.node({
  handle: "filter",
  agent: workflow.newAgent({ alias: "filter", provider: "claude", model: "default" }),
  publicLabel: "Filter",
  instructions: "Filter the candidate list and hand the selected options to the finisher.",
  canvas: { x: 280, y: 100 },
});

const finisher = workflow.node({
  handle: "finisher",
  agent: workflow.newAgent({ alias: "finisher", provider: "opencode", model: "default" }),
  publicLabel: "Finisher",
  instructions: "Turn the filtered candidates into the final result.",
  canCompleteWorkflowRun: true,
  canvas: { x: 560, y: 100 },
});

workflow.edge(generator, filter, { handle: "generated_candidates", handoffSchema: candidates });
workflow.edge(filter, finisher, { handle: "filtered_candidates", handoffSchema: filtered });
workflow.endpoint(generator, { handle: "entry", alias: "entry", canvas: { x: -220, y: 100 } });
```

## Tournament

Path: `examples/workflow-code/tournament.js`

```js
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
workflow.endpoint(seeder, { handle: "entry", alias: "entry", canvas: { x: -220, y: 120 } });
```

## Loop Until Done

Path: `examples/workflow-code/loop-until-done.js`

```js
workflow.define({
  alias: "pattern-loop-until-done",
  maxConcurrent: 32,
});

const workProduct = workflow.schema({
  handle: "work_product",
  alias: "Work product",
  schema: {
    type: "object",
    required: ["artifact"],
    properties: {
      artifact: { type: "string" },
      iteration: { type: "integer" },
    },
    additionalProperties: false,
  },
});

const feedback = workflow.schema({
  handle: "feedback",
  alias: "Checker feedback",
  schema: {
    type: "object",
    required: ["status", "notes"],
    properties: {
      status: { enum: ["revise", "done"] },
      notes: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Accepted result",
  schema: {
    type: "object",
    required: ["answer"],
    properties: {
      answer: { type: "string" },
      iterations: { type: "integer" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "loop-worker", provider: "codex", model: "default" }),
  publicLabel: "Worker",
  instructions: "Create or revise the artifact, then hand it to the checker.",
  maxTurns: 6,
  canvas: { x: 0, y: 100 },
});

const checker = workflow.node({
  handle: "checker",
  agent: workflow.newAgent({ alias: "checker", provider: "claude", model: "default" }),
  publicLabel: "Checker",
  instructions: "If work is insufficient, route feedback back to the worker. If accepted, submit final output.",
  canCompleteWorkflowRun: true,
  maxTurns: 6,
  canvas: { x: 300, y: 100 },
});

workflow.edge(worker, checker, { handle: "work_to_checker", handoffSchema: workProduct, validationPolicy: "halt" });
workflow.edge(checker, worker, { handle: "revise_loop", handoffSchema: feedback, validationPolicy: "warn" });
workflow.endpoint(worker, { handle: "entry", alias: "entry", canvas: { x: -220, y: 100 } });
```

## Orchestrator-Workers

Path: `examples/workflow-code/orchestrator-workers.js`

```js
workflow.define({
  alias: "pattern-orchestrator-workers",
  maxConcurrent: 32,
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

const orchestrator = workflow.node({
  handle: "orchestrator",
  agent: workflow.newAgent({ alias: "orchestrator", provider: "codex", model: "default" }),
  publicLabel: "Orchestrator",
  instructions: "Decompose the task and hand a focused assignment to the worker.",
  canvas: { x: 0, y: 100 },
});

const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "orchestrated-worker", provider: "opencode", model: "default" }),
  publicLabel: "Worker",
  instructions: "Complete the assigned subtask and hand the result to the synthesizer.",
  canvas: { x: 280, y: 100 },
});

const synthesizer = workflow.node({
  handle: "synthesizer",
  agent: workflow.newAgent({ alias: "orchestrated-synthesizer", provider: "claude", model: "default" }),
  publicLabel: "Synthesizer",
  instructions: "Combine the orchestration context and worker result into final output.",
  canCompleteWorkflowRun: true,
  canvas: { x: 560, y: 100 },
});

workflow.edge(orchestrator, worker, { handle: "orchestrator_to_worker", handoffSchema: assignment });
workflow.edge(worker, synthesizer, { handle: "worker_to_synthesizer", handoffSchema: result });
workflow.endpoint(orchestrator, { handle: "entry", alias: "entry", canvas: { x: -220, y: 100 } });
```

## Evaluator-Optimizer

Path: `examples/workflow-code/evaluator-optimizer.js`

```js
workflow.define({
  alias: "pattern-evaluator-optimizer",
  maxConcurrent: 32,
});

const candidate = workflow.schema({
  handle: "candidate",
  alias: "Candidate",
  schema: {
    type: "object",
    required: ["candidate"],
    properties: {
      candidate: { type: "string" },
      version: { type: "integer" },
    },
    additionalProperties: false,
  },
});

const evaluation = workflow.schema({
  handle: "evaluation",
  alias: "Evaluation",
  schema: {
    type: "object",
    required: ["decision", "feedback"],
    properties: {
      decision: { enum: ["accept", "revise"] },
      feedback: { type: "string" },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Optimized answer",
  schema: {
    type: "object",
    required: ["answer", "accepted"],
    properties: {
      answer: { type: "string" },
      accepted: { type: "boolean" },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const optimizer = workflow.node({
  handle: "optimizer",
  agent: workflow.newAgent({ alias: "optimizer", provider: "codex", model: "default" }),
  publicLabel: "Optimizer",
  instructions: "Produce an improved candidate and hand it to the evaluator.",
  maxTurns: 6,
  canvas: { x: 0, y: 100 },
});

const evaluator = workflow.node({
  handle: "evaluator",
  agent: workflow.newAgent({ alias: "evaluator", provider: "claude", model: "default" }),
  publicLabel: "Evaluator",
  instructions: "Evaluate the candidate. Route feedback back for revision or submit final output when accepted.",
  canCompleteWorkflowRun: true,
  maxTurns: 6,
  canvas: { x: 300, y: 100 },
});

workflow.edge(optimizer, evaluator, { handle: "candidate_to_evaluator", handoffSchema: candidate, validationPolicy: "halt" });
workflow.edge(evaluator, optimizer, { handle: "revision_loop", handoffSchema: evaluation, validationPolicy: "warn" });
workflow.endpoint(optimizer, { handle: "entry", alias: "entry", canvas: { x: -220, y: 100 } });
```

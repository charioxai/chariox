const params = workflow.parameters({
  schema: {
    type: "object",
    properties: {
      max_review_cycles_per_step: {
        type: "integer",
        minimum: 1,
        maximum: 24,
        default: 6,
        title: "Max review cycles per step",
        description: "Soft review loop budget for one planner assignment before the reviewer routes the state back to the planner.",
      },
    },
    additionalProperties: false,
  },
});

workflow.define({
  alias: "pattern-planner-worker-reviewer",
  maxConcurrent: 32,
  prompt: "Use the planner to decompose the goal, the worker to implement each assignment, and the reviewer to verify each implementation before the planner decides whether the whole goal is complete.",
});

const implementationAssignment = workflow.schema({
  handle: "implementation_assignment",
  alias: "Implementation assignment",
  schema: {
    type: "object",
    required: ["step_id", "task", "context", "acceptance_criteria"],
    properties: {
      step_id: { type: "string" },
      task: { type: "string" },
      context: { type: "string" },
      acceptance_criteria: { type: "array", items: { type: "string" } },
      constraints: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const implementationResult = workflow.schema({
  handle: "implementation_result",
  alias: "Implementation result",
  schema: {
    type: "object",
    required: ["step_id", "summary", "changed_files", "verification"],
    properties: {
      step_id: { type: "string" },
      summary: { type: "string" },
      changed_files: { type: "array", items: { type: "string" } },
      verification: { type: "array", items: { type: "string" } },
      open_questions: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const revisionRequest = workflow.schema({
  handle: "revision_request",
  alias: "Revision request",
  schema: {
    type: "object",
    required: ["step_id", "review_iteration", "issues", "required_changes", "reasoning"],
    properties: {
      step_id: { type: "string" },
      review_iteration: { type: "integer", minimum: 1 },
      issues: { type: "array", items: { type: "string" } },
      required_changes: { type: "array", items: { type: "string" } },
      reasoning: { type: "string" },
    },
    additionalProperties: false,
  },
});

const acceptedStepReport = workflow.schema({
  handle: "accepted_step_report",
  alias: "Accepted step report",
  schema: {
    type: "object",
    required: ["step_id", "review_iterations", "accepted", "summary", "verification"],
    properties: {
      step_id: { type: "string" },
      review_iterations: { type: "integer", minimum: 1 },
      accepted: { type: "boolean" },
      summary: { type: "string" },
      verification: { type: "array", items: { type: "string" } },
      remaining_risks: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

const finalOutput = workflow.schema({
  handle: "final_output",
  alias: "Goal result",
  schema: {
    type: "object",
    required: ["completed", "summary", "implemented_steps", "verification"],
    properties: {
      completed: { type: "boolean" },
      summary: { type: "string" },
      implemented_steps: { type: "array", items: { type: "string" } },
      verification: { type: "array", items: { type: "string" } },
      remaining_risks: { type: "array", items: { type: "string" } },
    },
    additionalProperties: false,
  },
});

workflow.define({ runOutputSchema: finalOutput });

const planner = workflow.node({
  handle: "planner",
  agent: workflow.newAgent({ alias: "goal-planner", provider: "codex", model: "default" }),
  publicLabel: "Planner",
  instructions: `You own the full user goal and are the only node allowed to finish the workflow.

Maintain a concise plan. Send one focused implementation assignment at a time to the worker on the planner_to_worker edge using the implementation_assignment schema. Include the step_id, concrete task, relevant context, acceptance criteria, and constraints.

When the reviewer sends an accepted step report on reviewer_to_planner, decide whether the whole goal is complete. If the goal is complete, submit final output. If more work remains, send the next focused assignment to the worker on planner_to_worker.

Do not route directly to the reviewer. Do not finish until the original goal has been achieved.`,
  canCompleteWorkflowRun: true,
  canvas: { x: 0, y: 140 },
});

const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "goal-worker", provider: "codex", model: "default" }),
  publicLabel: "Worker",
  instructions: `Implement exactly the current planner assignment.

After each implementation attempt, send the result to the reviewer on the worker_to_reviewer edge using the implementation_result schema. Include changed files or artifacts, verification performed, and open questions.

If the reviewer routes revision feedback back to you on reviewer_to_worker, apply the requested changes and send a new implementation result to the reviewer. Do not submit final workflow output.`,
  canvas: { x: 460, y: 140 },
});

const reviewer = workflow.node({
  handle: "reviewer",
  agent: workflow.newAgent({ alias: "goal-reviewer", provider: "claude", model: "default" }),
  publicLabel: "Reviewer",
  instructions: `Review only the worker's latest implementation result for the current planner assignment.

If concrete required changes remain and the current assignment has not reached ${params.max_review_cycles_per_step} review cycle${params.max_review_cycles_per_step === 1 ? "" : "s"}, route feedback back to the worker on reviewer_to_worker using the revision_request schema.

If the implementation is acceptable, route an accepted step report to the planner on reviewer_to_planner using the accepted_step_report schema.

If this is the ${params.max_review_cycles_per_step} review cycle for the current assignment, route to reviewer_to_planner even if issues remain. In that accepted step report, set accepted according to your judgment and include unresolved issues in remaining_risks so the planner can decide the next assignment.

Do not submit final workflow output. Do not route directly to planner unless you are reporting an accepted step or returning control after the review-cycle limit.`,
  canvas: { x: 920, y: 140 },
});

workflow.edge(planner, worker, {
  handle: "planner_to_worker",
  handoffSchema: implementationAssignment,
  validationPolicy: "halt",
});
workflow.edge(worker, reviewer, {
  handle: "worker_to_reviewer",
  handoffSchema: implementationResult,
  validationPolicy: "halt",
});
workflow.edge(reviewer, worker, {
  handle: "reviewer_to_worker",
  handoffSchema: revisionRequest,
  validationPolicy: "warn",
});
workflow.edge(reviewer, planner, {
  handle: "reviewer_to_planner",
  handoffSchema: acceptedStepReport,
  validationPolicy: "halt",
});

workflow.endpoint(planner, { handle: "entry", alias: "entry", canvas: { x: -220, y: 140 } });

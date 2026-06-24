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
workflow.endpoint(classifier, { handle: "entry", alias: "entry", canvas: { x: -180, y: 120 } });

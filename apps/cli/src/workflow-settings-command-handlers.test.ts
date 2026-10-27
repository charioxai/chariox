import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeAttachment,
  RuntimeSession,
  WorkflowDefinition,
} from "./cli-types.js"
import {
  handleWorkflowSettingsCommand,
  isWorkflowSettingsCommand,
  type WorkflowSettingsCommandContext,
  type WorkflowSettingsCommandDeps,
} from "./workflow-settings-command-handlers.js"

test("workflow settings command predicate recognizes settings subcommands", () => {
  assert.equal(isWorkflowSettingsCommand("flush-context"), true)
  assert.equal(isWorkflowSettingsCommand("run"), false)
  assert.equal(isWorkflowSettingsCommand(undefined), false)
})

test("workflow flush-context command reads and updates the selected workflow", async () => {
  const harness = createHarness({
    selectedWorkflowRef: "workflow-1",
    resolveWorkflow: async (workflowRef) => {
      harness.calls.push(`resolve:${workflowRef}`)
      return { workflow: workflow({ id: workflowRef, flush_agent_context_before_run: false }) }
    },
    setWorkflowFlushContext: async (workflowRef, value) => {
      harness.calls.push(`set-flush:${workflowRef}:${String(value)}`)
      return {
        workflow: workflow({ id: workflowRef, flush_agent_context_before_run: value }),
        session: session({ id: "session-flush" }),
      }
    },
  })

  await handleWorkflowSettingsCommand(harness.deps, harness.context, ["flush-context"])
  await handleWorkflowSettingsCommand(harness.deps, harness.context, ["flush-context", "true"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "footer:info:workflow workflow-1 flush-context: false",
    "resolve:workflow-1",
    "upsert:workflow-1",
    "set-flush:workflow-1:true",
    "apply:session-flush",
    "upsert:workflow-1",
    "footer:info:workflow workflow-1 flush-context set to true",
  ])
})

test("workflow schema settings read and update workflow schema refs", async () => {
  const harness = createHarness({
    selectedWorkflowRef: "workflow-1",
    setWorkflowRunOutputSchema: async (workflowRef, schemaRef) => {
      harness.calls.push(`set-run-schema:${workflowRef}:${schemaRef ?? "none"}`)
      return {
        workflow: workflow({ id: workflowRef, run_output_schema_ref: schemaRef }),
        session: session({ id: "session-schema" }),
      }
    },
  })

  await handleWorkflowSettingsCommand(harness.deps, harness.context, ["run-output-schema", "none"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "set-run-schema:workflow-1:none",
    "apply:session-schema",
    "upsert:workflow-1",
    "footer:info:workflow workflow-1 run-output-schema set to none",
  ])
})

test("workflow max-turns command reads and updates session config", async () => {
  const harness = createHarness({
    session: session({ config_state: { version: 1, values: { "workflow.max_turns": "4" } } }),
    updateSessionConfig: async (sessionId, attachmentId, values, requiresIdle) => {
      harness.calls.push(`update-config:${sessionId}:${attachmentId}:${values["workflow.max_turns"]}:${String(requiresIdle)}`)
      return { session: session({ id: "session-config" }), config: { version: 2, values } }
    },
  })

  await handleWorkflowSettingsCommand(harness.deps, harness.context, ["max-turns"])
  await handleWorkflowSettingsCommand(harness.deps, harness.context, ["max-turns", "off"])

  const detached = createHarness({ attachment: null })
  await handleWorkflowSettingsCommand(detached.deps, detached.context, ["max-turns", "3"])

  assert.deepEqual(harness.calls, [
    "footer:info:workflow max turns: 4",
    "update-config:session-1:attachment-1:0:false",
    "apply:session-config",
    "footer:info:workflow max turns disabled",
  ])
  assert.deepEqual(detached.calls, [
    "footer:error:must be attached to set workflow max turns",
  ])
})

type HarnessOptions = Partial<WorkflowSettingsCommandDeps> & {
  attachment?: RuntimeAttachment | null
  context?: Partial<WorkflowSettingsCommandContext>
  selectedWorkflowRef?: string | null
  session?: RuntimeSession
}

function createHarness(overrides: HarnessOptions) {
  const {
    attachment = { id: "attachment-1", session_id: "session-1" },
    context: contextOverrides,
    selectedWorkflowRef = null,
    session: currentSession = session(),
    ...depOverrides
  } = overrides
  const calls: string[] = []
  const deps: WorkflowSettingsCommandDeps = {
    sessionState: () => currentSession,
    attachmentState: () => attachment,
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    updateSessionConfig: async (sessionId, attachmentId, values, requiresIdle) => {
      calls.push(`update-config:${sessionId}:${attachmentId}:${values["workflow.max_turns"]}:${String(requiresIdle)}`)
      return {
        session: session({ id: sessionId, config_state: { version: 2, values } }),
        config: { version: 2, values },
      }
    },
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
    },
    resolveWorkflow: async (workflowRef) => {
      calls.push(`resolve:${workflowRef}`)
      return { workflow: workflow({ id: workflowRef }) }
    },
    upsertWorkflowDefinition: (nextWorkflow) => {
      calls.push(`upsert:${nextWorkflow.id}`)
    },
    setWorkflowFlushContext: async (workflowRef, value) => ({
      workflow: workflow({ id: workflowRef, flush_agent_context_before_run: value }),
      session: session({ id: "session-flush" }),
    }),
    setWorkflowRunOutputSchema: async (workflowRef, schemaRef) => ({
      workflow: workflow({ id: workflowRef, run_output_schema_ref: schemaRef }),
      session: session({ id: "session-schema" }),
    }),
    ...depOverrides,
  }
  const context: WorkflowSettingsCommandContext = {
    firstWorkflowArgIsExplicit: (workflowRef) => Boolean(workflowRef && workflowRef.startsWith("workflow-")),
    selectedWorkflowRef: () => selectedWorkflowRef,
    workflowRefOrSelected: (workflowRef) => workflowRef ?? selectedWorkflowRef,
    ...contextOverrides,
  }
  return { calls, context, deps }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 4,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: null,
    nodes: [],
    edges: [],
    endpoints: [],
    run_output_schema_ref: "run-schema",
    ...overrides,
  }
}

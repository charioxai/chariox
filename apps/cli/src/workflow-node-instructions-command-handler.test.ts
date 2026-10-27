import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import {
  handleWorkflowNodeInstructionsCommand,
  type WorkflowNodeInstructionsCommandContext,
  type WorkflowNodeInstructionsCommandDeps,
} from "./workflow-node-instructions-command-handler.js"

test("workflow node instructions show opens the existing instructions draft", async () => {
  const harness = createHarness()

  await handleWorkflowNodeInstructionsCommand(harness.deps, harness.context, ["node", "instructions", "show", "workflow-1", "node-1"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "open:workflow-1:node-1:old instructions",
    "select:workflow-1",
    "footer:info:opened node node-1 instructions in the I/O panel",
  ])
})

test("workflow node instructions set opens the editor or updates from a file", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "chariox-node-instructions-"))
  try {
    await writeFile(join(workspace, "instructions.md"), "file instructions", "utf8")
    const harness = createHarness({ workspace })

    await handleWorkflowNodeInstructionsCommand(harness.deps, harness.context, ["node", "instructions", "set", "workflow-1", "node-1"])
    await handleWorkflowNodeInstructionsCommand(harness.deps, harness.context, ["node", "instructions", "set", "workflow-1", "node-1", "instructions.md"])

    assert.deepEqual(harness.calls, [
      "resolve:workflow-1",
      "upsert:workflow-1",
      "open:workflow-1:node-1:old instructions",
      "select:workflow-1",
      "footer:info:editing node instructions in the I/O panel; submit text then /workflow node instructions save",
      "resolve:workflow-1",
      "upsert:workflow-1",
      "update:workflow-1:node-1:file instructions",
      "apply:session-1",
      "upsert:workflow-1",
      "footer:info:updated node instructions for node-1",
    ])
  } finally {
    await rm(workspace, { recursive: true, force: true })
  }
})

test("workflow node instructions save persists the open editor draft", async () => {
  const harness = createHarness({
    editorContext: { workflowId: "workflow-1", nodeId: "node-1" },
    editorDraft: "edited instructions",
  })

  await handleWorkflowNodeInstructionsCommand(harness.deps, harness.context, ["node", "instructions", "save"])

  assert.deepEqual(harness.calls, [
    "update:workflow-1:node-1:edited instructions",
    "apply:session-1",
    "upsert:workflow-1",
    "close",
    "footer:info:saved node instructions for node-1",
  ])
})

test("workflow node instructions command validates target, action, and editor state", async () => {
  const missingTarget = createHarness()
  await handleWorkflowNodeInstructionsCommand(missingTarget.deps, missingTarget.context, ["node", "instructions", "show"])

  const missingNode = createHarness({ workflow: workflow({ nodes: [] }) })
  await handleWorkflowNodeInstructionsCommand(missingNode.deps, missingNode.context, ["node", "instructions", "show", "workflow-1", "node-1"])

  const noEditor = createHarness({ editorContext: null })
  await handleWorkflowNodeInstructionsCommand(noEditor.deps, noEditor.context, ["node", "instructions", "save"])

  const unknownAction = createHarness()
  await handleWorkflowNodeInstructionsCommand(unknownAction.deps, unknownAction.context, ["node", "instructions", "edit", "workflow-1", "node-1"])

  assert.deepEqual(missingTarget.calls, [
    "footer:error:usage: /workflow node instructions show|set [workflow-ref] <node-id> [file]",
  ])
  assert.deepEqual(missingNode.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "footer:error:workflow node node-1 not found",
  ])
  assert.deepEqual(noEditor.calls, [
    "footer:error:no workflow node instructions editor is open",
  ])
  assert.deepEqual(unknownAction.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "footer:error:usage: /workflow node instructions show|set|save|close [workflow-ref] <node-id> [file]",
  ])
})

type HarnessOptions = Partial<WorkflowNodeInstructionsCommandDeps> & {
  context?: Partial<WorkflowNodeInstructionsCommandContext>
  editorContext?: { workflowId: string; nodeId: string } | null
  editorDraft?: string
  workflow?: WorkflowDefinition
  workspace?: string
}

function createHarness(options: HarnessOptions = {}) {
  const {
    context: contextOverrides,
    editorContext = null,
    editorDraft = "",
    workflow: currentWorkflow = workflow({ nodes: [node()] }),
    workspace = "/tmp",
    ...depOverrides
  } = options
  const calls: string[] = []
  const deps: WorkflowNodeInstructionsCommandDeps = {
    currentWorkspaceTarget: () => workspace,
    resolveWorkflow: async (workflowRef) => {
      calls.push(`resolve:${workflowRef}`)
      return { workflow: { ...currentWorkflow, id: workflowRef } }
    },
    upsertWorkflowDefinition: (nextWorkflow) => {
      calls.push(`upsert:${nextWorkflow.id}`)
    },
    updateWorkflowNodeInstructions: async (workflowRef, nodeId, instructions) => {
      calls.push(`update:${workflowRef}:${nodeId}:${instructions ?? "null"}`)
      return {
        node: node({ id: nodeId, instructions }),
        workflow: workflow({ id: workflowRef }),
        session: session(),
      }
    },
    openWorkflowNodeInstructionsEditor: (workflowId, nodeId, draft) => {
      calls.push(`open:${workflowId}:${nodeId}:${draft}`)
    },
    closeWorkflowNodeInstructionsEditor: () => {
      calls.push("close")
    },
    getWorkflowNodeInstructionsDraft: () => editorDraft,
    getWorkflowNodeInstructionsContext: () => editorContext,
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
    },
    selectWorkflowCanvas: (workflowId) => {
      calls.push(`select:${workflowId}`)
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    ...depOverrides,
  }
  const context: WorkflowNodeInstructionsCommandContext = {
    firstWorkflowArgIsExplicit: (workflowRef) => Boolean(workflowRef && workflowRef.startsWith("workflow-")),
    workflowRefOrSelected: (workflowRef) => workflowRef ?? "workflow-1",
    ...contextOverrides,
  }
  return { calls, context, deps }
}

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: null,
    nodes: [],
    edges: [],
    endpoints: [],
    ...overrides,
  }
}

function node(overrides: Partial<WorkflowNodeDefinition> = {}): WorkflowNodeDefinition {
  return {
    id: "node-1",
    agent_id: "agent-1",
    instructions: "old instructions",
    ...overrides,
  }
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

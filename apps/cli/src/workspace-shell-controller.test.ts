import assert from "node:assert/strict"
import test from "node:test"

import { createDefaultShellContext } from "@arroba/kernel-client/shell-core"

import type { RuntimeSession } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { submitWorkspaceShellCommand } from "./workspace-shell-controller.js"
import type { WorkspaceShellEntry } from "./workspace-shell.js"

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "s1",
    alias: null,
    status: "Active",
    workspace_id: "workspace",
    worktree_id: "worktree",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 1,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  } as RuntimeSession
}

test("submitWorkspaceShellCommand records shell output and refreshes selected workflow", async () => {
  const initialContext = createDefaultShellContext({ sessionId: "s1" })
  const nextContext = { ...initialContext, workflowId: "wf2" }
  const entries: WorkspaceShellEntry[] = []
  let storedContext = initialContext
  let appliedSession: RuntimeSession | null = null
  let selectedWorkflowId: string | null = null
  let selectedWorkflowNodeId: string | null | "unset" = "unset"
  let transcriptRebuilt = false
  let footerFlash: { message: string; tone: "info" | "error" } | null = null

  const result = await submitWorkspaceShellCommand("@ workflow list", {
    client: {} as LocalIpcClient,
    executeShellLine: async (line, context, _deps, write) => {
      assert.equal(line, "workflow list")
      assert.equal(context, initialContext)
      write?.("Workflow list\n")
      return { ok: true, context: nextContext }
    },
    workspaceShellContext: () => storedContext,
    setWorkspaceShellContext: (context) => {
      storedContext = context
    },
    nextEntryId: () => 7,
    setWorkspaceShellEntries: (updater) => {
      entries.splice(0, entries.length, ...updater(entries))
    },
    sessionState: () => session({ workflows: [{ id: "wf2", alias: "Next", nodes: [] }] }),
    refreshSessionState: async () => session({ workflows: [{ id: "wf2", alias: "Next", nodes: [] }] }),
    applySessionState: (nextSession) => {
      appliedSession = nextSession
    },
    selectedWorkflowId: () => selectedWorkflowId,
    setSelectedWorkflowId: (workflowId) => {
      selectedWorkflowId = workflowId
    },
    setSelectedWorkflowNodeId: (nodeId) => {
      selectedWorkflowNodeId = nodeId
    },
    rebuildTranscript: () => {
      transcriptRebuilt = true
    },
    flashFooter: (message, tone) => {
      footerFlash = { message, tone }
    },
  })

  assert.equal(result.ok, true)
  assert.equal(result.output, "Workflow list")
  assert.equal(storedContext, nextContext)
  assert.deepEqual(entries, [{ id: 7, command: "workflow list", output: "Workflow list", ok: true }])
  assert.equal((appliedSession as RuntimeSession | null)?.id, "s1")
  assert.equal(selectedWorkflowId, "wf2")
  assert.equal(selectedWorkflowNodeId, null)
  assert.equal(transcriptRebuilt, true)
  assert.deepEqual(footerFlash, { message: "shell command completed", tone: "info" })
})

test("submitWorkspaceShellCommand rejects empty shell marker before executing", async () => {
  const context = createDefaultShellContext()
  let footerFlash: { message: string; tone: "info" | "error" } | null = null

  const result = await submitWorkspaceShellCommand("@   ", {
    client: {} as LocalIpcClient,
    executeShellLine: async () => {
      throw new Error("should not execute")
    },
    workspaceShellContext: () => context,
    setWorkspaceShellContext: () => {},
    nextEntryId: () => 1,
    setWorkspaceShellEntries: () => {},
    sessionState: () => session(),
    refreshSessionState: async () => session(),
    applySessionState: () => {},
    selectedWorkflowId: () => null,
    setSelectedWorkflowId: () => {},
    setSelectedWorkflowNodeId: () => {},
    rebuildTranscript: () => {},
    flashFooter: (message, tone) => {
      footerFlash = { message, tone }
    },
  })

  assert.equal(result.ok, false)
  assert.equal(result.output, "usage: @ <arroba-shell command>")
  assert.equal(result.context, context)
  assert.deepEqual(footerFlash, { message: "usage: @ <arroba-shell command>", tone: "error" })
})

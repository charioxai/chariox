import assert from "node:assert/strict"
import test from "node:test"

import { createCliAutomationActionHandler } from "./cli-automation-handler.js"
import type { CliOptions, RuntimeSession } from "./cli-types.js"
import type { WorkspaceScreenMode } from "./workspace-screen.js"

test("automation action handler switches attached workspace screens through app deps", async () => {
  let screen: WorkspaceScreenMode = "agents"
  let rebuilt = false
  let layoutApplied = false
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    snapshot: () => ({ screen }),
    isAttached: () => true,
    setWorkspaceScreenMode: (next) => {
      screen = next
    },
    rebuildTranscript: () => {
      rebuilt = true
    },
    applyResponseLayout: () => {
      layoutApplied = true
    },
  })

  const result = await handler({ action: "switch_screen", screen: "workflow" })

  assert.deepEqual(result, { screen: "workflow" })
  assert.equal(rebuilt, true)
  assert.equal(layoutApplied, true)
})

test("automation action handler rejects screen switching while detached", async () => {
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    isAttached: () => false,
  })

  await assert.rejects(
    handler({ action: "switch_screen", screen: "workflow" }),
    /cannot switch screen without an attached session/,
  )
})

test("automation action handler waits until snapshot filters match", async () => {
  let attempts = 0
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    snapshot: () => {
      attempts += 1
      return { shell: { entries: Array.from({ length: attempts >= 3 ? 2 : 1 }, (_entry, index) => ({ id: index })) } }
    },
    sleep: async () => {},
  })

  const result = await handler({
    action: "wait_for",
    shellEntryCount: 2,
    intervalMs: 1,
    timeoutMs: 100,
  })

  assert.deepEqual(result, { shell: { entries: [{ id: 0 }, { id: 1 }] } })
  assert.equal(attempts, 3)
})

function baseDeps() {
  return {
    client: null as never,
    options: { provider: "opencode", model: "default", effort: "" } as CliOptions,
    appLogger: null,
    snapshot: () => ({}),
    isAttached: () => false,
    kernelConnected: () => false,
    workflowScreenActive: () => false,
    setWorkspaceScreenMode: (_screen: WorkspaceScreenMode) => {},
    rebuildTranscript: () => {},
    applyResponseLayout: () => {},
    showWorkflowScreen: () => {},
    submitWorkspaceShellCommand: async () => null,
    attachmentState: () => null,
    sessionState: () => ({ id: "session-1" }) as RuntimeSession,
    focusedAgentId: () => null,
    setPromptText: () => {},
    submitPrompt: async () => {},
    activateWaitingRoom: async () => {},
    connectDetachedKernelFromWaitingRoom: async () => {},
    submitFocusedInteractionChoice: async () => {},
    cycleFocusedInteractionChoice: () => {},
    restoreTerminalAndExit: async () => {},
  }
}

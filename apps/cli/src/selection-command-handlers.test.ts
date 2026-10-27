import assert from "node:assert/strict"
import test from "node:test"

import { handleModelSlashCommand } from "./selection-command-handlers.js"

test("model slash command delegates to model selection", async () => {
  let selectedModel: string | null = null

  await handleModelSlashCommand({
    isAttached: () => true,
    sessionState: () => ({ id: "session-1", agents: [] }) as never,
    attachmentState: () => null,
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    flashFooter: () => {},
    applyModelSelection: async (value) => {
      selectedModel = value
    },
    applyVariantSelection: async () => {},
    logViewCommand: () => {},
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({ session: {} as never, config: {} as never }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
  }, {
    kind: "model",
    raw: "/model claude-headless/claude-opus-4-7",
    value: "claude-headless/claude-opus-4-7",
  })

  assert.equal(selectedModel, "claude-headless/claude-opus-4-7")
})

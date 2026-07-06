import { strict as assert } from "node:assert"
import test from "node:test"

import type { RuntimeSession, WorkspaceLiveSyncStatus } from "./cli-types.js"
import type { PromptMetaPart } from "@arroba/kernel-client/prompt-meta"
import {
  createSessionChromeRenderController,
  type SessionChromeRenderControllerDeps,
} from "./session-chrome-render-controller.js"

test("session chrome render controller applies attached chrome in order", () => {
  const calls: string[] = []
  const metaParts = [{ label: "model", value: "gpt" }] as unknown as PromptMetaPart[]
  const controller = createSessionChromeRenderController(createDeps({
    calls,
    attached: true,
    promptMetaParts: metaParts,
    sessionStatusMode: "working",
  }))

  controller.assignPromptStateBox("prompt-box")
  controller.assignFooterSummaryBox("footer-box")
  controller.apply()

  assert.deepEqual(calls, [
    "placeholder",
    "summary:prompt-box:footer-box:ready:muted:Session session-1 • 2 CLIs • 2 agents • Ctrl+C stop • Tab focus • ? keys:none",
    "meta:1",
    "status",
    "footers",
    "interactions",
  ])
})

test("session chrome render controller clears prompt meta while detached", () => {
  const calls: string[] = []
  const controller = createSessionChromeRenderController(createDeps({
    calls,
    attached: false,
    promptMetaParts: [{ label: "provider", value: "opencode" }] as unknown as PromptMetaPart[],
  }))

  controller.apply()

  assert.deepEqual(calls, [
    "placeholder",
    "summary:none:none:ready:muted:Waiting room • arrows • Enter • A archive • D delete • Ctrl+T keys:none",
    "meta:0",
    "status",
    "footers",
    "interactions",
  ])
})

test("session chrome render controller throttles only while activity chrome is volatile", () => {
  assert.equal(createSessionChromeRenderController(createDeps({ working: true })).shouldThrottle(), true)
  assert.equal(createSessionChromeRenderController(createDeps({ activeStatusLabel: "Stopping" })).shouldThrottle(), true)
  assert.equal(createSessionChromeRenderController(createDeps({ providerActivityLabel: "Running tool" })).shouldThrottle(), true)
  assert.equal(createSessionChromeRenderController(createDeps({ streamingAgentId: "agent-1" })).shouldThrottle(), true)
  assert.equal(createSessionChromeRenderController(createDeps()).shouldThrottle(), false)
})

function createDeps(overrides: {
  calls?: string[]
  attached?: boolean
  promptMetaParts?: PromptMetaPart[]
  sessionStatusMode?: "idle" | "working" | "disconnected"
  workspaceLiveSyncStatus?: WorkspaceLiveSyncStatus | null
  working?: boolean
  activeStatusLabel?: string | null
  providerActivityLabel?: string | null
  streamingAgentId?: string | null
  terminalWidth?: number
} = {}): SessionChromeRenderControllerDeps<Record<string, never>, string> {
  const calls = overrides.calls ?? []
  const attached = overrides.attached ?? true
  return {
    renderer: {},
    createSummaryRenderState: () => ({}),
    syncPromptPlaceholder: () => {
      calls.push("placeholder")
    },
    getFatalError: () => null,
    getSubmitting: () => false,
    getFooterHint: () => "ready",
    isAttached: () => attached,
    getSession: session,
    getConnectedClientCount: () => 2,
    getMultiAgentMode: () => true,
    getResponseLayout: () => "split",
    getSessionStatusMode: () => overrides.sessionStatusMode ?? "idle",
    getWorkspaceLiveSyncStatus: () => overrides.workspaceLiveSyncStatus ?? null,
    getHotkeyToggleLabel: () => "?",
    getTerminalWidth: () => overrides.terminalWidth ?? 80,
    getFooterFlash: () => null,
    getPromptMetaParts: () => overrides.promptMetaParts ?? [],
    setPromptMetaRenderables: (parts) => {
      calls.push(`meta:${parts.length}`)
    },
    renderStatusIndicator: () => {
      calls.push("status")
    },
    renderSplitPaneFooters: () => {
      calls.push("footers")
    },
    renderAgentInteractions: () => {
      calls.push("interactions")
    },
    getWorking: () => overrides.working ?? false,
    getActiveStatusLabel: () => overrides.activeStatusLabel ?? null,
    getProviderActivityLabel: () => overrides.providerActivityLabel ?? null,
    getStreamingAgentId: () => overrides.streamingAgentId ?? null,
    renderSummary: (options) => {
      calls.push([
        "summary",
        options.promptStateBox ?? "none",
        options.footerSummaryBox ?? "none",
        options.promptStateLabel,
        options.promptStateTone,
        options.footerSummary,
        options.footerFlash?.message ?? "none",
      ].join(":"))
    },
  }
}

function session(): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    agents: [{ id: "agent-1" }, { id: "agent-2" }],
  } as unknown as RuntimeSession
}

import { strict as assert } from "node:assert"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import type { PromptMetaPart } from "./prompt-meta.js"
import {
  createSessionChromeRenderController,
  type SessionChromeRenderControllerDeps,
} from "./session-chrome-render-controller.js"
import { SESSION_NEW_FOOTER_HINT } from "./sessions.js"

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
    "summary:prompt-box:footer-box:ready:muted:Session session-1 • 2 CLIs connected • 2 agents in session • Ctrl+C to stop • Tab cycles focus • Ctrl+P opens workflow • ? hotkeys:none",
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
    `summary:none:none:ready:muted:${SESSION_NEW_FOOTER_HINT}:none`,
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
  working?: boolean
  activeStatusLabel?: string | null
  providerActivityLabel?: string | null
  streamingAgentId?: string | null
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
    getFocusedHasPromptWork: () => false,
    getHotkeyToggleLabel: () => "?",
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

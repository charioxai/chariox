import assert from "node:assert/strict"
import test from "node:test"

import type { ProviderAuthStatus, RuntimeProviderRun, RuntimeSession } from "./cli-types.js"
import { createProviderSelectionController } from "./provider-selection-controller.js"
import { fallbackProviderCatalog, type BackendProviderId } from "./provider-catalog.js"
import type { ArrobaPreferences } from "./preferences.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createWaitingRoomState, type WaitingRoomState } from "./waiting-room.js"

test("provider selection controller applies detached model selection to waiting room state", async () => {
  const harness = createHarness({ attached: false })

  await harness.controller.applyModelSelection("opencode/gpt-5.4")

  assert.equal(harness.reconciledStates().at(-1)?.modelId, "opencode/gpt-5.4")
  assert.equal(harness.profileUpdates().length, 0)
  assert.deepEqual(harness.footerMessages().at(-1), {
    message: "selected model opencode/gpt-5.4",
    tone: "info",
  })
})

test("provider selection controller updates attached agent variants", async () => {
  const harness = createHarness({
    attached: true,
    currentSelection: { provider: "opencode", model: "opencode/gpt-5.4", effort: "medium" },
  })

  await harness.controller.applyVariantSelection("high")

  assert.deepEqual(harness.profileUpdates().at(-1)?.profile, {
    provider: "opencode",
    model: "opencode/gpt-5.4",
    effort: "high",
  })
  assert.equal(harness.providerRunCleared(), true)
  assert.deepEqual(harness.footerMessages().at(-1), {
    message: "variant set to high",
    tone: "info",
  })
})

test("provider selection controller blocks model changes controlled by native TUI", async () => {
  const harness = createHarness({
    attached: true,
    providerRun: {
      agent_instance_id: "agent-1",
      client_interface: "native_tui",
    } as RuntimeProviderRun,
  })

  await harness.controller.applyModelSelection("opencode/gpt-5.4")

  assert.equal(harness.profileUpdates().length, 0)
  assert.deepEqual(harness.footerMessages().at(-1), {
    message: "model is controlled by the provider-native TUI for this agent",
    tone: "error",
  })
})

test("provider selection controller applies saved provider defaults and auth notices", async () => {
  const harness = createHarness({
    attached: true,
    preferences: {
      providers: {
        codex: {
          model: "codex/gpt-5.4",
          effort: "high",
        },
      },
    },
    authStatus: {
      provider: "codex",
      auth_state: "missing",
      account_profile: null,
      login_hint: "Run codex login.",
      detected_version: null,
    },
  })

  await harness.controller.applyProviderSelection("codex")

  assert.deepEqual(harness.defaults(), {
    provider: "codex",
    model: "codex/gpt-5.4",
    effort: "high",
  })
  assert.deepEqual(harness.profileUpdates().at(-1)?.profile, {
    provider: "codex",
    model: "codex/gpt-5.4",
    effort: "high",
  })
  assert.equal(harness.notices().at(-1), "Codex is not logged in. Run codex login.")
  assert.deepEqual(harness.footerMessages().at(-1), {
    message: "Codex selected",
    tone: "info",
  })
})

test("provider selection controller reports provider update failures", async () => {
  const harness = createHarness({
    attached: true,
    updateAgentProfile: async () => {
      throw new Error("profile down")
    },
  })

  await harness.controller.applyProviderSelection("codex")

  assert.deepEqual(harness.footerMessages().at(-1), {
    message: "profile down",
    tone: "error",
  })
  assert.equal(harness.warnings().at(-1)?.message, "provider profile update failed")
})

function createHarness(options: {
  attached?: boolean
  currentSelection?: { provider: string; model: string; effort: string }
  providerRun?: RuntimeProviderRun | null
  preferences?: ArrobaPreferences
  authStatus?: ProviderAuthStatus
  updateAgentProfile?: (
    sessionId: string,
    agentId: string,
    profile: { provider: BackendProviderId; model: string; effort: string },
  ) => Promise<{ session: RuntimeSession }>
} = {}) {
  const catalog = fallbackProviderCatalog()
  let waitingRoomState = createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "medium", "opencode", DEFAULT_THEME_REGISTRY)
  let defaults = { provider: "opencode" as BackendProviderId, model: "opencode/gpt-5.4", effort: "medium" }
  let providerRun = options.providerRun ?? null
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const reconciledStates: WaitingRoomState[] = []
  const profileUpdates: Array<{
    sessionId: string
    agentId: string
    profile: { provider: BackendProviderId; model: string; effort: string }
  }> = []
  const notices: string[] = []
  const warnings: Array<{ message: string; fields: Record<string, unknown> }> = []
  let providerRunCleared = false

  const controller = createProviderSelectionController({
    currentProviderSelection: () => options.currentSelection ?? defaults,
    waitingRoomState: () => waitingRoomState,
    availableSessions: () => [],
    providerCatalog: () => catalog,
    themeRegistry: () => DEFAULT_THEME_REGISTRY,
    preferences: () => options.preferences ?? {},
    defaults: () => defaults,
    setDefaults: (selection) => {
      defaults = selection
    },
    reconcileWaitingRoom: (next) => {
      waitingRoomState = next
      reconciledStates.push(next)
    },
    isAttached: () => options.attached ?? false,
    focusedAgentId: () => "agent-1",
    providerRunState: () => providerRun,
    sessionState: () => ({ id: "session-1" }) as RuntimeSession,
    updateAgentProfile: async (sessionId, agentId, profile) => {
      profileUpdates.push({ sessionId, agentId, profile })
      if (options.updateAgentProfile) {
        return options.updateAgentProfile(sessionId, agentId, profile)
      }
      return { session: { id: "updated-session" } as RuntimeSession }
    },
    applySessionState: () => {},
    clearProviderRunState: () => {
      providerRun = null
      providerRunCleared = true
    },
    getProviderAuthStatus: async (provider) => options.authStatus ?? ({
      provider,
      auth_state: "authenticated",
      account_profile: null,
      login_hint: null,
      detected_version: null,
    }),
    appendNotice: (text) => {
      notices.push(text)
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    warn: (message, fields) => {
      warnings.push({ message, fields })
    },
  })

  return {
    controller,
    defaults: () => defaults,
    footerMessages: () => footerMessages,
    reconciledStates: () => reconciledStates,
    profileUpdates: () => profileUpdates,
    providerRunCleared: () => providerRunCleared,
    notices: () => notices,
    warnings: () => warnings,
  }
}

import assert from "node:assert/strict"
import test from "node:test"

import type { ProviderAccountProfile, ProviderAuthStatus, RuntimeProviderRun, RuntimeSession } from "./cli-types.js"
import { createProviderSelectionController } from "./provider-selection-controller.js"
import { fallbackProviderCatalog, type BackendProviderId } from "./provider-catalog.js"
import type { CharioxPreferences } from "./preferences.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

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

test("provider selection controller marks detached selections from local provider fallback", async () => {
  const harness = createHarness({
    attached: false,
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
  })

  await harness.controller.applyModelSelection("opencode/gpt-5.4")
  await harness.controller.applyVariantSelection("high")
  await harness.controller.applyProviderSelection("codex")

  assert.deepEqual(harness.footerMessages().map((message) => message.message), [
    "selected model opencode/gpt-5.4 (local provider list)",
    "selected variant high (local provider list)",
    "Codex selected (local provider list)",
  ])
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

test("provider selection controller applies detached mode and permissions to launch defaults", async () => {
  const harness = createHarness({ attached: false })

  await harness.controller.applyModeSelection("plan")
  await harness.controller.applyPermissionSelection("required")

  assert.equal(harness.reconciledStates().at(-1)?.executionMode, "plan")
  assert.equal(harness.reconciledStates().at(-1)?.permissionLevel, "required")
  assert.deepEqual(harness.configUpdates(), [])
  assert.deepEqual(harness.footerMessages().slice(-2), [
    { message: "mode default set to plan", tone: "info" },
    { message: "permissions default set to required", tone: "info" },
  ])
})

test("provider selection controller routes attached mode and permissions through agent config", async () => {
  const harness = createHarness({ attached: true })

  await harness.controller.applyModeSelection("plan")
  await harness.controller.applyPermissionSelection("required")

  assert.deepEqual(harness.configUpdates(), [
    { sessionId: "session-1", agentId: "agent-1", config: { executionMode: "plan" } },
    { sessionId: "session-1", agentId: "agent-1", config: { permissionLevel: "required" } },
  ])
})

test("provider selection controller updates attached agent models through profile API", async () => {
  const harness = createHarness({
    attached: true,
    currentSelection: { provider: "opencode", model: "opencode/gpt-5.3", effort: "medium" },
  })

  await harness.controller.applyModelSelection("opencode/gpt-5.4")

  assert.deepEqual(harness.profileUpdates().at(-1), {
    sessionId: "session-1",
    agentId: "agent-1",
    profile: {
      provider: "opencode",
      model: "opencode/gpt-5.4",
      effort: "medium",
    },
  })
  assert.equal(harness.providerRunCleared(), true)
})

test("provider selection controller loads the chosen account catalog before one atomic agent update", async () => {
  const secondaryCatalog = {
    all: [{
      id: "codex",
      name: "Codex",
      models: {
        "gpt-5.6-luna": {
          id: "gpt-5.6-luna",
          name: "Luna",
          status: "active",
          variants: { low: {} },
        },
      },
    }],
    default: { codex: "gpt-5.6-luna" },
    connected: ["codex"],
  }
  const harness = createHarness({
    attached: true,
    currentSelection: { provider: "codex", accountProfile: "default", model: "codex/gpt-5.4", effort: "high" },
    providerAccounts: [{
      provider: "codex",
      profile_id: "secondary",
      label: "Validation",
      auth_state: "authenticated",
      is_default: false,
    } as ProviderAccountProfile],
    scopedCatalog: secondaryCatalog,
  })

  await harness.controller.applyAccountSelection("Validation")

  assert.deepEqual(harness.catalogLoads(), [{ provider: "codex", accountProfile: "secondary" }])
  assert.deepEqual(harness.profileUpdates().at(-1)?.profile, {
    provider: "codex",
    accountProfile: "secondary",
    model: "codex/gpt-5.6-luna",
    effort: "low",
  })
  assert.deepEqual(harness.footerMessages().at(-1), { message: "account set to Validation", tone: "info" })
})

test("provider selection controller marks attached profile updates from local provider fallback", async () => {
  const harness = createHarness({
    attached: true,
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
    currentSelection: { provider: "opencode", model: "opencode/gpt-5.4", effort: "medium" },
  })

  await harness.controller.applyModelSelection("opencode/gpt-5.4")
  await harness.controller.applyVariantSelection("high")

  assert.deepEqual(harness.footerMessages().map((message) => message.message), [
    "model set to opencode/gpt-5.4 (local provider list)",
    "variant set to high (local provider list)",
  ])
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
    accountProfile: "default",
    model: "codex/gpt-5.4",
    effort: "high",
  })
  assert.deepEqual(harness.profileUpdates().at(-1)?.profile, {
    provider: "codex",
    accountProfile: "default",
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
  currentSelection?: { provider: string; accountProfile?: string; model: string; effort: string }
  providerRun?: RuntimeProviderRun | null
  preferences?: CharioxPreferences
  authStatus?: ProviderAuthStatus
  providerCatalog?: ReturnType<typeof fallbackProviderCatalog>
  scopedCatalog?: ReturnType<typeof fallbackProviderCatalog> | { all: any[]; default: Record<string, string>; connected: string[] }
  providerAccounts?: ProviderAccountProfile[]
  updateAgentProfile?: (
    sessionId: string,
    agentId: string,
    profile: { provider: BackendProviderId; accountProfile?: string; model: string; effort: string },
  ) => Promise<{ session: RuntimeSession }>
} = {}) {
  const catalog = options.providerCatalog ?? fallbackProviderCatalog()
  let waitingRoomState = createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "medium", "opencode", DEFAULT_THEME_REGISTRY)
  let defaults: { provider: BackendProviderId; accountProfile?: string; model: string; effort: string } = {
    provider: "opencode",
    accountProfile: "default",
    model: "opencode/gpt-5.4",
    effort: "medium",
  }
  let providerRun = options.providerRun ?? null
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const reconciledStates: WaitingRoomState[] = []
  const profileUpdates: Array<{
    sessionId: string
    agentId: string
    profile: { provider: BackendProviderId; accountProfile?: string; model: string; effort: string }
  }> = []
  const catalogLoads: Array<{ provider: BackendProviderId; accountProfile: string }> = []
  const notices: string[] = []
  const warnings: Array<{ message: string; fields: Record<string, unknown> }> = []
  const configUpdates: Array<{
    sessionId: string
    agentId: string
    config: { executionMode?: "build" | "plan"; permissionLevel?: "required" | "yolo" }
  }> = []
  let providerRunCleared = false

  const controller = createProviderSelectionController({
    currentProviderSelection: () => options.currentSelection ?? defaults,
    waitingRoomState: () => waitingRoomState,
    availableSessions: () => [],
    providerCatalog: () => catalog,
    providerAccounts: () => options.providerAccounts ?? [],
    loadProviderCatalog: async (provider, accountProfile) => {
      catalogLoads.push({ provider, accountProfile })
      return options.scopedCatalog ?? catalog
    },
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
    updateAgentConfig: async (sessionId, agentId, config) => {
      configUpdates.push({ sessionId, agentId, config })
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
    configUpdates: () => configUpdates,
    catalogLoads: () => catalogLoads,
    providerRunCleared: () => providerRunCleared,
    notices: () => notices,
    warnings: () => warnings,
  }
}

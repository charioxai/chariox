import assert from "node:assert/strict"
import test from "node:test"

import { handleProviderSlashCommand, type ProviderCommandHandlerDeps } from "./provider-command-handlers.js"

test("provider account list explains unavailable usage and exhausted limits", async () => {
  const notices: string[] = []
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: () => {},
    appendNotice: (message) => notices.push(message),
    listProviderAccountProfiles: async () => [{
      owner_user_id: "owner-a",
      provider: "codex",
      profile_id: "secondary",
      label: "Validation",
      origin: "chariox_created",
      is_default: false,
      auth_state: "authenticated",
      usage: {
        profile_id: "secondary",
        provider: "codex",
        availability: "available",
        meters: [{
          meter_id: "primary",
          label: "Primary",
          kind: "rolling_limit",
          scope: "account",
          used_percent: 100,
          resets_at_ms: Date.now() + 3_600_000,
          state: "exhausted",
          source: "codex.app_server",
          observed_at_ms: Date.now(),
        }],
        source: "codex.app_server",
      },
      materializations: [],
    }, {
      owner_user_id: "owner-a",
      provider: "opencode",
      profile_id: "default",
      label: "opencode-1",
      origin: "default",
      is_default: true,
      auth_state: "authenticated",
      usage: {
        profile_id: "default",
        provider: "opencode",
        availability: "unavailable",
        meters: [],
        source: "opencode.local_stats",
      },
      materializations: [],
    }],
  }

  await handleProviderSlashCommand(deps, { kind: "provider", raw: "/provider accounts list", value: "accounts list" })

  assert.match(notices.join("\n"), /100% used · exhausted · resets in 1h/)
  assert.match(notices.join("\n"), /not reported \(OpenCode exposes local stats, not upstream balance\)/)
  assert.doesNotMatch(notices.join("\n"), /secondary|owner@example.com/)
})

test("provider account actions accept aliases while keeping profile ids internal", async () => {
  const calls: string[] = []
  const notices: string[] = []
  const profile: Awaited<ReturnType<NonNullable<ProviderCommandHandlerDeps["listProviderAccountProfiles"]>>>[number] = {
    owner_user_id: "owner-a",
    provider: "codex",
    profile_id: "opaque-profile-id",
    label: "codex-2",
    origin: "chariox_created",
    is_default: false,
    auth_state: "authenticated",
    identity_summary: "owner@example.com",
    usage: {
      profile_id: "opaque-profile-id",
      provider: "codex",
      availability: "unavailable",
      meters: [],
      source: "provider_api_unavailable",
    },
    materializations: [],
  }
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: () => {},
    appendNotice: (message) => notices.push(message),
    listProviderAccountProfiles: async () => [profile],
    setDefaultProviderAccountProfile: async (provider, profileId) => {
      calls.push(`${provider}:${profileId}`)
      return { ...profile, is_default: true }
    },
  }

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider accounts default codex codex-2",
    value: "accounts default codex codex-2",
  })

  assert.deepEqual(calls, ["codex:opaque-profile-id"])
  assert.deepEqual(notices, ["default: codex codex-2"])
})

test("provider account creation permits automatic aliases", async () => {
  const labels: string[] = []
  const profile = {
    owner_user_id: "owner-a",
    provider: "codex",
    profile_id: "opaque-profile-id",
    label: "codex-2",
    origin: "chariox_created" as const,
    is_default: false,
    auth_state: "not_configured" as const,
    usage: {
      profile_id: "opaque-profile-id",
      provider: "codex",
      availability: "unavailable" as const,
      meters: [],
      source: "provider_not_observed",
    },
    materializations: [],
  }
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: () => {},
    appendNotice: () => {},
    createProviderAccountProfile: async (_provider, label) => {
      labels.push(label)
      return profile
    },
  }

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider accounts add codex",
    value: "accounts add codex",
  })

  assert.deepEqual(labels, [""])
})

test("provider login targets the explicitly selected account profile", async () => {
  const starts: Array<{ provider: string; accountProfile?: string }> = []
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: () => {},
    appendNotice: () => {},
    startProviderLogin: async (provider, accountProfile) => {
      starts.push({ provider, accountProfile })
      return { provider, account_profile: accountProfile ?? "default", login_kind: "chatgptDeviceCode" }
    },
  }

  await handleProviderSlashCommand(deps, { kind: "provider", raw: "/provider login codex secondary", value: "login codex secondary" })

  assert.deepEqual(starts, [{ provider: "codex", accountProfile: "secondary" }])
})

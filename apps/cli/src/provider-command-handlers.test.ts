import assert from "node:assert/strict"
import test from "node:test"

import { handleProviderSlashCommand, resolveProviderAccountAlias, type ProviderCommandHandlerDeps } from "./provider-command-handlers.js"

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
    listProviderAccountProfiles: async () => [{
      owner_user_id: "user-1",
      provider: "codex",
      profile_id: "opaque-secondary-id",
      label: "secondary",
      origin: "chariox_created",
      is_default: false,
      auth_state: "authenticated",
      usage: {
        profile_id: "opaque-secondary-id",
        provider: "codex",
        availability: "unavailable",
        source: "test",
      },
    }],
    startProviderLogin: async (provider, accountProfile) => {
      starts.push(accountProfile === undefined ? { provider } : { provider, accountProfile })
      return {
        provider,
        account_profile: accountProfile ?? "default",
        login_kind: "chatgptDeviceCode",
        login_id: null,
        auth_url: null,
        verification_url: null,
        user_code: null,
      }
    },
  }

  await handleProviderSlashCommand(deps, { kind: "provider", raw: "/provider login codex secondary", value: "login codex secondary" })

  assert.deepEqual(starts, [{ provider: "codex", accountProfile: "opaque-secondary-id" }])
})

test("provider status, logout, and reauth reject an unknown account alias before provider work", async () => {
  const footers: Array<{ message: string; tone: "info" | "error" }> = []
  const providerCalls: string[] = []
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: (message, tone) => footers.push({ message, tone }),
    appendNotice: () => {},
    listProviderAccountProfiles: async () => [],
    getProviderAuthStatus: async () => {
      providerCalls.push("status")
      throw new Error("status should not run")
    },
    logoutProvider: async () => {
      providerCalls.push("logout")
      throw new Error("logout should not run")
    },
    startProviderLogin: async () => {
      providerCalls.push("login")
      throw new Error("login should not run")
    },
  }

  for (const action of ["status", "logout", "reauth"]) {
    await handleProviderSlashCommand(deps, {
      kind: "provider",
      raw: `/provider ${action} codex missing-account`,
      value: `${action} codex missing-account`,
    })
  }

  assert.deepEqual(providerCalls, [])
  assert.deepEqual(footers, Array.from({ length: 3 }, () => ({
    message: "provider account alias missing-account was not found for codex",
    tone: "error" as const,
  })))
})

test("provider account alias resolution trims whitespace and ignores case without exposing ids", async () => {
  const footers: string[] = []
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: (message) => footers.push(message),
    appendNotice: () => {},
    listProviderAccountProfiles: async () => [{
      owner_user_id: "owner-a",
      provider: "codex",
      profile_id: "opaque-stable-id",
      label: "Validation",
      origin: "chariox_created",
      is_default: false,
      auth_state: "authenticated",
      usage: { profile_id: "opaque-stable-id", provider: "codex", availability: "unavailable", source: "test" },
    }],
  }

  assert.equal(await resolveProviderAccountAlias(deps, "codex", "  Validation  "), "opaque-stable-id")
  assert.equal(await resolveProviderAccountAlias(deps, "codex", "\tvalidation\n"), "opaque-stable-id")
  assert.deepEqual(footers, [])
})

test("provider account actions reject unknown aliases with public-label errors and no mutation", async () => {
  const footers: Array<{ message: string; tone: "info" | "error" }> = []
  const mutations: string[] = []
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: (message, tone) => footers.push({ message, tone }),
    appendNotice: () => {},
    listProviderAccountProfiles: async () => [],
    setDefaultProviderAccountProfile: async () => {
      mutations.push("default")
      throw new Error("default switch should not run")
    },
    refreshProviderAccountProfile: async () => {
      mutations.push("refresh")
      throw new Error("refresh should not run")
    },
    removeProviderAccountProfile: async () => {
      mutations.push("remove")
      throw new Error("remove should not run")
    },
  }

  for (const action of ["refresh", "default", "remove"]) {
    await handleProviderSlashCommand(deps, {
      kind: "provider",
      raw: `/provider accounts ${action} codex missing`,
      value: `accounts ${action} codex missing`,
    })
  }

  assert.deepEqual(mutations, [])
  assert.deepEqual(footers, Array.from({ length: 3 }, () => ({
    message: "provider account alias missing was not found for codex",
    tone: "error" as const,
  })))
})

test("provider status resolves a padded alias to the stable id and reports only public labels", async () => {
  const statuses: Array<{ provider: string; accountProfile?: string | undefined }> = []
  const notices: string[] = []
  const footers: string[] = []
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: (message) => footers.push(message),
    appendNotice: (message) => notices.push(message),
    listProviderAccountProfiles: async () => [{
      owner_user_id: "owner-a",
      provider: "codex",
      profile_id: "opaque-stable-id",
      label: "Validation",
      origin: "chariox_created",
      is_default: false,
      auth_state: "authenticated",
      identity_summary: "owner@example.com",
      usage: { profile_id: "opaque-stable-id", provider: "codex", availability: "unavailable", source: "test" },
    }],
    getProviderAuthStatus: async (provider, accountProfile) => {
      statuses.push({ provider, ...(accountProfile === undefined ? {} : { accountProfile }) })
      return {
        provider,
        account_profile: accountProfile ?? "default",
        auth_state: "authenticated",
        identity_summary: "owner@example.com",
        login_hint: null,
        detected_version: null,
      }
    },
  }

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider status codex VALIDATION",
    value: "status codex VALIDATION",
  })

  assert.deepEqual(statuses, [{ provider: "codex", accountProfile: "opaque-stable-id" }])
  const rendered = [...notices, ...footers].join("\n")
  assert.match(rendered, /codex\/Validation/)
  assert.doesNotMatch(rendered, /opaque-stable-id/)
})

test("provider login and reauth pass a --method enrollment selection to the kernel", async () => {
  const starts: Array<{ provider: string; accountProfile?: string; method?: string }> = []
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: () => {},
    appendNotice: () => {},
    listProviderAccountProfiles: async () => [{
      owner_user_id: "user-1",
      provider: "codex",
      profile_id: "opaque-secondary-id",
      label: "secondary",
      origin: "chariox_created",
      is_default: false,
      auth_state: "authenticated",
      usage: {
        profile_id: "opaque-secondary-id",
        provider: "codex",
        availability: "unavailable",
        source: "test",
      },
    }],
    logoutProvider: async () => ({
      kind: "logged_out",
      result: { provider: "codex", account_profile: "opaque-secondary-id" },
    }),
    startProviderLogin: async (provider, accountProfile, method) => {
      starts.push({
        provider,
        ...(accountProfile === undefined ? {} : { accountProfile }),
        ...(method === undefined ? {} : { method }),
      })
      return {
        provider,
        account_profile: accountProfile ?? "default",
        login_kind: method ? `${method}:started` : "chatgptDeviceCode",
        login_id: null,
        auth_url: null,
        verification_url: null,
        user_code: null,
      }
    },
  }

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider login codex secondary --method device_code",
    value: "login codex secondary --method device_code",
  })
  assert.deepEqual(starts, [{
    provider: "codex",
    accountProfile: "opaque-secondary-id",
    method: "device_code",
  }])

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider reauth codex secondary --method terminal",
    value: "reauth codex secondary --method terminal",
  })
  assert.deepEqual(starts[1], {
    provider: "codex",
    accountProfile: "opaque-secondary-id",
    method: "terminal",
  })

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider login codex secondary",
    value: "login codex secondary",
  })
  assert.equal(starts[2]?.method, undefined)
})

test("provider login rejects --method without a value", async () => {
  const footers: Array<{ message: string; tone: string }> = []
  let started = false
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: (message, tone) => footers.push({ message, tone }),
    appendNotice: () => {},
    startProviderLogin: async () => {
      started = true
      throw new Error("login must not start")
    },
  }

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider login codex secondary --method",
    value: "login codex secondary --method",
  })

  assert.equal(started, false)
  assert.deepEqual(footers, [{
    message: "--method requires an enrollment method",
    tone: "error",
  }])
})

test("account listing reports credential kind without exposing ids", async () => {
  const notices: string[] = []
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: () => {},
    appendNotice: (message) => notices.push(message),
    listProviderAccountProfiles: async () => [
      {
        owner_user_id: "owner-a",
        provider: "codex",
        profile_id: "opaque-managed",
        label: "Managed",
        origin: "chariox_created",
        is_default: true,
        auth_state: "authenticated",
        credential_kind: "subscription",
        usage: {
          profile_id: "opaque-managed",
          provider: "codex",
          availability: "unavailable",
          meters: [],
          source: "test",
        },
      },
      {
        owner_user_id: "owner-a",
        provider: "claude",
        profile_id: "opaque-claude",
        label: "Claude Work",
        origin: "chariox_created",
        is_default: false,
        auth_state: "not_configured",
        credential_kind: null,
        credential_kind_not_reported_reason:
          "the provider-native login does not report the resulting credential type",
        usage: {
          profile_id: "opaque-claude",
          provider: "claude",
          availability: "unavailable",
          meters: [],
          source: "test",
        },
      },
    ],
  }

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider accounts list",
    value: "accounts list",
  })

  const rendered = notices.join("\n")
  assert.match(rendered, /codex Managed \[default\] · subscription/)
  assert.match(rendered, /kind not reported \(the provider-native login does not report the resulting credential type\)/)
  assert.doesNotMatch(rendered, /opaque-managed|opaque-claude/)
})

test("accounts add with --method enrolls the new alias through the chosen mode", async () => {
  const enrollments: Array<{ provider: string; accountProfile?: string; method?: string }> = []
  const footers: string[] = []
  const created = {
    owner_user_id: "owner-a",
    provider: "codex",
    profile_id: "opaque-new-profile",
    label: "Fresh",
    origin: "chariox_created" as const,
    is_default: false,
    auth_state: "not_configured" as const,
    credential_kind: "subscription" as const,
    usage: {
      profile_id: "opaque-new-profile",
      provider: "codex",
      availability: "unavailable" as const,
      meters: [],
      source: "test",
    },
  }
  const deps: ProviderCommandHandlerDeps = {
    currentProviderId: () => "codex",
    flashFooter: (message) => footers.push(message),
    appendNotice: () => {},
    createProviderAccountProfile: async () => created,
    startProviderLogin: async (provider, accountProfile, method) => {
      enrollments.push({
        provider,
        ...(accountProfile === undefined ? {} : { accountProfile }),
        ...(method === undefined ? {} : { method }),
      })
      return {
        provider,
        account_profile: accountProfile ?? "default",
        login_kind: method ? `${method}:started` : "chatgptDeviceCode",
        login_id: null,
        auth_url: null,
        verification_url: null,
        user_code: null,
      }
    },
  }

  await handleProviderSlashCommand(deps, {
    kind: "provider",
    raw: "/provider accounts add codex Fresh --method device_code",
    value: "accounts add codex Fresh --method device_code",
  })

  assert.deepEqual(enrollments, [{
    provider: "codex",
    accountProfile: "opaque-new-profile",
    method: "device_code",
  }])
  assert.match(footers.join(" "), /enrollment started/)
})

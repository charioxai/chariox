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
      label: "Default",
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

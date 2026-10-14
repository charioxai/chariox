import assert from "node:assert/strict"
import test from "node:test"

import type { ProviderAccountProfile } from "@chariox/kernel-client"
import {
  accountRowOpensProviderAccounts,
  providerAccountAddChoices,
  providerAccountsPopupFamilies,
  providerAccountsPopupItems,
} from "./provider-accounts-popup.js"

function profile(provider: string, profileId: string, authState: ProviderAccountProfile["auth_state"], extra: Partial<ProviderAccountProfile> = {}): ProviderAccountProfile {
  return {
    owner_user_id: "local",
    provider,
    profile_id: profileId,
    label: profileId,
    origin: "default",
    is_default: true,
    auth_state: authState,
    usage: { profile_id: profileId, provider, availability: "unavailable", source: "test" },
    ...extra,
  } as ProviderAccountProfile
}

test("MP-08 popup lists only installed providers, the selected provider first, with an add row each", () => {
  const items = providerAccountsPopupItems({
    profiles: [
      profile("codex", "codex-1", "not_configured"),
      profile("claude", "claude-1", "authenticated"),
      profile("opencode", "opencode-1", "unknown"),
    ],
    installedProviders: ["claude", "codex"],
    providerId: "codex",
  })
  assert.deepEqual(items.map((item) => item.label), [
    "Codex · codex-1 (default) · needs sign-in",
    "Add new Codex account",
    "Claude · claude-1 (default) · signed in",
    "Add new Claude account",
    "Link existing provider directory",
  ])
  const codex = items[0]!
  assert.deepEqual(codex.choices.map((choice) => choice.label), ["Sign in", "Use for new session", "Refresh", "Rename", "Remove"])
  assert.deepEqual(codex.choices[0]!.command, { kind: "run", command: "/provider login codex codex-1" })
  assert.deepEqual(items[1]!.choices.map((choice) => choice.command), [{ kind: "run", command: "/provider accounts add codex --method device_code" }])
  const claude = items[2]!
  assert.deepEqual(claude.choices.map((choice) => choice.label), ["Use for new session", "Refresh", "Create setup token", "Rename", "Log out", "Remove"])
  assert.deepEqual(claude.choices[0]!.command, { kind: "use", provider: "claude", profileId: "claude-1" })
  assert.deepEqual(items[4]!.choices.map((choice) => choice.command), [
    { kind: "stage", command: "/provider accounts link codex " },
    { kind: "stage", command: "/provider accounts link claude " },
  ])
})

test("MP-08 popup add choices follow each provider's sign-in methods", () => {
  assert.deepEqual(providerAccountAddChoices("claude-p").map((choice) => choice.command), [
    { kind: "run", command: "/provider accounts add claude --method terminal" },
  ])
  assert.deepEqual(providerAccountAddChoices("opencode").map((choice) => choice.label), ["OpenCode Go", "OpenCode Zen", "Other OpenCode provider"])
})

test("MP-08 popup keeps every provider for kernels that do not report installed harnesses", () => {
  assert.deepEqual(providerAccountsPopupFamilies(undefined), ["claude", "codex", "opencode"])
  assert.deepEqual(providerAccountsPopupFamilies([]), [])
  assert.deepEqual(providerAccountsPopupItems({ profiles: [], installedProviders: [], providerId: "codex" }), [])
})

test("MP-08 Account row opens the popup only when the provider has no signed-in account", () => {
  assert.equal(accountRowOpensProviderAccounts(undefined, "codex"), false, "unknown inventory keeps the row's normal action")
  assert.equal(accountRowOpensProviderAccounts([], "codex"), true)
  assert.equal(accountRowOpensProviderAccounts([profile("codex", "codex-1", "expired")], "codex"), true)
  assert.equal(accountRowOpensProviderAccounts([profile("claude", "claude-1", "authenticated")], "claude-headless"), false)
})

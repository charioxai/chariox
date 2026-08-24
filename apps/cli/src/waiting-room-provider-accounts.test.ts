import assert from "node:assert/strict"
import test from "node:test"

import type { ProviderAccountProfile } from "@chariox/kernel-client"
import {
  defaultProviderAccountProfileId,
  providerAccountFamily,
  providerAccountDisplayLabel,
  providerAccountsForProvider,
  selectedProviderAccount,
} from "./waiting-room-provider-accounts.js"

const profiles: ProviderAccountProfile[] = [
  account("claude", "claude-primary", true),
  account("claude", "claude-secondary"),
  account("codex", "codex-primary", true),
]

test("Claude adapters share the Claude provider-account family", () => {
  assert.equal(providerAccountFamily("claude-headless"), "claude")
  assert.equal(providerAccountFamily("claude-p"), "claude")
  assert.deepEqual(
    providerAccountsForProvider(profiles, "claude-headless").map((profile) => profile.profile_id),
    ["claude-primary", "claude-secondary"],
  )
})

test("provider account selection stays isolated to the selected provider family", () => {
  assert.equal(
    selectedProviderAccount(profiles, "claude-p", "claude-secondary")?.profile_id,
    "claude-secondary",
  )
  assert.equal(selectedProviderAccount(profiles, "claude-p", "codex-primary"), null)
  assert.deepEqual(
    providerAccountsForProvider(profiles, "codex").map((profile) => profile.profile_id),
    ["codex-primary"],
  )
  assert.equal(selectedProviderAccount(profiles, "claude-headless", "default")?.profile_id, "claude-primary")
  assert.equal(selectedProviderAccount(profiles, "codex", undefined)?.profile_id, "codex-primary")
})

test("a missing default stays unavailable instead of silently selecting the first account", () => {
  const accounts = [account("codex", "codex-secondary")]
  assert.equal(defaultProviderAccountProfileId(accounts, "codex"), "default")
  assert.equal(selectedProviderAccount(accounts, "codex", "default"), null)
})

test("provider account display uses only the public alias", () => {
  const profile = account("codex", "internal-profile", true)
  profile.label = "codex-1"
  profile.identity_summary = "owner@example.com"

  assert.equal(providerAccountDisplayLabel(profile), "codex-1")
})

function account(
  provider: string,
  profileId: string,
  isDefault = false,
): ProviderAccountProfile {
  return {
    owner_user_id: "local",
    provider,
    profile_id: profileId,
    label: profileId,
    origin: "linked",
    is_default: isDefault,
    auth_state: "authenticated",
    usage: {
      profile_id: profileId,
      provider,
      availability: "unavailable",
      source: "test",
    },
  }
}

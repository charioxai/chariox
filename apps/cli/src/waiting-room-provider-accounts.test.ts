import assert from "node:assert/strict"
import test from "node:test"

import type {
  ProviderAccountProfile,
  ProviderAccountUsageMeter,
} from "@chariox/kernel-client"
import {
  defaultProviderAccountProfileId,
  normalizeProviderAccountProfileId,
  providerAccountFamily,
  providerAccountDisplayLabel,
  providerAccountSignInCommand,
  selectedProviderAccountStartBlockReason,
  providerAccountsForProvider,
  resolveProviderAccountSelection,
  selectedProviderAccount,
} from "./waiting-room-provider-accounts.js"

const profiles: ProviderAccountProfile[] = [
  account("claude", "claude-primary", true),
  account("claude", "claude-secondary"),
  account("codex", "codex-primary", true),
]

test("MP-08/MP-10/MP-11 destination normalization preserves explicit choices and its own default", () => {
  assert.equal(normalizeProviderAccountProfileId(profiles, "claude", "claude-secondary"), "claude-secondary")
  assert.equal(normalizeProviderAccountProfileId(profiles, "claude", "home-only"), "claude-primary")
  const signedOutDefault = profiles.map(profile => ({ ...profile, auth_state: "not_configured" as const }))
  assert.equal(normalizeProviderAccountProfileId(signedOutDefault, "claude", "home-only"), "claude-primary")
  assert.equal(normalizeProviderAccountProfileId([], "codex", "home-only"), "default")
})

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

test("provider account display uses the public alias and sign-in state", () => {
  const profile = account("codex", "internal-profile", true)
  profile.label = "codex-1"
  profile.identity_summary = "owner@example.com"

  assert.equal(providerAccountDisplayLabel(profile), "codex-1 · signed in")
})

test("TUI marks hard exhaustion without hiding Claude and Codex accounts backed by credits", () => {
  const resetsAtMs = Date.now() + 60_000
  const resetLabel = "resets " + new Date(resetsAtMs).toISOString().slice(0, 16).replace("T", " ") + " UTC"
  const allowance = usageMeter("allowance", "rolling_limit", "exhausted", resetsAtMs)
  const credits = usageMeter("credits", "credit_balance", "exhausted")

  assert.equal(
    providerAccountDisplayLabel(accountWithUsage("opencode", [allowance])),
    "Account · signed in",
    "unscoped exhaustion must not mark every OpenCode service exhausted",
  )
  assert.equal(
    providerAccountDisplayLabel(accountWithUsage("opencode", [
      { ...allowance, service_id: "opencode-go" },
    ]), "opencode-go/test-model"),
    `Account · OpenCode Go (exhausted) · ${resetLabel} · signed in`,
  )
  for (const provider of ["claude", "claude-headless", "claude-p", "codex"]) {
    assert.equal(
      providerAccountDisplayLabel(accountWithUsage(provider, [allowance])),
      "Account · signed in",
      `${provider} allowance exhaustion alone must remain selectable in the TUI`,
    )
    assert.equal(
      providerAccountDisplayLabel(accountWithUsage(provider, [allowance, credits])),
      `Account (exhausted) · ${resetLabel} · signed in`,
      `${provider} must be marked exhausted when allowance and credits are exhausted`,
    )
    assert.equal(
      providerAccountDisplayLabel(accountWithUsage(provider, [allowance, credits, {
        ...credits,
        meter_id: "extra-spend",
        kind: "spend_limit",
        state: "healthy",
        observed_at_ms: Date.now() - 86_400_001,
      }])),
      "Account · signed in",
      `${provider} must not discard a stale credit meter to claim total exhaustion`,
    )
  }
})

test("provider account selection accepts public aliases while preserving internal id support", () => {
  const primary = account("codex", "internal-primary", true)
  primary.label = "codex-1"
  const secondary = account("codex", "internal-secondary")
  secondary.label = "Validation"

  assert.equal(resolveProviderAccountSelection([primary, secondary], "codex", "Validation").kind, "resolved")
  assert.equal(resolvedProfileId(resolveProviderAccountSelection([primary, secondary], "codex", " validation ")), "internal-secondary")
  assert.equal(resolvedProfileId(resolveProviderAccountSelection([primary, secondary], "codex", "internal-secondary")), "internal-secondary")
  assert.equal(resolveProviderAccountSelection([primary, secondary], "opencode", "Validation").kind, "missing")
})

test("provider account selection rejects ambiguous case-folded aliases", () => {
  const first = account("codex", "internal-first")
  first.label = "Work"
  const second = account("codex", "internal-second")
  second.label = "work"

  assert.deepEqual(resolveProviderAccountSelection([first, second], "codex", "WORK"), {
    kind: "ambiguous",
    aliases: ["Work", "work"],
  })
})

function resolvedProfileId(selection: ReturnType<typeof resolveProviderAccountSelection>): string | null {
  return selection.kind === "resolved" ? selection.profile.profile_id : null
}

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

function accountWithUsage(
  provider: string,
  meters: ProviderAccountUsageMeter[],
): ProviderAccountProfile {
  const profile = account(provider, `${provider}-account`, true)
  profile.label = "Account"
  profile.usage = {
    profile_id: profile.profile_id,
    provider,
    availability: "available",
    meters,
    observed_at_ms: Date.now(),
    source: "test",
  }
  return profile
}

function usageMeter(
  meterId: string,
  kind: ProviderAccountUsageMeter["kind"],
  state: ProviderAccountUsageMeter["state"],
  resetsAtMs?: number,
): ProviderAccountUsageMeter {
  return {
    meter_id: meterId,
    label: meterId,
    kind,
    scope: "account",
    state,
    source: "test",
    observed_at_ms: Date.now(),
    ...(resetsAtMs === undefined ? {} : { resets_at_ms: resetsAtMs }),
  }
}

test("MP-08 / MP-11 an unvalidated linked account asks for a check instead of another login", () => {
  const profile = account("codex", "linked", true)
  profile.auth_state = "unknown"
  assert.match(providerAccountDisplayLabel(profile), /sign-in not checked$/)
  assert.match(selectedProviderAccountStartBlockReason([profile], "codex", "default") ?? "", /Refresh its status/)
  assert.equal(providerAccountSignInCommand(profile), "/provider accounts refresh codex linked")
})

test("MP-08 / MP-11 home inventory does not decide worker account authentication", () => {
  const local = account("codex", "local", true)
  local.auth_state = "not_configured"
  assert.match(selectedProviderAccountStartBlockReason([local], "codex", "default", { kind: "local" }) ?? "", /needs sign-in/)
  assert.equal(selectedProviderAccountStartBlockReason([local], "codex", "worker-account", { worker_kernel_id: "worker" }), null)
})

// MP-08 / MP-11 known exhausted quota must be visible before work starts.
test("exhausted selected account exposes reset and a usable alternative", () => {
  const reset = Date.now() + 3_600_000
  const exhausted = accountWithUsage("codex", [usageMeter("rolling/300/codex", "rolling_limit", "exhausted", reset), usageMeter("credits", "credit_balance", "exhausted")])
  const good = account("codex", "available")
  assert.match(providerAccountDisplayLabel(exhausted, "codex/gpt-6.1-sol"), /exhausted.*resets.*UTC/)
  assert.match(selectedProviderAccountStartBlockReason([exhausted, good], "codex", exhausted.profile_id, null, "codex/gpt-6.1-sol") ?? "", /exhausted.*resets.*available/)
  assert.equal(selectedProviderAccountStartBlockReason([exhausted], "codex", exhausted.profile_id, null, "codex/gpt-5.3-codex-spark"), null)
})

// MP-08 / MP-11 OpenCode owns Go → Zen fallback: show exhaustion, never refuse before the harness runs.
test("exhausted OpenCode service stays visible but does not block the start", () => {
  const reset = Date.now() + 3_600_000
  const go = { ...usageMeter("go/monthly", "rolling_limit", "exhausted", reset), service_id: "opencode-go" }
  const zen = { ...usageMeter("zen/credits", "credit_balance", "exhausted"), service_id: "opencode" }
  const openCode = accountWithUsage("opencode", [go, zen])
  assert.match(providerAccountDisplayLabel(openCode, "opencode-go/glm-5.3-flash"), /OpenCode Go \(exhausted\).*resets.*UTC/)
  for (const model of ["opencode-go/glm-5.3-flash", "opencode/deepseek-v4-flash"]) {
    assert.equal(selectedProviderAccountStartBlockReason([openCode], "opencode", openCode.profile_id, null, model), null, model)
  }
})

// MP-08 / MP-10 / MP-11: the kernel checks explicit namespaces; missing defaults still need login.
test("explicit Claude references defer to the kernel while missing defaults need sign-in", () => {
  for (const mode of ["claude", "claude-p", "claude-headless"]) {
    assert.equal(selectedProviderAccountStartBlockReason([], mode, "claude-miguel"), null)
    assert.match(selectedProviderAccountStartBlockReason([], mode, "default") ?? "", /needs sign-in/)
  }
})

// MP-08 / MP-10 / MP-11: a replica or retained alias is outside home inventory.
test("explicit accounts outside the home inventory defer authentication to the kernel", () => {
  const local = account("codex", "local", true)
  local.auth_state = "not_configured"
  assert.equal(selectedProviderAccountStartBlockReason([local], "codex", "replica-id", { kind: "local" }), null)
  assert.equal(selectedProviderAccountStartBlockReason([local], "codex", "retained-home-label"), null)
  assert.match(selectedProviderAccountStartBlockReason([local], "codex", "local") ?? "", /needs sign-in/)
  assert.match(selectedProviderAccountStartBlockReason([], "codex", "default") ?? "", /needs sign-in/)
})

// MP-08 / MP-11 OpenCode owns Go → Zen fallback: show exhaustion, never refuse before the harness runs.
test("exhausted OpenCode service stays visible but does not block the start", () => {
  const reset = Date.now() + 3_600_000
  const go = { ...usageMeter("go/monthly", "rolling_limit", "exhausted", reset), service_id: "opencode-go" }
  const zen = { ...usageMeter("zen/credits", "credit_balance", "exhausted"), service_id: "opencode" }
  const openCode = accountWithUsage("opencode", [go, zen])
  assert.match(providerAccountDisplayLabel(openCode, "opencode-go/glm-5.3-flash"), /OpenCode Go \(exhausted\).*resets.*UTC/)
  for (const model of ["opencode-go/glm-5.3-flash", "opencode/deepseek-v4-flash"]) {
    assert.equal(selectedProviderAccountStartBlockReason([openCode], "opencode", openCode.profile_id, null, model), null, model)
  }
})

import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"

import { providerAccountAdmissionBlocked, providerAccountCapacity, providerAccountCapacityResetLabel, providerAccountName, providerAccountStatusLabel } from "./provider-account-capacity.js"
import type { ProviderAccountProfile, ProviderAccountUsageMeter } from "./kernel-types-provider.js"

const nowMs = 10_000
const usage: ProviderAccountUsageMeter = {
  meter_id: "monthly",
  label: "Monthly",
  kind: "rolling_limit",
  scope: "plan",
  state: "exhausted",
  source: "test",
  observed_at_ms: nowMs,
  resets_at_ms: nowMs + 1_000,
}
const credits: ProviderAccountUsageMeter = {
  meter_id: "credits",
  label: "Credits",
  kind: "credit_balance",
  scope: "account",
  state: "exhausted",
  source: "test",
  observed_at_ms: nowMs,
}

test("blocks provider exhaustion but requires both allowance and credits for Claude and Codex", () => {
  const openCode = profile("opencode", [usage])
  assert.equal(providerAccountCapacity(openCode, nowMs).state, "warning")
  assert.equal(providerAccountName(openCode), "Account")

  for (const provider of ["codex", "claude", "claude-headless", "claude-p"]) {
    assert.equal(providerAccountCapacity(profile(provider, [usage]), nowMs).state, "warning")
    assert.equal(providerAccountCapacity(profile(provider, [usage, credits]), nowMs).state, "exhausted")
    assert.equal(providerAccountCapacity(profile(provider, [
      usage,
      { ...credits, state: "healthy" },
    ]), nowMs).detail, "usage allowance exhausted · credits available")
    assert.equal(providerAccountCapacity(profile(provider, [
      usage,
      { ...credits, state: "unknown" },
    ]), nowMs).detail, "usage allowance exhausted · credits not confirmed exhausted")
  }
})

test("does not block stale or reset usage", () => {
  assert.equal(providerAccountCapacity(profile("opencode", [
    { ...usage, resets_at_ms: nowMs },
  ]), nowMs).state, "ready")
  assert.equal(providerAccountCapacity({
    ...profile("opencode", [usage]),
    usage: { ...profile("opencode", [usage]).usage, availability: "stale" },
  }, nowMs).state, "unknown")
})

test("does not refresh stale Zen capacity from a fresh Go observation", () => {
  const staleZen = {
    ...credits,
    service_id: "opencode",
    observed_at_ms: nowMs - 24 * 60 * 60 * 1000 - 1,
  }
  const freshGo = {
    ...usage,
    service_id: "opencode-go",
    state: "healthy" as const,
  }
  const account = profile("opencode", [staleZen, freshGo])

  assert.equal(providerAccountCapacity(account, nowMs, "opencode/deepseek-v4-pro").state, "unknown")
  assert.equal(providerAccountCapacity(account, nowMs, "opencode/deepseek-v4-pro").detail, "OpenCode Zen balance stale")
  assert.equal(providerAccountCapacity(account, nowMs, "opencode-go/deepseek-v4-pro").state, "ready")
})

test("scopes OpenCode Go exhaustion to Go rather than Zen or arbitrary upstream providers", () => {
  const openCode = profile("opencode", [{ ...usage, service_id: "opencode-go" }])

  assert.equal(providerAccountCapacity(openCode, nowMs, "opencode-go/deepseek-v4-pro").state, "exhausted")
  assert.equal(providerAccountCapacity(openCode, nowMs, "opencode/gpt-5.2").state, "unknown")
  assert.equal(providerAccountCapacity(openCode, nowMs, "opencode/gpt-5.2").detail, "OpenCode Zen balance not reported")
  assert.equal(providerAccountCapacity(openCode, nowMs, "gpt-5.2").detail, "OpenCode Zen balance not reported")
  assert.equal(providerAccountCapacity(openCode, nowMs, "openai/gpt-5.2").state, "unknown")
  assert.equal(providerAccountName(openCode, "opencode-go/deepseek-v4-pro"), "Account · OpenCode Go")
  assert.equal(providerAccountStatusLabel(openCode, nowMs, "opencode-go/deepseek-v4-pro"), "usage limit until 1 Jan")
  assert.equal(providerAccountCapacity(openCode, nowMs).state, "warning")

  const zenExhausted = profile("opencode", [{ ...usage, service_id: "opencode" }])
  assert.equal(providerAccountCapacity(zenExhausted, nowMs, "gpt-5.2").state, "exhausted")
})

test("Codex general and Spark allowance exhaustion are independent", () => {
  const general = { ...usage, meter_id: "rolling/10080/codex" }
  const spark = { ...usage, meter_id: "rolling/300/codex_bengalfox", state: "healthy" as const }
  const account = profile("codex", [general, spark, credits])
  assert.equal(providerAccountCapacity(account, nowMs, "gpt-5.6-luna").state, "exhausted")
  assert.equal(providerAccountCapacity(account, nowMs, "gpt-5.3-codex-spark").state, "ready")
  assert.equal(providerAccountCapacity(account, nowMs, "codex/gpt-5.3-codex-spark").state, "ready")
  const reverse = profile("codex", [{ ...general, state: "healthy" }, { ...spark, state: "exhausted" }, credits])
  assert.equal(providerAccountCapacity(reverse, nowMs, "gpt-5.6-luna").state, "ready")
  assert.equal(providerAccountCapacity(reverse, nowMs, "gpt-5.3-codex-spark").state, "exhausted")
  assert.equal(providerAccountCapacity(reverse, nowMs, "codex/gpt-5.3-codex-spark").state, "exhausted")
})

test("presentation capacity matches the shared kernel admission fixtures", () => {
  type FixtureMeter = Pick<ProviderAccountUsageMeter,
    "meter_id" | "label" | "kind" | "state" | "observed_at_ms"
  > & Partial<Pick<ProviderAccountUsageMeter, "service_id" | "resets_at_ms">>
  type Fixture = {
    readonly name: string
    readonly provider: string
    readonly model: string
    readonly now_ms: number
    readonly meters: readonly FixtureMeter[]
    readonly expected_state: ReturnType<typeof providerAccountCapacity>["state"]
    readonly expected_blocked: boolean
  }
  const fixtures = JSON.parse(readFileSync(
    new URL("../../../fixtures/provider-account-capacity.json", import.meta.url),
    "utf8",
  )) as readonly Fixture[]

  for (const fixture of fixtures) {
    const meters = fixture.meters.map((meter): ProviderAccountUsageMeter => ({
      ...meter,
      scope: "account",
      source: "shared_fixture",
    }))
    const state = providerAccountCapacity(profile(fixture.provider, meters), fixture.now_ms, fixture.model).state
    assert.equal(state, fixture.expected_state, fixture.name)
    assert.equal(providerAccountAdmissionBlocked(profile(fixture.provider, meters), fixture.now_ms, fixture.model), fixture.expected_blocked, `${fixture.name} admission contract`)
  }
})

function profile(provider: string, meters: ProviderAccountUsageMeter[]): ProviderAccountProfile {
  return {
    owner_user_id: "owner",
    provider,
    profile_id: "account",
    label: "Account",
    origin: "default",
    is_default: true,
    auth_state: "authenticated",
    usage: {
      profile_id: "account",
      provider,
      availability: "available",
      meters,
      source: "test",
    },
    materializations: [],
  }
}

test("inline status names only what stops an account, and healthy accounts have none", () => {
  const day = Date.UTC(2026, 9, 22, 6, 13)
  const exhausted = (provider: string, resetsAt: number | null) => profile(provider, [
    { ...usage, resets_at_ms: resetsAt, observed_at_ms: day - 1_000 },
    { ...credits, observed_at_ms: day - 1_000 },
  ])
  assert.equal(providerAccountStatusLabel(profile("codex", []), nowMs), null)
  assert.equal(providerAccountStatusLabel(profile("codex", [{ ...usage, state: "healthy" }]), nowMs), null)
  assert.equal(providerAccountStatusLabel(exhausted("codex", day), day - 1_000), "usage limit until 22 Oct")
  assert.equal(providerAccountStatusLabel(exhausted("claude", null), day - 1_000), "usage limit reached")
  const later = { ...usage, meter_id: "weekly", resets_at_ms: day + 86_400_000, observed_at_ms: day - 1_000 }
  assert.equal(providerAccountStatusLabel(profile("codex", [
    { ...usage, resets_at_ms: day, observed_at_ms: day - 1_000 }, later, { ...credits, observed_at_ms: day - 1_000 },
  ]), day - 1_000), "usage limit until 23 Oct")
  // Admission reopens when the allowance windows all reset or any credit meter resets, whichever is first.
  const at = day - 3 * 86_400_000
  const weekly = { ...usage, resets_at_ms: day, observed_at_ms: at }
  const spend = { ...credits, meter_id: "spend", kind: "spend_limit" as const, observed_at_ms: at }
  const creditsBack = { ...spend, resets_at_ms: day - 86_400_000 }
  assert.equal(providerAccountStatusLabel(profile("codex", [weekly, creditsBack]), at), "usage limit until 21 Oct")
  assert.equal(providerAccountCapacityResetLabel(profile("codex", [weekly, creditsBack]), at), "resets 2026-10-21 06:13 UTC")
  const creditsLater = { ...spend, resets_at_ms: day + 86_400_000 }
  assert.equal(providerAccountStatusLabel(profile("codex", [weekly, creditsLater]), at), "usage limit until 22 Oct")
  assert.equal(providerAccountStatusLabel(profile("codex", [weekly, spend]), at), "usage limit until 22 Oct")
  // An exhausted allowance with no scheduled reset keeps the account limited: never drop it or invent a date.
  const undated = { ...usage, meter_id: "five-hour", resets_at_ms: null, observed_at_ms: day - 1_000 }
  const dated = { ...usage, resets_at_ms: day, observed_at_ms: day - 1_000 }
  const credit = { ...credits, observed_at_ms: day - 1_000 }
  assert.equal(providerAccountStatusLabel(profile("codex", [undated, credit]), day - 1_000), "usage limit reached")
  assert.equal(providerAccountStatusLabel(profile("codex", [dated, credit]), day - 1_000), "usage limit until 22 Oct")
  assert.equal(providerAccountStatusLabel(profile("codex", [undated, dated, credit]), day - 1_000), "usage limit reached")
  assert.equal(providerAccountCapacityResetLabel(profile("codex", [undated, dated, credit]), day - 1_000), null)
  const creditBack = { ...credit, resets_at_ms: day + 86_400_000 }
  assert.equal(providerAccountStatusLabel(profile("codex", [undated, dated, creditBack]), day - 1_000), "usage limit until 23 Oct")
  assert.equal(providerAccountStatusLabel(profile("opencode", [{ ...dated, service_id: "opencode" }, { ...undated, meter_id: "zen", service_id: "opencode" }]), day - 1_000, "opencode/x"), "usage limit reached")
  for (const [authState, label] of [
    ["unknown", "sign-in not checked"],
    ["not_configured", "sign in"],
    ["expired", "sign in again"],
    ["error", "sign in again"],
  ] as const) {
    assert.equal(providerAccountStatusLabel({ ...profile("codex", []), auth_state: authState }, nowMs), label, authState)
  }
})

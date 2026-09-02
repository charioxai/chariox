import assert from "node:assert/strict"
import test from "node:test"

import { providerAccountCapacity, providerAccountCapacityLabel } from "./provider-account-capacity.js"
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
  assert.equal(providerAccountCapacity(openCode, nowMs).state, "exhausted")
  assert.equal(providerAccountCapacityLabel(openCode, nowMs), "Account (exhausted)")

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

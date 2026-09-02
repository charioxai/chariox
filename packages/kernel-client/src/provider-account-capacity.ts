import type { ProviderAccountProfile, ProviderAccountUsageMeter } from "./kernel-types-provider.js"

export type ProviderAccountCapacityState = "ready" | "warning" | "exhausted" | "unknown"

export type ProviderAccountCapacity = {
  readonly state: ProviderAccountCapacityState
  readonly detail: string
}

export function providerAccountCapacity(
  profile: ProviderAccountProfile,
  nowMs = Date.now(),
): ProviderAccountCapacity {
  const usage = profile.usage
  if (!usage || !["available", "partial"].includes(usage.availability)) {
    return {
      state: "unknown",
      detail: usage?.availability === "stale" ? "usage status stale" : "usage status not reported",
    }
  }
  const meters = usage.meters ?? []
  const currentlyExhausted = (meter: ProviderAccountUsageMeter): boolean => (
    meter.state === "exhausted"
    && (meter.resets_at_ms == null || meter.resets_at_ms > nowMs)
  )
  const exhausted = meters.filter(currentlyExhausted)
  const provider = profile.provider === "claude-headless" || profile.provider === "claude-p"
    ? "claude"
    : profile.provider
  if (provider === "codex" || provider === "claude") {
    const exhaustedUsage = exhausted.filter((meter) => meter.kind !== "credit_balance" && meter.kind !== "spend_limit")
    const creditCapacity = meters.filter((meter) => meter.kind === "credit_balance" || meter.kind === "spend_limit")
    if (exhaustedUsage.length && creditCapacity.length && creditCapacity.every(currentlyExhausted)) {
      return { state: "exhausted", detail: "usage allowance and credits exhausted" }
    }
    if (exhaustedUsage.length) {
      const creditDetail = creditCapacity.length === 0
        ? "credits not reported"
        : creditCapacity.some((meter) => meter.state === "healthy" || meter.state === "warning")
          ? "credits available"
          : "credits not confirmed exhausted"
      return {
        state: "warning",
        detail: `usage allowance exhausted · ${creditDetail}`,
      }
    }
  } else if (exhausted.length) {
    return { state: "exhausted", detail: "usage exhausted" }
  }
  if (meters.some((meter) => meter.state === "warning")) {
    return { state: "warning", detail: "usage nearing limit" }
  }
  return { state: "ready", detail: "usage available" }
}

export function providerAccountCapacityLabel(profile: ProviderAccountProfile, nowMs = Date.now()): string {
  return providerAccountCapacity(profile, nowMs).state === "exhausted"
    ? `${profile.label} (exhausted)`
    : profile.label
}

import type { ProviderAccountProfile, ProviderAccountUsageMeter } from "./kernel-types-provider.js"

export type ProviderAccountCapacityState = "ready" | "warning" | "exhausted" | "unknown"

export type ProviderAccountCapacity = {
  readonly state: ProviderAccountCapacityState
  readonly detail: string
}

export function providerAccountCapacity(
  profile: ProviderAccountProfile,
  nowMs = Date.now(),
  model?: string | null,
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
  const provider = profile.provider === "claude-headless" || profile.provider === "claude-p"
    ? "claude"
    : profile.provider
  const selectedOpenCodeService = provider === "opencode" ? openCodeServiceFromModel(model) : null
  const relevantMeters = provider === "opencode" && selectedOpenCodeService
    ? meters.filter((meter) => openCodeMeterService(meter) === selectedOpenCodeService)
    : provider === "opencode"
      ? meters.filter((meter) => openCodeMeterService(meter) == null)
      : meters
  const exhausted = relevantMeters.filter(currentlyExhausted)
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
  if (relevantMeters.some((meter) => meter.state === "warning")) {
    return { state: "warning", detail: "usage nearing limit" }
  }
  if (provider === "opencode" && selectedOpenCodeService && relevantMeters.length === 0) {
    return {
      state: "unknown",
      detail: `${openCodeServiceLabel(profile, selectedOpenCodeService)} balance not reported`,
    }
  }
  if (provider === "opencode" && !selectedOpenCodeService && meters.some((meter) => currentlyExhausted(meter) && openCodeMeterService(meter))) {
    return { state: "warning", detail: "one OpenCode service is exhausted" }
  }
  return { state: "ready", detail: "usage available" }
}

export function providerAccountCapacityLabel(profile: ProviderAccountProfile, nowMs = Date.now(), model?: string | null): string {
  const serviceId = profile.provider === "opencode" ? openCodeServiceFromModel(model) : null
  const serviceLabel = serviceId ? openCodeServiceLabel(profile, serviceId) : null
  const base = serviceLabel && profile.label.localeCompare(serviceLabel, "en", { sensitivity: "base" }) !== 0
    ? `${profile.label} · ${serviceLabel}`
    : profile.label
  return providerAccountCapacity(profile, nowMs, model).state === "exhausted"
    ? `${base} (exhausted)`
    : base
}

function openCodeServiceFromModel(model?: string | null): string | null {
  const normalized = model?.trim()
  if (!normalized || normalized === "default") return null
  const separator = normalized.indexOf("/")
  if (separator === -1) return "opencode"
  return separator > 0 && separator < normalized.length - 1 ? normalized.slice(0, separator) : null
}

function openCodeMeterService(meter: ProviderAccountUsageMeter): string | null {
  if (meter.service_id) return meter.service_id
  return meter.meter_id.startsWith("go/") ? "opencode-go" : null
}

function openCodeServiceLabel(profile: ProviderAccountProfile, serviceId: string): string {
  const reported = profile.services?.find((service) => service.service_id === serviceId)?.label
  if (reported) return reported
  if (serviceId === "opencode-go") return "OpenCode Go"
  if (serviceId === "opencode") return "OpenCode Zen"
  return serviceId
}

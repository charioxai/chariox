import type { ProviderAccountProfile, ProviderAccountUsageMeter } from "./kernel-types-provider.js"

export type ProviderAccountCapacityState = "ready" | "warning" | "exhausted" | "unknown"

export type ProviderAccountCapacity = {
  readonly state: ProviderAccountCapacityState
  readonly detail: string
}

const PROVIDER_USAGE_STALE_AFTER_MS = 24 * 60 * 60 * 1000

function meterCurrentlyExhausted(meter: ProviderAccountUsageMeter, nowMs: number): boolean {
  return nowMs - meter.observed_at_ms <= PROVIDER_USAGE_STALE_AFTER_MS
    && meter.state === "exhausted"
    && (meter.resets_at_ms == null || meter.resets_at_ms > nowMs)
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
  const isFresh = (meter: ProviderAccountUsageMeter): boolean => (
    nowMs - meter.observed_at_ms <= PROVIDER_USAGE_STALE_AFTER_MS
  )
  const currentlyExhausted = (meter: ProviderAccountUsageMeter): boolean => meterCurrentlyExhausted(meter, nowMs)
  const provider = profile.provider === "claude-headless" || profile.provider === "claude-p"
    ? "claude"
    : profile.provider
  const selectedOpenCodeService = provider === "opencode" ? openCodeServiceFromModel(model) : null
  const relevantMeters = providerAccountModelMeters(profile, model)
  const freshRelevantMeters = relevantMeters.filter(isFresh)
  const exhausted = freshRelevantMeters.filter(currentlyExhausted)
  if (provider === "codex" || provider === "claude") {
    const exhaustedUsage = exhausted.filter((meter) => meter.kind !== "credit_balance" && meter.kind !== "spend_limit")
    const creditCapacity = meters.filter((meter) => (
      meter.kind === "credit_balance" || meter.kind === "spend_limit"
    ))
    if (exhaustedUsage.length && creditCapacity.length && creditCapacity.every(currentlyExhausted)) {
      return { state: "exhausted", detail: "usage allowance and credits exhausted" }
    }
    if (exhaustedUsage.length) {
      const creditDetail = creditCapacity.length === 0
        ? "credits not reported"
        : creditCapacity.some((meter) => isFresh(meter) && (meter.state === "healthy" || meter.state === "warning"))
          ? "credits available"
          : "credits not confirmed exhausted"
      return {
        state: "warning",
        detail: `usage allowance exhausted · ${creditDetail}`,
      }
    }
  } else if (exhausted.length && (provider !== "opencode" || selectedOpenCodeService)) {
    return { state: "exhausted", detail: "usage exhausted" }
  }
  if (relevantMeters.length > 0 && freshRelevantMeters.length === 0) {
    const staleDetail = provider === "opencode" && selectedOpenCodeService
      ? `${openCodeServiceLabel(profile, selectedOpenCodeService)} balance stale`
      : "usage status stale"
    return { state: "unknown", detail: staleDetail }
  }
  if (freshRelevantMeters.some((meter) => meter.state === "warning")) {
    return { state: "warning", detail: "usage nearing limit" }
  }
  if (provider === "opencode" && selectedOpenCodeService && relevantMeters.length === 0) {
    return {
      state: "unknown",
      detail: `${openCodeServiceLabel(profile, selectedOpenCodeService)} balance not reported`,
    }
  }
  if (provider === "opencode" && !selectedOpenCodeService && meters.some(currentlyExhausted)) {
    return { state: "warning", detail: "one OpenCode service is exhausted" }
  }
  return { state: "ready", detail: "usage available" }
}

/** Pre-admission gate. OpenCode owns service fallback (Go may draw Zen credits), so only its own refusal stops a run. */
export function providerAccountAdmissionBlocked(profile: ProviderAccountProfile, nowMs = Date.now(), model?: string | null): boolean {
  return profile.provider !== "opencode" && providerAccountCapacity(profile, nowMs, model).state === "exhausted"
}

/** The account name, qualified by the OpenCode service when the selected model fixes one. */
export function providerAccountName(profile: ProviderAccountProfile, model?: string | null): string {
  const serviceId = profile.provider === "opencode" ? openCodeServiceFromModel(model) : null
  const serviceLabel = serviceId ? openCodeServiceLabel(profile, serviceId) : null
  return serviceLabel && profile.label.localeCompare(serviceLabel, "en", { sensitivity: "base" }) !== 0
    ? `${profile.label} · ${serviceLabel}`
    : profile.label
}

/**
 * The one short reason an account cannot take work right now, shown inline
 * after its name. Healthy accounts have no status.
 */
export function providerAccountStatusLabel(profile: ProviderAccountProfile, nowMs = Date.now(), model?: string | null): string | null {
  if (profile.auth_state === "unknown") return "sign-in not checked"
  if (profile.auth_state === "not_configured") return "sign in"
  if (profile.auth_state !== "authenticated") return "sign in again"
  if (providerAccountCapacity(profile, nowMs, model).state !== "exhausted") return null
  const until = providerAccountRecoveryMs(profile, nowMs, model)
  return until === null ? "usage limit reached" : `usage limit until ${utcDayMonth(until)}`
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]

function utcDayMonth(ms: number): string {
  const date = new Date(ms)
  return `${date.getUTCDate()} ${MONTHS[date.getUTCMonth()]}`
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

export function providerAccountCapacityResetLabel(profile: ProviderAccountProfile, nowMs = Date.now(), model?: string | null): string | null {
  const reset = providerAccountRecoveryMs(profile, nowMs, model)
  return reset === null ? null : `resets ${new Date(reset).toISOString().slice(0, 16).replace("T", " ")} UTC`
}

/**
 * First scheduled instant an exhausted account is admitted again, from the same
 * condition as providerAccountCapacity: Codex/Claude reopen when every exhausted
 * allowance window resets or any credit/spend meter resets; other providers when
 * every exhausted meter resets. Null when nothing is scheduled.
 */
function providerAccountRecoveryMs(profile: ProviderAccountProfile, nowMs: number, model?: string | null): number | null {
  if (providerAccountCapacity(profile, nowMs, model).state !== "exhausted") return null
  // A group reopens when every exhausted meter has reset; one without a scheduled reset keeps it closed.
  const reopens = (meters: readonly ProviderAccountUsageMeter[]): number | null => {
    const live = meters.filter(meter => meterCurrentlyExhausted(meter, nowMs))
    return live.length && live.every(meter => meter.resets_at_ms != null) ? Math.max(...live.map(meter => meter.resets_at_ms!)) : null
  }
  const isCredit = (meter: ProviderAccountUsageMeter): boolean => meter.kind === "credit_balance" || meter.kind === "spend_limit"
  const modelMeters = providerAccountModelMeters(profile, model)
  const family = profile.provider.startsWith("claude") ? "claude" : profile.provider
  if (family !== "codex" && family !== "claude") return reopens(modelMeters)
  const credits = (profile.usage?.meters ?? []).filter(isCredit)
  const options = [reopens(modelMeters.filter(meter => !isCredit(meter))), ...credits.map(meter => reopens([meter]))]
    .filter((reset): reset is number => reset !== null)
  return options.length ? Math.min(...options) : null
}

function providerAccountModelMeters(profile: ProviderAccountProfile, model?: string | null) {
  const meters = profile.usage?.meters ?? []
  const family = profile.provider.startsWith("claude") ? "claude" : profile.provider
  const selectedOpenCodeService = family === "opencode" ? openCodeServiceFromModel(model) : null
  const codexModel = model?.trim().replace(/^codex\//, "")
  return family === "opencode" && selectedOpenCodeService
    ? meters.filter((meter) => openCodeMeterService(meter) === selectedOpenCodeService)
    : family === "opencode"
      ? meters.filter((meter) => openCodeMeterService(meter) == null)
      : family === "codex" ? meters.filter((meter) => {
        if (!codexModel || codexModel === "default" || !meter.meter_id.startsWith("rolling/")) return true
        const bucket = meter.meter_id.split("/").at(-1)
        if (bucket !== "codex" && bucket !== "codex_bengalfox") return true
        return bucket === (codexModel === "gpt-5.3-codex-spark" ? "codex_bengalfox" : "codex")
      }) : meters
}

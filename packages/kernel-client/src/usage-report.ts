// MP-08 / MP-10 / MP-11: projection only; kernel owns counting and quotes.
import type { SessionUsageReport, ProviderUsageTotals } from "./kernel-types-provider.js"
function line(name: string, t: ProviderUsageTotals): string {
  const n = (v: number | null) => v === null ? "unavailable" : String(v)
  const nanos = t.api_equivalent_nanodollars === null ? null : BigInt(t.api_equivalent_nanodollars)
  const cost = nanos === null ? "unavailable" : `$${nanos / 1_000_000_000n}.${String(nanos % 1_000_000_000n).padStart(9, "0")}`
  return `${name}: ${t.turns} turns; input ${n(t.usage.input_tokens)}; cached ${n(t.usage.cached_input_tokens)}; output ${n(t.usage.output_tokens)}; reasoning ${n(t.usage.reasoning_tokens)}; ${cost} API-equivalent USD; ${t.unavailable_turns} unavailable turns`
}
export function formatSessionUsage(report: SessionUsageReport): string {
  return ["Usage (standard API-equivalent estimate; excludes subscription billing and infrastructure)", line("Session", report.total),
    ...Object.entries(report.agents).map(([id, t]) => line(`Agent ${id}`, t)),
    ...Object.entries(report.delegation_trees).map(([id, t]) => line(`Tree ${id}`, t))].join("\n")
}

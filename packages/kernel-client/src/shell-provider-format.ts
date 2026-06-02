import type {
  ProviderAuthStatus,
  ProviderLoginStart,
  ProviderProcessInfo,
} from "./kernel-types.js"

export function formatProviderAuthStatus(status: ProviderAuthStatus): string {
  return [
    status.account_profile ? `${status.provider}: ${status.auth_state} as ${status.account_profile}` : `${status.provider}: ${status.auth_state}`,
    status.detected_version ? `version ${status.detected_version}` : null,
    status.login_hint ?? null,
  ].filter(Boolean).join(" • ")
}

export function formatProviderLoginStart(login: ProviderLoginStart, verb: "login" | "reauth"): string {
  return [
    `${login.provider} ${verb} started`,
    login.user_code ? `code ${login.user_code}` : null,
    login.verification_url ?? login.auth_url ?? null,
  ].filter(Boolean).join(" • ")
}

export function formatProviderProcesses(processes: ProviderProcessInfo[]): string {
  if (processes.length === 0) {
    return "no daemon-tracked provider processes"
  }
  return processes.map((process) => {
    const blockers = process.teardown_blockers.length > 0 ? ` blockers=${process.teardown_blockers.join(",")}` : ""
    return `${process.process_id} ${process.provider} ${process.process_label} pid=${process.pid ?? "-"} rss=${formatBytes(process.resident_set_bytes ?? null)} status=${process.status} safe=${String(process.teardown_safe)} sessions=${process.owner_session_ids.join(",") || "-"}${blockers} next=${providerProcessNextAction(process)}`
  }).join("\n")
}

function providerProcessNextAction(process: ProviderProcessInfo): string {
  const teardown = `run /provider processes teardown ${process.provider}`
  if (process.teardown_safe) {
    return `${teardown} to stop only safe daemon-tracked processes owned by you`
  }
  if (process.attached_session_ids.length > 0) {
    return `detach or finish attached sessions ${process.attached_session_ids.join(",")}; then ${teardown}`
  }
  if (process.active_workflow_run_ids.length > 0) {
    return `stop or finish workflow runs ${process.active_workflow_run_ids.join(",")}; then ${teardown}`
  }
  if (process.teardown_blockers.length > 0) {
    return `resolve blockers: ${process.teardown_blockers.join("; ")}; then ${teardown}`
  }
  return `inspect the owning session before teardown; then ${teardown}`
}

function formatBytes(bytes: number | null): string {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0) {
    return "unknown"
  }
  const units = ["B", "KiB", "MiB", "GiB"]
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  const formatted = unitIndex === 0 ? `${Math.round(value)}` : value >= 10 ? value.toFixed(1) : value.toFixed(2)
  return `${formatted}${units[unitIndex]}`
}

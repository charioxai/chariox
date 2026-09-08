import { getAppInstallationJournalRequest, getAppInstallationRequest, listAppInstallationsRequest } from "./ipc-app-requests.js"
import type { AppInstallationSummary, AppUpdateSummary } from "./kernel-types-apps.js"
import type { ShellCommandResult } from "./shell-core.js"

type Client = { send(request: Record<string, unknown>): Promise<Record<string, unknown>> }
const usage = "usage: app list [--after <installation-id>] [--limit <1..100>] | status <installation-id> | journal <installation-id>"

export async function executeAppCommand(args: string[], client: Client): Promise<ShellCommandResult> {
  const [action = "list", ...rest] = args
  let request: Record<string, unknown>
  if (action === "list") {
    const options: { after?: string; limit?: number } = {}
    for (let index = 0; index < rest.length; index += 2) {
      const flag = rest[index]
      const value = rest[index + 1]
      if (value == null || value.startsWith("--")) return { ok: false, message: usage }
      if (flag === "--after" && options.after == null) options.after = value
      else if (flag === "--limit" && options.limit == null && /^\d+$/.test(value) && Number(value) >= 1 && Number(value) <= 100) options.limit = Number(value)
      else return { ok: false, message: usage }
    }
    request = listAppInstallationsRequest(options)
  } else if ((action === "status" || action === "journal") && rest.length === 1 && rest[0]) {
    request = action === "status" ? getAppInstallationRequest(rest[0]) : getAppInstallationJournalRequest(rest[0])
  } else return { ok: false, message: usage }

  const response = await client.send(request)
  const failure = response.AppRequestFailed as { code?: string } | undefined
  if (failure) {
    const messages: Record<string, string> = {
      unauthorized: "This connection is not authorized to access Apps.",
      not_found: "App installation not found.",
      invalid_request: "Invalid App request.",
      busy: "App requests are busy. Try again shortly.",
      storage_unavailable: "App storage is unavailable.",
    }
    return { ok: false, message: messages[failure.code ?? ""] ?? "App request failed.", data: failure }
  }
  if (action === "list") {
    const page = expect<{ installations: AppInstallationSummary[]; next_cursor: string | null }>(response, "AppInstallationsListed")
    const lines = page.installations.map(formatInstallation)
    if (page.next_cursor) lines.push(`Next page: app list --after ${JSON.stringify(page.next_cursor)}`)
    return { ok: true, message: lines.join("\n") || "No App installations.", data: page }
  }
  if (action === "status") {
    const data = expect<{ installation: AppInstallationSummary }>(response, "AppInstallation")
    return { ok: true, message: formatInstallation(data.installation), data }
  }
  const data = expect<{ installation_id: string; updates: AppUpdateSummary[] }>(response, "AppInstallationJournal")
  return {
    ok: true,
    message: data.updates.map(update => `Generation ${update.generation}: ${update.release.version} · ${update.phase} · ${update.decision}`).join("\n") || "No retained App updates.",
    data,
  }
}

function formatInstallation(app: AppInstallationSummary): string {
  const version = app.active_release ? `version ${app.active_release.version}` : "no active release"
  const pending = app.pending_generation == null ? "" : `; pending generation ${app.pending_generation}`
  const paused = app.admission_paused ? "; admission paused" : ""
  return `${app.installation_id} · ${app.app_id} · ${version}; generation ${app.generation}${pending}${paused}`
}

function expect<T>(response: Record<string, unknown>, variant: string): T {
  const value = response[variant]
  if (value == null || typeof value !== "object" || Array.isArray(value)) throw new Error(`Expected ${variant} response`)
  return value as T
}

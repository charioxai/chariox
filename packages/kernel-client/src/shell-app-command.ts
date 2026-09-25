import {
  configureAppAutomationRequest, controlAppWorkerRequest, disableAppAutomationRequest, getAppInstallationJournalRequest,
  getAppInstallationRequest, getAppWorkerRequest, listAppAutomationsRequest, listAppInstallationsRequest,
  getAppLogsRequest, openAppViewRequest, uninstallAppRequest,
} from "./ipc-app-requests.js"
import type { AppAutomationSummary, AppInstallationSummary, AppUpdateSummary, AppWorkerSummary } from "./kernel-types-apps.js"
import type { ShellCommandResult } from "./shell-core.js"

type Client = { send(request: Record<string, unknown>): Promise<Record<string, unknown>> }
const usage = [
  "usage: app list [--after <installation-id>] [--limit <1..100>] | status <installation-id> | journal <installation-id>",
  "       app logs <installation-id> [--after <sequence>]",
  "       app worker <installation-id> | start <installation-id> | stop <installation-id> | restart <installation-id>",
  "       app open <installation-id> [--session <session-id>] | uninstall <installation-id> [--generation <n>]",
  "       app automation list <installation-id>",
  "       app automation add <installation-id> <automation-id> <event> <session-id> <workflow> [--queue <queue>] [--scheduled] [--revision <n>]",
  "       app automation disable <installation-id> <automation-id> <revision>",
].join("\n")

export async function executeAppCommand(
  args: string[],
  client: Client,
  defaults: { sessionId?: string | undefined } = {},
): Promise<ShellCommandResult> {
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
  } else if (action === "logs" && rest[0] && (rest.length === 1 || (rest.length === 3 && rest[1] === "--after" && /^\d{1,19}$/.test(rest[2] ?? "")))) {
    request = getAppLogsRequest(rest[0], rest[2])
  } else if (action === "worker" && rest.length === 1 && rest[0]) {
    request = getAppWorkerRequest(rest[0])
  } else if ((action === "start" || action === "stop" || action === "restart") && rest.length === 1 && rest[0]) {
    request = controlAppWorkerRequest(rest[0], action)
  } else if (action === "open" && rest[0] && (rest.length === 1 || (rest.length === 3 && rest[1] === "--session" && rest[2]))) {
    const sessionId = rest[2] ?? defaults.sessionId
    if (!sessionId) return { ok: false, message: "Attach to a session or pass --session to open an App view." }
    request = openAppViewRequest(sessionId, rest[0])
  } else if (action === "uninstall" && rest[0] && (rest.length === 1 || (rest.length === 3 && rest[1] === "--generation" && /^\d{1,19}$/.test(rest[2] ?? "")))) {
    // Fenced by --generation (the one the user reviewed) or, by default, the
    // generation read just now; the kernel refuses a stale one without effect.
    let generation = rest[2]
    if (!generation) {
      const current = await client.send(getAppInstallationRequest(rest[0]))
      const installation = (current.AppInstallation as { installation?: AppInstallationSummary } | undefined)?.installation
      if (!installation) return appFailure(current)
      generation = installation.generation
    }
    request = uninstallAppRequest(rest[0], generation)
  } else if (action === "automation") {
    const parsed = automationRequest(rest)
    if (!parsed) return { ok: false, message: usage }
    request = parsed
  } else return { ok: false, message: usage }

  const response = await client.send(request)
  if (response.AppRequestFailed) return appFailure(response, action)
  if (response.AppLogs) {
    const data = expect<{ installation_id: string; entries: AppLogEntry[] }>(response, "AppLogs")
    const lines = data.entries.map(formatLogEntry)
    const last = data.entries.at(-1)
    if (last) lines.push(`More: app logs ${data.installation_id} --after ${last.sequence}`)
    return { ok: true, message: lines.join("\n") || "No App log entries.", data }
  }
  if (action === "uninstall") {
    const data = expect<{ installation: AppInstallationSummary }>(response, "AppInstallation")
    return { ok: true, message: `Uninstalled ${data.installation.installation_id}. Its data is kept.`, data }
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
  if (response.AppViewOpened) {
    const data = expect<{ installation_id: string; target_id: string; origin: string; bound_agent_id?: string | null }>(response, "AppViewOpened")
    const bound = data.bound_agent_id ? ` Its tools are available to the focus agent (${data.bound_agent_id}).` : ""
    return { ok: true, message: `Opened ${data.installation_id} as a Room browser Tab (${data.origin}). Use /room view to see it.${bound}`, data }
  }
  if (response.AppWorker) {
    const data = expect<{ worker: AppWorkerSummary }>(response, "AppWorker")
    return { ok: true, message: formatWorker(data.worker), data }
  }
  if (response.AppAutomations) {
    const data = expect<{ installation_id: string; automations: AppAutomationSummary[] }>(response, "AppAutomations")
    return { ok: true, message: data.automations.map(formatAutomation).join("\n") || "No App automations.", data }
  }
  if (response.AppAutomation) {
    const data = expect<{ installation_id: string; automation: AppAutomationSummary }>(response, "AppAutomation")
    return { ok: true, message: formatAutomation(data.automation), data }
  }
  const data = expect<{ installation_id: string; updates: AppUpdateSummary[] }>(response, "AppInstallationJournal")
  return {
    ok: true,
    message: data.updates.map(update => `Generation ${update.generation}: ${update.release.version} · ${update.phase} · ${update.decision}`).join("\n") || "No retained App updates.",
    data,
  }
}

function automationRequest(args: string[]): Record<string, unknown> | null {
  const [verb, installation, ...rest] = args
  if (!installation) return null
  if (verb === "list" && rest.length === 0) return listAppAutomationsRequest(installation)
  if (verb === "disable" && rest.length === 2 && /^\d+$/.test(rest[1] ?? "")) {
    return disableAppAutomationRequest(installation, rest[0] ?? "", Number(rest[1]))
  }
  if (verb !== "add" || rest.length < 4) return null
  const [automationId, eventName, sessionId, publicationRef, ...flags] = rest
  let queueRef: string | undefined
  let scheduled = false
  let expectedRevision = 0
  for (let index = 0; index < flags.length; index += 1) {
    const flag = flags[index]
    const value = flags[index + 1]
    if (flag === "--scheduled") scheduled = true
    else if (flag === "--queue" && value && queueRef === undefined) { queueRef = value; index += 1 }
    else if (flag === "--revision" && value && /^\d+$/.test(value)) { expectedRevision = Number(value); index += 1 }
    else return null
  }
  return configureAppAutomationRequest({
    installationId: installation, automationId: automationId ?? "", expectedRevision,
    eventName: eventName ?? "", sessionId: sessionId ?? "", publicationRef: publicationRef ?? "",
    ...(queueRef === undefined ? {} : { queueRef }), scheduled,
  })
}

function formatWorker(worker: AppWorkerSummary): string {
  const enabled = worker.enabled ? "" : " (stopped by user)"
  const failure = worker.failure ? ` · ${worker.failure}` : ""
  return `${worker.installation_id} · ${worker.phase.replace("_", " ")}${enabled}${failure}`
}

function formatAutomation(automation: AppAutomationSummary): string {
  const scheduled = automation.scheduled ? " · scheduled" : ""
  return `${automation.automation_id} · ${automation.event_name} v${automation.event_version} → workflow ${automation.publication_id} (queue ${automation.queue_id}) · ${automation.status} · revision ${automation.revision}${scheduled}`
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

function appFailure(response: Record<string, unknown>, action?: string): ShellCommandResult {
  const failure = (response.AppRequestFailed ?? { code: "unexpected" }) as { code?: string }
  const messages: Record<string, string> = {
    unauthorized: "This connection is not authorized to access Apps.",
    not_found: action === "automation"
      ? "Not found: check the App installation, session, workflow or automation."
      : "App installation not found.",
    invalid_request: "Invalid App request.",
    busy: "App requests are busy. Try again shortly.",
    storage_unavailable: "App storage is unavailable.",
    conflict: "The request conflicts with the App's current state (for example a stale revision). Refresh and try again.",
    limit_exceeded: "An App limit was reached.",
  }
  return { ok: false, message: messages[failure.code ?? ""] ?? "App request failed.", data: failure }
}

type AppLogEntry = { sequence: string; at_ms: number; level: string; message: string; fields: Record<string, unknown> }

// App-authored text: C0/C1 controls and bidi overrides are shown escaped,
// never interpreted (U+009B is CSI in 8-bit terminals such as xterm.js).
function formatLogEntry(entry: AppLogEntry): string {
  const safe = (value: string) => value.replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, "0")}`)
  const fields = Object.keys(entry.fields ?? {}).length ? ` ${safe(JSON.stringify(entry.fields))}` : ""
  return `${new Date(entry.at_ms).toISOString()} ${entry.level.toUpperCase().padEnd(5)} ${safe(entry.message)}${fields}`
}

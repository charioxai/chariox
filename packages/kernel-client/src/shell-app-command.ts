import {
  configureAppAutomationRequest, controlAppWorkerRequest, disableAppAutomationRequest, getAppInstallationJournalRequest,
  getAppInstallationRequest, getAppWorkerRequest, listAppAutomationsRequest, listAppInstallationsRequest,
  getAppLogsRequest, openAppViewRequest, uninstallAppRequest, createAppInboxRouteRequest, listAppInboxRoutesRequest,
  type AppInboxConnection,
  removeAppInboxRouteRequest, testAppInboxRouteRequest,
  grantAppConnectionRequest, revokeAppConnectionRequest, listAppConnectionsRequest, getAppSetRequest,
  revokeAppFileGrantsRequest,
} from "./ipc-app-requests.js"
import type { AppAutomationSummary, AppConnectionSummary, AppInboxRouteSummary, AppInstallationSummary, AppUpdateSummary, AppWorkerSummary } from "./kernel-types-apps.js"
import { LocalIpcError } from "./local-ipc-error.js"
import type { ShellCommandResult } from "./shell-core.js"

type Client = { send(request: Record<string, unknown>): Promise<Record<string, unknown>> }
const usage = [
  "usage: app list [--after <installation-id>] [--limit <1..100>] | status <installation-id> | journal <installation-id> | set",
  "       app logs <installation-id> [--after <sequence>]",
  "       app worker <installation-id> | start <installation-id> | stop <installation-id> | restart <installation-id>",
  "       app open <installation-id> [--session <session-id>] | uninstall <installation-id> [--generation <n>] [--delete-data]",
  "       app automation list <installation-id>",
  "       app automation add <installation-id> <automation-id> <event> <session-id> <workflow> [--queue <queue>] [--scheduled] [--delivery queue|inject] [--revision <n>]",
  "       app automation disable <installation-id> <automation-id> <revision>",
  "       app inbox list <installation-id> | remove <installation-id> <route-id>",
  "       app inbox add <installation-id> <route-id> <event> <source-event-type> [--version <n>] [--connection <generator>/<connection-id>/<scope>]",
  "       app inbox test <installation-id> <route-id> <occurrence-id> '<json-payload>'",
  "       app connection list <installation-id> | grant <installation-id> <generator>/<connection-id> | revoke <installation-id> <connection-id>",
  "       app file revoke <installation-id> [<operation-id>]",
].join("\n")

/** `/app` arguments with an `inbox test` payload taken from the raw line, so
 * every shell passes the JSON exactly as typed, quotes and spaces included. */
export function appCommandArgs(raw: string, args: readonly string[]): string[] {
  const test = /^\/?app\s+inbox\s+test\s+(\S+)\s+(\S+)\s+(\S+)\s+([\s\S]+)$/.exec(raw.trim())
  return test ? ["inbox", "test", test[1]!, test[2]!, test[3]!, test[4]!.trim()] : [...args]
}

export async function executeAppCommand(
  args: string[],
  client: Client,
  defaults: { sessionId?: string | undefined; appCommandPrefix?: string } = {},
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
  } else if (action === "set" && rest.length === 0) {
    request = getAppSetRequest()
  } else if ((action === "status" || action === "journal") && rest.length === 1 && rest[0]) {
    request = action === "status" ? getAppInstallationRequest(rest[0]) : getAppInstallationJournalRequest(rest[0])
  } else if (action === "logs" && rest[0] && (rest.length === 1 || (rest.length === 3 && rest[1] === "--after" && /^\d{1,19}$/.test(rest[2] ?? "")))) {
    request = getAppLogsRequest(rest[0], rest[2])
  } else if (action === "worker" && rest.length === 1 && rest[0]) {
    request = getAppWorkerRequest(rest[0])
  } else if ((action === "start" || action === "stop" || action === "restart") && rest.length === 1 && rest[0]) {
    request = controlAppWorkerRequest(rest[0], action)
  } else if (action === "open") {
    if (!rest[0] || rest[0].startsWith("--") || !(rest.length === 1 ||
      (rest.length === 3 && rest[1] === "--session" && rest[2] && !rest[2].startsWith("--")))) {
      return { ok: false, message: "usage: app open <installation-id> [--session <session-id>]" }
    }
    const sessionId = rest[2] ?? defaults.sessionId
    if (!sessionId) return { ok: false, message: "Attach to a session or pass --session to open an App view." }
    request = openAppViewRequest(sessionId, rest[0])
  } else if (action === "uninstall" && rest[0] && uninstallFlags(rest.slice(1))) {
    // Fenced by --generation (the one the user reviewed) or, by default, the
    // generation read just now; the kernel refuses a stale one without effect.
    const flags = uninstallFlags(rest.slice(1))!
    let generation = flags.generation
    if (!generation) {
      const current = await client.send(getAppInstallationRequest(rest[0]))
      const installation = (current.AppInstallation as { installation?: AppInstallationSummary } | undefined)?.installation
      if (!installation) return appFailure(current)
      generation = installation.generation
    }
    request = uninstallAppRequest(rest[0], generation, flags.deleteData)
  } else if (action === "inbox") {
    const parsed = inboxRequest(rest)
    if (!parsed) return { ok: false, message: usage }
    request = parsed
  } else if (action === "connection") {
    const [verb, installation, target] = rest
    if (verb === "list" && installation && rest.length === 2) request = listAppConnectionsRequest(installation)
    else if (verb === "revoke" && installation && target && rest.length === 3) request = revokeAppConnectionRequest(installation, target)
    else if (verb === "grant" && installation && target && rest.length === 3 && /^[^/]+\/[^/]+$/.test(target)) {
      const [generatorId = "", connectionId = ""] = target.split("/")
      request = grantAppConnectionRequest(installation, generatorId, connectionId)
    } else return { ok: false, message: usage }
  } else if (action === "file" && rest[0] === "revoke" && rest[1] && rest.length <= 3) {
    request = revokeAppFileGrantsRequest(rest[1], rest[2])
  } else if (action === "automation") {
    const parsed = automationRequest(rest)
    if (!parsed) return { ok: false, message: usage }
    request = parsed
  } else return { ok: false, message: usage }

  let response: Record<string, unknown>
  try {
    response = await client.send(request)
  } catch (error) {
    // Only a worker control or an uninstall ends this way: the client does not
    // resend one the kernel may be running, so its outcome is checked, not retried.
    if (!(error instanceof LocalIpcError && error.code === "outcome_unknown")) throw error
    const check = action === "uninstall" ? `app status ${rest[0]}` : `app worker ${rest[0]}`
    return { ok: false, message: `The kernel did not answer, so the ${action} may still happen. Check with ${check} before trying again.` }
  }
  if (response.AppRequestFailed) return appFailure(response, action === "connection" || action === "file" ? `${action} ${rest[0]}` : action)
  if (response.AppLogs) {
    const data = expect<{ installation_id: string; entries: AppLogEntry[] }>(response, "AppLogs")
    const lines = data.entries.map(formatLogEntry)
    const last = data.entries.at(-1)
    if (last) lines.push(`More: app logs ${data.installation_id} --after ${last.sequence}`)
    return { ok: true, message: lines.join("\n") || "No App log entries.", data }
  }
  if (action === "uninstall") {
    const data = expect<{ installation: AppInstallationSummary }>(response, "AppInstallation")
    const kept = data.installation.data_kept
      ? `Its data is kept: app update ${data.installation.installation_id} <package> reinstalls into it.`
      : "Its data is deleted."
    return { ok: true, message: `Uninstalled ${data.installation.installation_id}. ${kept}`, data }
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
  if (response.AppConnections) {
    const data = expect<{ installation_id: string; connections: AppConnectionSummary[] }>(response, "AppConnections")
    const lines = data.connections.map((connection) =>
      `${connection.connection_id} · ${connection.generator_id} · actions: ${connection.actions.join(", ") || "none declared"}`)
    return { ok: true, message: lines.join("\n") || "No connections granted to this App.", data }
  }
  if (response.AppFileGrantsRevoked) {
    const data = expect<{ installation_id: string; requests: number; files: number }>(response, "AppFileGrantsRevoked")
    if (data.requests === 0) return { ok: true, message: `No open file requests or unused grants for ${data.installation_id}.`, data }
    const plural = (count: number, noun: string) => `${count} ${noun}${count === 1 ? "" : "s"}`
    return { ok: true, message: `Revoked ${plural(data.requests, "file request")} for ${data.installation_id}; `
      + `${plural(data.files, "granted file")} the App had not imported ${data.files === 1 ? "was" : "were"} dropped. `
      + "Files it already imported stay in its data.", data }
  }
  if (response.AppSet) {
    const data = expect<{ schema: string; installations: Array<{ installation_id: string; app_id: string;
      release: { version: string }; automations: unknown[]; inbox_routes: unknown[]; connections: unknown[] }> }>(response, "AppSet")
    const plural = (count: number, noun: string) => `${count} ${noun}${count === 1 ? "" : "s"}`
    const lines = data.installations.map((installation) => `${installation.installation_id} · ${installation.app_id} v${installation.release.version} · `
      + `${plural(installation.automations.length, "automation")}, ${plural(installation.inbox_routes.length, "inbox route")}, `
      + `${plural(installation.connections.length, "connection")}`)
    return { ok: true, message: [`App set (${data.schema}): ${plural(data.installations.length, "active installation")}`, ...lines].join("\n"), data }
  }
  if (response.AppWorker) {
    const data = expect<{ worker: AppWorkerSummary }>(response, "AppWorker")
    return { ok: true, message: formatWorker(data.worker, defaults.appCommandPrefix ?? "app"), data }
  }
  if (response.AppAutomations) {
    const data = expect<{ installation_id: string; automations: AppAutomationSummary[] }>(response, "AppAutomations")
    return { ok: true, message: data.automations.map(formatAutomation).join("\n") || "No App automations.", data }
  }
  if (response.AppInboxRoutes) {
    const data = expect<{ installation_id: string; routes: AppInboxRouteSummary[] }>(response, "AppInboxRoutes")
    return { ok: true, message: data.routes.map(formatInboxRoute).join("\n") || "No App inbox routes.", data }
  }
  if (response.AppInboxOccurrenceAccepted) {
    const data = expect<{ route_id: string; occurrence_id: string; duplicate: boolean }>(response, "AppInboxOccurrenceAccepted")
    const note = data.duplicate ? "was already accepted" : "accepted; the App receives it shortly"
    return { ok: true, message: `Occurrence ${data.occurrence_id} on ${data.route_id} ${note}.`, data }
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
  let deliveryMode: "queue" | "inject" = "queue"
  let expectedRevision = 0
  for (let index = 0; index < flags.length; index += 1) {
    const flag = flags[index]
    const value = flags[index + 1]
    if (flag === "--scheduled") scheduled = true
    else if (flag === "--delivery") {
      if (value !== "queue" && value !== "inject") return null
      deliveryMode = value
      index += 1
    }
    else if (flag === "--queue" && value && queueRef === undefined) { queueRef = value; index += 1 }
    else if (flag === "--revision" && value && /^\d+$/.test(value)) { expectedRevision = Number(value); index += 1 }
    else return null
  }
  return configureAppAutomationRequest({
    installationId: installation, automationId: automationId ?? "", expectedRevision,
    eventName: eventName ?? "", sessionId: sessionId ?? "", publicationRef: publicationRef ?? "",
    ...(queueRef === undefined ? {} : { queueRef }), scheduled, deliveryMode,
  })
}

/** `app inbox ...` arguments to a kernel request, or null when malformed. */
export function inboxRequest(args: string[]): Record<string, unknown> | null {
  const [verb, installation, ...rest] = args
  if (!installation) return null
  if (verb === "list" && rest.length === 0) return listAppInboxRoutesRequest(installation)
  if (verb === "remove" && rest.length === 1 && rest[0]) return removeAppInboxRouteRequest(installation, rest[0])
  // The JSON payload is the rest of the line: a terminal splits it at spaces.
  if (verb === "test" && rest.length >= 3 && rest[0] && rest[1]) {
    try {
      return testAppInboxRouteRequest(installation, rest[0], rest[1], JSON.parse(rest.slice(2).join(" ")))
    } catch {
      return null
    }
  }
  if (verb !== "add" || rest.length < 3 || rest.length % 2 === 0) return null
  const [routeId, eventName, sourceEventType, ...flags] = rest
  if (!routeId || !eventName || !sourceEventType) return null
  let version = 1
  let connection: AppInboxConnection | undefined
  for (let index = 0; index < flags.length; index += 2) {
    const [flag, value = ""] = [flags[index], flags[index + 1]]
    if (flag === "--version" && /^[1-9]\d{0,8}$/.test(value)) version = Number(value)
    else if (flag === "--connection") {
      // generator/connection-id/scope; the scope may itself contain slashes.
      const [generatorId, connectionId, ...scope] = value.split("/")
      if (!generatorId || !connectionId || scope.length === 0 || !scope.join("/")) return null
      connection = { generatorId, connectionId, connectionScope: scope.join("/") }
    } else return null
  }
  return createAppInboxRouteRequest({
    installationId: installation, routeId, eventName, sourceEventType, sourceEventVersion: version,
    ...(connection ? { connection } : {}),
  })
}

function formatInboxRoute(route: AppInboxRouteSummary): string {
  const counts = `${route.pending} pending, ${route.delivered} delivered, ${route.failed} failed, ${route.expired} expired`
  const source = route.connection ? ` from ${route.connection.generator_id} (${route.connection.connection_scope})` : ""
  return `${route.route_id} · ${route.source_event_type} v${route.source_event_version}${source} → ${route.event_name} · ${counts}`
}

function formatWorker(worker: AppWorkerSummary, commandPrefix: string): string {
  const enabled = worker.enabled ? "" : " (stopped by user)"
  const hint = worker.failure === "app_lifecycle_disk_space"
    ? `: not enough free disk space on the host; app logs ${worker.installation_id} says how much to free`
    : ""
  const failure = worker.failure ? ` · ${worker.failure}${hint}` : ""
  const recovery = worker.phase === "quarantined"
    ? ` · explicit start required: ${commandPrefix} start ${JSON.stringify(worker.installation_id)}`
    : ""
  return `${worker.installation_id} · ${worker.phase.replace("_", " ")}${recovery}${enabled}${failure}`
}

function formatAutomation(automation: AppAutomationSummary): string {
  const scheduled = automation.scheduled ? " · scheduled" : ""
  return `${automation.automation_id} · ${automation.event_name} v${automation.event_version} → workflow ${automation.publication_id} (queue ${automation.queue_id}) · ${automation.status} · revision ${automation.revision}${scheduled}${automation.delivery_mode === "inject" ? " · inject" : ""}`
}

function formatInstallation(app: AppInstallationSummary): string {
  const version = app.active_release ? `version ${app.active_release.version}; digest ${app.active_release.package_digest}` : "no active release"
  const pending = app.pending_generation == null ? "" : `; pending generation ${app.pending_generation}`
  const paused = app.admission_paused ? "; admission paused" : ""
  const kept = app.data_kept ? "; data kept" : ""
  return `${app.installation_id} · ${app.app_id} · ${version}; generation ${app.generation}${pending}${paused}${kept}`
}

function uninstallFlags(args: string[]): { generation?: string; deleteData: boolean } | null {
  const flags: { generation?: string; deleteData: boolean } = { deleteData: false }
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === "--delete-data" && !flags.deleteData) flags.deleteData = true
    else if (arg === "--generation" && !flags.generation && /^\d{1,19}$/.test(args[index + 1] ?? "")) flags.generation = args[++index]!
    else return null
  }
  return flags
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
      : action === "inbox" ? "Not found: check the App installation and route."
      : action?.startsWith("connection") ? "Not found: check the App installation and the connection id."
      : action === "file revoke" ? "Not found: check the App installation and the file request id."
      : "App installation not found.",
    invalid_request: action === "inbox"
      ? "Invalid App request: the event must be one the App declares as incoming, and a payload must match its schema."
      : action === "connection grant"
        ? "Invalid App request: the App's signed manifest must declare this generator under capabilities.connections."
        : "Invalid App request.",
    busy: "App requests are busy. Try again shortly.",
    receipt_expired: "App receipt expired. This request was not re-executed; check the App state before issuing a new command.",
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

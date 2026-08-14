import type {
  EventConnection,
  EventConnectionAuthorization,
  EventConnectionPage,
  EventConnectionTestResult,
  EventGeneratorCatalogDetail,
  EventGeneratorCatalogPage,
  EventGeneratorResourcePage,
  WorkflowEventBindingDependency,
} from "./kernel-types.js"
import {
  browseEventGeneratorCategoryRequest,
  eventCatalogLandingRequest,
  getEventConnectionRequest,
  getEventGeneratorDetailRequest,
  installEventConnectionRequest,
  listEventConnectionDependenciesRequest,
  listEventConnectionResourcesRequest,
  listEventConnectionsRequest,
  observeEventConnectionAuthorizationRequest,
  reconnectEventConnectionRequest,
  refreshEventConnectionRequest,
  removeEventConnectionRequest,
  searchEventGeneratorCatalogRequest,
  testEventConnectionRequest,
} from "./ipc-event-publication-requests.js"
import type { ShellCommandResult } from "./shell-core.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export async function executeNotificationCommand(
  args: string[],
  client: ShellKernelClient,
): Promise<ShellCommandResult> {
  const [action = "home", ...rest] = args

  if (action === "home") {
    const [connectionsResponse, catalogResponse] = await Promise.all([
      client.send(listEventConnectionsRequest({ limit: 10 })),
      client.send(eventCatalogLandingRequest(10)),
    ])
    const connections = expectField<EventConnectionPage>(connectionsResponse, "EventConnectionsPage", "page")
    const catalog = expectField<EventGeneratorCatalogPage>(catalogResponse, "EventGeneratorCatalogPage", "page")
    return {
      ok: true,
      message: [
        "Installed connections",
        formatConnections(connections),
        "",
        "Discover services",
        formatCatalog(catalog),
      ].join("\n"),
      data: { connections, catalog },
    }
  }

  if (action === "catalog" || action === "search") {
    const parsed = parseOptions(rest)
    if (!parsed.ok) return parsed
    const request = action === "search" || parsed.query
      ? searchEventGeneratorCatalogRequest(parsed.query, parsed)
      : eventCatalogLandingRequest(parsed.limit)
    const page = expectField<EventGeneratorCatalogPage>(
      await client.send(request), "EventGeneratorCatalogPage", "page",
    )
    return { ok: true, message: formatCatalog(page), data: page }
  }

  if (action === "category") {
    const [category, ...options] = rest
    if (!category) return failure("usage: notifications category <category> [--cursor <cursor>] [--limit <n>]")
    const parsed = parseOptions(options)
    if (!parsed.ok) return parsed
    const page = expectField<EventGeneratorCatalogPage>(
      await client.send(browseEventGeneratorCategoryRequest(category, parsed)),
      "EventGeneratorCatalogPage", "page",
    )
    return { ok: true, message: formatCatalog(page), data: page }
  }

  if (action === "show") {
    const [generatorId, version] = rest
    if (!generatorId) return failure("usage: notifications show <generator-id> [version]")
    const detail = expectField<EventGeneratorCatalogDetail>(
      await client.send(getEventGeneratorDetailRequest(generatorId, version)),
      "EventGeneratorDetail", "detail",
    )
    return { ok: true, message: JSON.stringify(detail, null, 2), data: detail, format: "json" }
  }

  if (action === "connections" || action === "list") {
    const parsed = parseConnectionListOptions(rest)
    if (!parsed.ok) return parsed
    const page = expectField<EventConnectionPage>(
      await client.send(listEventConnectionsRequest(parsed)),
      "EventConnectionsPage", "page",
    )
    return { ok: true, message: formatConnections(page), data: page }
  }

  if (action === "connect" || action === "install") {
    const [generatorId, returnUrl] = rest
    if (!generatorId) return failure(`usage: notifications ${action} <generator-id> [return-url]`)
    const authorization = expectField<EventConnectionAuthorization>(
      await client.send(installEventConnectionRequest(generatorId, returnUrl)),
      "EventConnectionAuthorizationStarted", "authorization",
    )
    return { ok: true, message: formatAuthorization(authorization), data: authorization }
  }

  if (action === "authorization") {
    const authorizationId = rest[0]
    if (!authorizationId) return failure("usage: notifications authorization <authorization-id>")
    const payload = expectVariant<{ authorization: EventConnectionAuthorization; connection?: EventConnection | null }>(
      await client.send(observeEventConnectionAuthorizationRequest(authorizationId)),
      "EventConnectionAuthorizationObserved",
    )
    return {
      ok: true,
      message: [formatAuthorization(payload.authorization), payload.connection ? formatConnection(payload.connection) : null]
        .filter(Boolean).join("\n"),
      data: payload,
    }
  }

  if (action === "connection") return executeConnectionAction(rest, client)

  return failure("usage: notifications [catalog|search|category|show|connections|connect|authorization|connection]")
}

async function executeConnectionAction(
  args: string[],
  client: ShellKernelClient,
): Promise<ShellCommandResult> {
  const [action, connectionId, ...rest] = args
  if (!action || !connectionId) {
    return failure("usage: notifications connection show|resources|refresh|test|reconnect|dependencies|remove <connection-id>")
  }
  if (action === "show") {
    const connection = expectField<EventConnection>(
      await client.send(getEventConnectionRequest(connectionId)), "EventConnection", "connection",
    )
    return { ok: true, message: formatConnection(connection), data: connection }
  }
  if (action === "resources") {
    const parsed = parseOptions(rest)
    if (!parsed.ok) return parsed
    const page = expectField<EventGeneratorResourcePage>(
      await client.send(listEventConnectionResourcesRequest(connectionId, parsed)),
      "EventConnectionResourcesPage", "page",
    )
    const lines = page.resources.map((resource) => `${resource.connection_scope}  ${resource.name}  ${resource.kind}`)
    if (page.next_cursor) lines.push(`next cursor: ${page.next_cursor}`)
    return { ok: true, message: lines.join("\n") || "no matching provider resources found", data: page }
  }
  if (action === "refresh") {
    const connection = expectField<EventConnection>(
      await client.send(refreshEventConnectionRequest(connectionId)), "EventConnection", "connection",
    )
    return { ok: true, message: `refreshed\n${formatConnection(connection)}`, data: connection }
  }
  if (action === "test") {
    const result = expectField<EventConnectionTestResult>(
      await client.send(testEventConnectionRequest(connectionId, rest[0])),
      "EventConnectionTested", "result",
    )
    return {
      ok: result.accepted,
      message: result.accepted
        ? `test event accepted as ${result.occurrence_id}`
        : (result.message ?? "test event did not match an active workflow trigger"),
      data: result,
    }
  }
  if (action === "reconnect") {
    const authorization = expectField<EventConnectionAuthorization>(
      await client.send(reconnectEventConnectionRequest(connectionId, rest[0])),
      "EventConnectionAuthorizationStarted", "authorization",
    )
    return { ok: true, message: formatAuthorization(authorization), data: authorization }
  }
  if (action === "dependencies") {
    const payload = await dependencies(client, connectionId)
    return { ok: true, message: formatDependencies(payload.dependencies), data: payload }
  }
  if (action === "remove") {
    const dependencyPayload = await dependencies(client, connectionId)
    const confirmed = rest.includes("--confirm")
    if (!confirmed) {
      return {
        ok: true,
        message: [
          `Removing ${connectionId} will deactivate these workflow event bindings:`,
          formatDependencies(dependencyPayload.dependencies),
          `Repeat with /notifications connection remove ${connectionId} --confirm to continue.`,
        ].join("\n"),
        data: dependencyPayload,
      }
    }
    const payload = expectVariant<{ connection: EventConnection; deactivated_bindings: WorkflowEventBindingDependency[] }>(
      await client.send(removeEventConnectionRequest(connectionId, true)), "EventConnectionRemoved",
    )
    return { ok: true, message: `removed ${connectionId}`, data: payload }
  }
  return failure(`unknown notification connection action: ${action}`)
}

async function dependencies(client: ShellKernelClient, connectionId: string) {
  return expectVariant<{ connection_id: string; dependencies: WorkflowEventBindingDependency[] }>(
    await client.send(listEventConnectionDependenciesRequest(connectionId)),
    "EventConnectionDependencies",
  )
}

function parseOptions(args: string[]):
  | { ok: true; query: string; category?: string; verification?: string; cursor?: string; limit: number }
  | { ok: false; message: string } {
  const query: string[] = []
  let category: string | undefined
  let verification: string | undefined
  let cursor: string | undefined
  let limit = 20
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === "--category") category = args[++index]
    else if (arg === "--verification") verification = args[++index]
    else if (arg === "--cursor") cursor = args[++index]
    else if (arg === "--limit") {
      const value = Number.parseInt(args[++index] ?? "", 10)
      if (!Number.isInteger(value) || value < 1 || value > 50) return failure("--limit must be between 1 and 50")
      limit = value
    } else if (arg?.startsWith("--")) return failure(`unknown option: ${arg}`)
    else if (arg) query.push(arg)
  }
  return { ok: true, query: query.join(" "), ...(category ? { category } : {}), ...(verification ? { verification } : {}), ...(cursor ? { cursor } : {}), limit }
}

function parseConnectionListOptions(args: string[]):
  | { ok: true; generatorId?: string; cursor?: string; limit: number }
  | { ok: false; message: string } {
  const generatorId = args[0]?.startsWith("--") ? undefined : args[0]
  const parsed = parseOptions(generatorId ? args.slice(1) : args)
  if (!parsed.ok) return parsed
  if (parsed.query) return failure("usage: notifications connections [generator-id] [--cursor <cursor>] [--limit <n>]")
  return { ok: true, ...(generatorId ? { generatorId } : {}), ...(parsed.cursor ? { cursor: parsed.cursor } : {}), limit: parsed.limit }
}

function formatCatalog(page: EventGeneratorCatalogPage): string {
  const lines = page.services.map((service) => `${service.generator_id}@${service.version}  ${service.name}  ${service.availability}  ${service.verification}  ${service.summary}`)
  if (page.next_cursor) lines.push(`next cursor: ${page.next_cursor}`)
  if (page.stale) lines.push("catalog result is from stale cache")
  return lines.join("\n") || "no notification services found"
}

function formatConnections(page: EventConnectionPage): string {
  const lines = page.connections.map(formatConnection)
  if (page.next_cursor) lines.push(`next cursor: ${page.next_cursor}`)
  return lines.join("\n") || "no notification services installed"
}

function formatConnection(connection: EventConnection): string {
  const validated = connection.last_validated_at_ms ? new Date(connection.last_validated_at_ms).toISOString() : "never"
  const health = connection.last_successful_health_check_at_ms
    ? new Date(connection.last_successful_health_check_at_ms).toISOString()
    : "unsupported or never"
  const problem = connection.problem_message ? `  problem=${connection.problem_message}` : ""
  return `${connection.connection_id}  ${connection.lifecycle_state}  ${connection.generator_id}  triggers=${connection.attached_trigger_count}  last validated=${validated}  health=${health}${problem}`
}

function formatAuthorization(authorization: EventConnectionAuthorization): string {
  return [
    `authorization ${authorization.authorization_id}  ${authorization.status}  ${authorization.generator_id}`,
    authorization.connection_id ? `connection: ${authorization.connection_id}` : null,
    authorization.authorization_url ? `open: ${authorization.authorization_url}` : null,
    authorization.user_code ? `code: ${authorization.user_code}` : null,
  ].filter(Boolean).join("\n")
}

function formatDependencies(items: WorkflowEventBindingDependency[]): string {
  return items.length === 0
    ? "no workflow event bindings depend on this connection"
    : items.map((item) => `${item.status}  session=${item.session_id} trigger-owner=${item.publication_id} binding=${item.binding_id}`).join("\n")
}

function expectField<T>(response: Record<string, unknown>, variant: string, field: string): T {
  return expectVariant<Record<string, unknown>>(response, variant)[field] as T
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  const value = response[variant]
  if (!value || typeof value !== "object") throw new Error(`kernel returned an unexpected response; expected ${variant}`)
  return value as T
}

function failure(message: string): { ok: false; message: string } {
  return { ok: false, message }
}

import type {
  EventDeliveryStatus,
  EventConnectionAuthorization,
  EventConnectionPage,
  EventGeneratorCatalogDetail,
  EventGeneratorCatalogPage,
  EventGeneratorEventPage,
  EventGeneratorResourcePage,
} from "./kernel-types.js"
import {
  browseEventGeneratorCategoryRequest,
  browseEventGeneratorEventsRequest,
  eventCatalogLandingRequest,
  getEventDeliveryStatusRequest,
  getEventGeneratorDetailRequest,
  installEventConnectionRequest,
  listEventConnectionResourcesRequest,
  listEventConnectionsRequest,
  searchEventGeneratorCatalogRequest,
} from "./ipc-event-publication-requests.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export async function executeWorkflowEventPublicationCommand(
  args: string[],
  context: ShellContext,
  client: ShellKernelClient,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return failure("no current session; run `session new` or `session use <ref>` first")
  }
  const [action = "catalog", ...rest] = args

  if (action === "catalog" || action === "search") {
    const parsed = parseCatalogOptions(rest)
    if (!parsed.ok) return parsed
    const request = parsed.query
      ? searchEventGeneratorCatalogRequest(parsed.query, parsed)
      : eventCatalogLandingRequest(parsed.limit)
    const page = expectField<EventGeneratorCatalogPage>(
      await client.send(request),
      "EventGeneratorCatalogPage",
      "page",
    )
    return { ok: true, message: formatCatalogPage(page), data: page }
  }

  if (action === "category") {
    const [category, ...optionArgs] = rest
    if (!category) {
      return failure("usage: workflow trigger event category <category> [--cursor <cursor>] [--limit <n>]")
    }
    const parsed = parseCatalogOptions(optionArgs)
    if (!parsed.ok) return parsed
    const page = expectField<EventGeneratorCatalogPage>(
      await client.send(browseEventGeneratorCategoryRequest(category, parsed)),
      "EventGeneratorCatalogPage",
      "page",
    )
    return { ok: true, message: formatCatalogPage(page), data: page }
  }

  if (action === "show") {
    const [generatorId, version] = rest
    if (!generatorId) return failure("usage: workflow trigger event show <generator-id> [version]")
    const detail = expectField<EventGeneratorCatalogDetail>(
      await client.send(getEventGeneratorDetailRequest(generatorId, version)),
      "EventGeneratorDetail",
      "detail",
    )
    return { ok: true, message: JSON.stringify(detail, null, 2), data: detail, format: "json" }
  }

  if (action === "events") {
    const [generatorId, ...options] = rest
    if (!generatorId) {
      return failure("usage: workflow trigger event events <generator-id> [query] [--cursor <cursor>] [--limit <n>]")
    }
    const parsed = parseCatalogOptions(options)
    if (!parsed.ok) return parsed
    const page = expectField<EventGeneratorEventPage>(
      await client.send(browseEventGeneratorEventsRequest(generatorId, {
        ...(parsed.query ? { query: parsed.query } : {}),
        ...(parsed.cursor ? { cursor: parsed.cursor } : {}),
        limit: parsed.limit,
      })),
      "EventGeneratorEventsPage",
      "page",
    )
    const lines = page.events.map((event) =>
      `${event.event_type}@${event.version}  ${event.name}  ${event.description}`,
    )
    if (page.next_cursor) lines.push(`next cursor: ${page.next_cursor}`)
    return {
      ok: true,
      message: lines.length > 0 ? lines.join("\n") : "no matching events found",
      data: page,
    }
  }

  if (action === "authorize" || action === "install") {
    const [generatorId, returnUrl] = rest
    if (!generatorId) {
      return failure(`usage: workflow trigger event ${action} <generator-id> [return-url]`)
    }
    const authorization = expectField<EventConnectionAuthorization>(
      await client.send(installEventConnectionRequest(generatorId, returnUrl)),
      "EventConnectionAuthorizationStarted",
      "authorization",
    )
    const message = authorization.status === "ready" && authorization.connection_id
      ? `installed ${generatorId}; connection=${authorization.connection_id}`
      : [
          `authorization for ${generatorId} requires user action`,
          authorization.authorization_url ? `open: ${authorization.authorization_url}` : null,
          authorization.user_code ? `code: ${authorization.user_code}` : null,
          `authorization: ${authorization.authorization_id}`,
        ].filter(Boolean).join("\n")
    return { ok: true, message, data: authorization }
  }

  if (action === "connections") {
    const generatorId = rest[0]?.startsWith("--") ? undefined : rest[0]
    const options = generatorId ? rest.slice(1) : rest
    const parsed = parseCatalogOptions(options)
    if (!parsed.ok) return parsed
    if (parsed.query) return failure("usage: workflow trigger event connections [generator-id] [--cursor <cursor>] [--limit <n>]")
    const page = expectField<EventConnectionPage>(
      await client.send(listEventConnectionsRequest({
        ...(generatorId ? { generatorId } : {}),
        ...(parsed.cursor ? { cursor: parsed.cursor } : {}),
        limit: parsed.limit,
      })),
      "EventConnectionsPage",
      "page",
    )
    const lines = page.connections.map((connection) =>
      `${connection.connection_id}  ${connection.status}  ${connection.generator_id}`,
    )
    if (page.next_cursor) lines.push(`next cursor: ${page.next_cursor}`)
    return { ok: true, message: lines.join("\n") || "no installed notification connections", data: page }
  }

  if (action === "resources") {
    const [connectionId, ...options] = rest
    if (!connectionId) {
      return failure("usage: workflow trigger event resources <connection-id> [query] [--cursor <cursor>] [--limit <n>]")
    }
    const parsed = parseCatalogOptions(options)
    if (!parsed.ok) return parsed
    const page = expectField<EventGeneratorResourcePage>(
      await client.send(listEventConnectionResourcesRequest(connectionId, {
        ...(parsed.query ? { query: parsed.query } : {}),
        ...(parsed.cursor ? { cursor: parsed.cursor } : {}),
        limit: parsed.limit,
      })),
      "EventConnectionResourcesPage",
      "page",
    )
    const lines = page.resources.map((resource) =>
      `${resource.connection_scope}  ${resource.name}  ${resource.kind}${resource.filter ? `  filter ${JSON.stringify(resource.filter)}` : ""}`,
    )
    if (page.next_cursor) lines.push(`next cursor: ${page.next_cursor}`)
    return {
      ok: true,
      message: lines.length > 0 ? lines.join("\n") : "no matching provider resources found",
      data: page,
    }
  }

  if (action === "status") {
    const status = expectField<EventDeliveryStatus>(
      await client.send(getEventDeliveryStatusRequest()),
      "EventDeliveryStatus",
      "status",
    )
    return { ok: true, message: formatDeliveryStatus(status), data: status }
  }

  return failure("usage: workflow trigger event catalog|category|show|events|connections|install|resources|status")
}

function parseCatalogOptions(args: string[]):
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
      if (!Number.isInteger(value) || value < 1 || value > 50) {
        return failure("--limit must be between 1 and 50")
      }
      limit = value
    } else if (arg?.startsWith("--")) return failure(`unknown catalog option: ${arg}`)
    else if (arg) query.push(arg)
  }
  return {
    ok: true,
    query: query.join(" "),
    ...(category ? { category } : {}),
    ...(verification ? { verification } : {}),
    ...(cursor ? { cursor } : {}),
    limit,
  }
}

function formatCatalogPage(page: EventGeneratorCatalogPage): string {
  const lines = page.services.map((service) =>
    `${service.generator_id}@${service.version}  ${service.name}  ${service.availability}  ${service.verification}  ${service.summary}`,
  )
  if (page.next_cursor) lines.push(`next cursor: ${page.next_cursor}`)
  if (page.stale) lines.push("catalog result is from stale cache")
  return lines.length > 0 ? lines.join("\n") : "no event generators found"
}

function formatDeliveryStatus(status: EventDeliveryStatus): string {
  const connection = !status.configured
    ? "not configured"
    : status.connected
      ? "connected"
      : "disconnected"
  return `event delivery ${connection}; active App routes=${status.active_route_count}${status.last_error ? `; error=${status.last_error}` : ""}`
}

function failure(message: string): { ok: false; message: string } {
  return { ok: false, message }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) throw new Error(`unexpected response variant: expected ${variant}`)
  return response[variant] as T
}

function expectField<T>(
  response: Record<string, unknown>,
  variant: string,
  field: string,
): T {
  const payload = expectVariant<Record<string, unknown>>(response, variant)
  if (!(field in payload)) throw new Error(`unexpected response shape: ${variant}.${field}`)
  return payload[field] as T
}

// MP-08 / MP-09 / MP-10 / MP-11: one durable inbox, usable by any harness.
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"
import type { OutsideEvent, EventsBatch, EventSubscription, WorkflowNotificationSourceSummary } from "@chariox/kernel-client/kernel-types"

export const EVENT_VERBS = ["subscribe", "unsubscribe", "subscription", "sources", "list", "wait", "watch", "ack", "mcp"] as const
export type EventVerb = typeof EVENT_VERBS[number]
export type EventFlags = Map<string, string | true>

function integer(value: string | true | undefined, fallback: number, flag: string, max = Number.MAX_SAFE_INTEGER): number {
  if (value === undefined) return fallback
  const number = Number(value)
  if (typeof value !== "string" || !Number.isSafeInteger(number) || number < 0 || number > max) throw new CommandFailure("usage", `${flag} needs an integer from 0 to ${max}`)
  return number
}
function since(flags: EventFlags): { since?: number } {
  return flags.has("--since") ? { since: integer(flags.get("--since"), 0, "--since") } : {}
}
function target(flags: EventFlags): { kernel: string; session_id: string; agent_id?: string } {
  const kernel = flags.get("--kernel")
  const session = flags.get("--session")
  const agent = flags.get("--agent")
  if (typeof kernel !== "string" || !kernel || typeof session !== "string" || !session || (agent !== undefined && typeof agent !== "string")) throw new CommandFailure("usage", "events subscribe/unsubscribe needs --kernel K --session S [--agent A], or unsubscribe --all")
  return { kernel, session_id: session, ...(typeof agent === "string" ? { agent_id: agent } : {}) }
}
export function eventTimeoutSeconds(verb: EventVerb, flags: EventFlags): number {
  return verb === "wait" || verb === "watch" ? integer(flags.get("--timeout"), 540, "--timeout", 900) : 30
}
function batch(response: unknown): EventsBatch {
  if (!response || typeof response !== "object") throw new CommandFailure("events_response_invalid", "kernel returned an invalid event response; update Chariox")
  const wire = response as { EventsListed?: EventsBatch; EventsCursorExpired?: { oldest_cursor: number } }
  if (wire.EventsCursorExpired) throw new CommandFailure("cursor_expired", "event cursor expired; read the followed targets again, then resume from oldest_cursor - 1", { oldest_cursor: wire.EventsCursorExpired.oldest_cursor })
  if (!wire.EventsListed) throw new CommandFailure("events_response_invalid", "kernel returned no event batch; update Chariox")
  return wire.EventsListed
}
function ended(events: readonly OutsideEvent[]): boolean { return events.some(event => event.type === "subscription_ended") }

// Notification sources are followed by id; a person or model may name one by its display name instead.
export async function listSources(send: SendRequest): Promise<WorkflowNotificationSourceSummary[]> {
  const wire = await send({ ListNotificationSources: {} }) as { NotificationSources?: { sources: WorkflowNotificationSourceSummary[] } }
  if (!wire?.NotificationSources) throw new CommandFailure("events_response_invalid", "kernel returned no source list; update Chariox")
  return wire.NotificationSources.sources
}
// A source is named by id or, when no id matches, by display name or by the name `events sources` prints.
function resolveSource(sources: readonly { source_id: string; display_name: string; name?: string | undefined }[], reference: string, refused: string): string {
  if (sources.some(source => source.source_id === reference)) return reference
  const wanted = reference.toLowerCase()
  const byDisplay = sources.filter(source => source.display_name.toLowerCase() === wanted)
  const named = byDisplay.length ? byDisplay : sources.filter(source => source.name?.toLowerCase() === wanted)
  if (named.length > 1) throw new CommandFailure("usage", `several sources are named "${reference}"; name one by its source_id: ${named.map(source => source.source_id).join(", ")}`)
  if (!named[0]) throw new CommandFailure("events_refused", `source not available: "${reference}" ${refused}`)
  return named[0].source_id
}
// What the kernel answers to a subscription request: `{ subscription }`, printed as it is.
function subscriptionBody(response: unknown): { subscription: EventSubscription } {
  const wire = response as { EventSubscription?: { subscription: EventSubscription } } | undefined
  if (!wire?.EventSubscription?.subscription) throw new CommandFailure("events_response_invalid", "kernel returned no subscription; update Chariox")
  return wire.EventSubscription
}
export async function followSource(send: SendRequest, reference: string): Promise<{ subscription: EventSubscription }> {
  const source = resolveSource(await listSources(send), reference, "is not a source you can follow; run chariox events sources")
  return subscriptionBody(await send({ SubscribeEvents: { targets: [], sources: [source] } }))
}
// Removal reads what the grant follows, not what the kernel offers: a disabled or unavailable source is gone from the
// offer but stays followed until it is detached. `all` ends every followed source and leaves followed targets alone.
export async function unfollowSource(send: SendRequest, reference: string): Promise<{ subscription: EventSubscription }> {
  const current = subscriptionBody(await send({ GetEventSubscription: {} }))
  const followed = current.subscription.sources ?? []
  if (reference === "all") return followed.length ? subscriptionBody(await send({ UnsubscribeEvents: { sources: followed.map(source => source.source_id) } })) : current
  // A follow row carries the display name only; the offer knows the name `events sources` prints.
  const names = new Map((await listSources(send)).map(source => [source.source_id, source.name]))
  const source = resolveSource(followed.map(follow => ({ ...follow, name: names.get(follow.source_id) })), reference, "is not followed; run chariox events subscription")
  return subscriptionBody(await send({ UnsubscribeEvents: { sources: [source] } }))
}

export async function runEventsVerb(verb: EventVerb, flags: EventFlags, args: readonly string[], send: SendRequest, write: (value: unknown) => void): Promise<number> {
  if (verb === "sources") { write({ sources: await listSources(send) }); return 0 }
  if ((verb === "subscribe" || verb === "unsubscribe") && flags.has("--source")) {
    const source = flags.get("--source")
    if (typeof source !== "string" || !source || ["--kernel", "--session", "--agent", "--all"].some(flag => flags.has(flag))) throw new CommandFailure("usage", `events ${verb} --source takes a source id or name and no other target flag`)
    write(await (verb === "subscribe" ? followSource : unfollowSource)(send, source))
    return 0
  }
  if (verb === "subscribe" || verb === "unsubscribe") {
    const all = flags.has("--all")
    if (all && (verb !== "unsubscribe" || flags.has("--kernel") || flags.has("--session") || flags.has("--agent"))) throw new CommandFailure("usage", "unsubscribe uses targets or --all")
    const response = await send(verb === "subscribe" ? { SubscribeEvents: { targets: [target(flags)] } } : { UnsubscribeEvents: all ? { all: true } : { targets: [target(flags)] } })
    write((response as { EventSubscription: unknown }).EventSubscription)
    return 0
  }
  if (verb === "subscription") { const response = await send({ GetEventSubscription: {} }); write((response as { EventSubscription: unknown }).EventSubscription); return 0 }
  if (verb === "mcp") throw new CommandFailure("usage", "events mcp serves a harness over stdio")
  if (verb === "ack") { const response = await send({ AckEvents: { cursor: integer(args[0], 0, "cursor") } }); write((response as { EventsAcknowledged: unknown }).EventsAcknowledged); return 0 }
  const limit = integer(flags.get("--limit"), 1000, "--limit", 1000)
  if (limit === 0) throw new CommandFailure("usage", "--limit needs an integer from 1 to 1000")
  const timeout_ms = eventTimeoutSeconds(verb, flags) * 1000
  let cursor = since(flags)
  do {
    const response = batch(await send(verb === "list" ? { ListEvents: { ...cursor, limit } } : { WaitEvents: { ...cursor, limit, timeout_ms } }))
    if (verb === "watch") { for (const event of response.events) write(event) } else write(response)
    if (ended(response.events)) return 3
    if (verb !== "watch") return response.timed_out ? 5 : 0
    cursor = { since: response.next_cursor }
  } while (true)
}

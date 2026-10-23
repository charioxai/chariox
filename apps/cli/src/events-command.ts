// MP-08 / MP-09 / MP-10 / MP-11: one durable inbox, usable by any harness.
import { CommandFailure } from "./agent-command-table.js"
import type { SendRequest } from "./kernel-files.js"
import type { OutsideEvent, EventsBatch } from "@chariox/kernel-client/kernel-types"

export const EVENT_VERBS = ["subscribe", "unsubscribe", "subscription", "list", "wait", "watch", "ack"] as const
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

export async function runEventsVerb(verb: EventVerb, flags: EventFlags, args: readonly string[], send: SendRequest, write: (value: unknown) => void): Promise<number> {
  if (verb === "subscribe" || verb === "unsubscribe") {
    const all = flags.has("--all")
    if (all && (verb !== "unsubscribe" || flags.has("--kernel") || flags.has("--session") || flags.has("--agent"))) throw new CommandFailure("usage", "unsubscribe uses targets or --all")
    const response = await send(verb === "subscribe" ? { SubscribeEvents: { targets: [target(flags)] } } : { UnsubscribeEvents: all ? { all: true } : { targets: [target(flags)] } })
    write((response as { EventSubscription: unknown }).EventSubscription)
    return 0
  }
  if (verb === "subscription") { const response = await send({ GetEventSubscription: {} }); write((response as { EventSubscription: unknown }).EventSubscription); return 0 }
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

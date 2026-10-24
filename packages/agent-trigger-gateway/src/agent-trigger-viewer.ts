import { renderTranscriptHtml, renderTerminalEntriesByTurn, renderFreeformTranscriptEntry, type TerminalOutputEntry, type TerminalTurnState } from "@chariox/transcript-view"
import type { Conversation } from "./agent-trigger-client.js"
import { unconfirmedHtml } from "./agent-trigger-unconfirmed.js"
import { applyOutputRecords, reconcileSnapshot } from "./agent-trigger-live.js"
const main = document.querySelector<HTMLElement>("main[data-trigger-base]")!
const base = main.dataset.triggerBase!
const csrf = document.querySelector<HTMLMetaElement>('meta[name="csrf-token"]')?.content
const csrfHeaders: Record<string, string> = csrf ? { "x-csrf-token": csrf } : {}
const output = document.querySelector<HTMLElement>("#conversation")!
const status = document.querySelector<HTMLElement>("#status")!
const offlineHint = document.querySelector<HTMLElement>("#offline-hint")!
const notice = document.querySelector<HTMLElement>("#notice")!
const unconfirmed = document.querySelector<HTMLElement>("#unconfirmed")!
const form = document.querySelector<HTMLFormElement>("form")!
const input = document.querySelector<HTMLTextAreaElement>("#prompt")!
const send = document.querySelector<HTMLButtonElement>("#send")!
const more = document.querySelector<HTMLButtonElement>("#history-more")!
const live = { entries: new Map<string, TerminalOutputEntry>(), turns: new Map<string, TerminalTurnState>(), decoders: new Map<string, TextDecoder>() }
const { entries, turns } = live
const expanded = new Set<string>()
let nextCursor: Conversation["nextCursor"] = null
const escapeHtml = (value: string) => value.replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]!)
function render() {
  const atBottom = output.scrollHeight - output.scrollTop - output.clientHeight < 60
  const scrollTop = output.scrollTop
  output.innerHTML = renderTerminalEntriesByTurn([...entries.values()].sort((a,b) => (a.createdAtMs ?? 0) - (b.createdAtMs ?? 0) || Number(a.key) - Number(b.key)), {
    turns, expandedEntryKeys: expanded, turnActions: false, escapeHtml, renderEntry: entry => renderFreeformTranscriptEntry(entry, { expandedEntryKeys: expanded }),
    renderTranscriptHtml,
  })
  output.scrollTop = atBottom ? output.scrollHeight : scrollTop
}
function apply(value: Conversation, older = false) {
  reconcileSnapshot(live, value)
  if (older || turns.size === value.turns.length) nextCursor = value.nextCursor
  more.hidden = !nextCursor
  unconfirmed.innerHTML = unconfirmedHtml(value.unconfirmed)
  status.textContent = !value.enabled ? "trigger disabled" : !value.online ? "agent offline" : value.busy ? "working" : "idle"
  offlineHint.hidden = value.online
  input.disabled = send.disabled = !value.online || !value.enabled
  render()
}
apply(JSON.parse(document.querySelector("#initial-conversation")!.textContent!) as Conversation)
output.scrollTop = output.scrollHeight
output.addEventListener("click", event => {
  const target = (event.target as Element).closest<HTMLElement>("[data-freeform-turn-toggle], [data-freeform-blob-toggle]")
  if (!target) return
  if (target.hasAttribute("data-freeform-turn-toggle")) {
    const turnId = target.closest<HTMLElement>("[data-turn-key]")?.dataset.turnKey
    if (turnId && turns.has(turnId)) { const turn = turns.get(turnId)!; turn.expanded = !turn.expanded }
  } else {
    const key = target.closest<HTMLElement>("[data-entry-key]")?.dataset.entryKey
    if (key) { if (expanded.has(key)) expanded.delete(key); else expanded.add(key) }
  }
  render()
})
unconfirmed.addEventListener("click", async event => {
  const button = (event.target as Element).closest<HTMLButtonElement>("[data-resend]")
  if (!button) return
  button.disabled = true
  try {
    const response = await fetch(`${base}resend`, { method: "POST", headers: { "Content-Type": "application/json", Accept: "application/json", ...csrfHeaders }, body: JSON.stringify({ prompt_id: button.dataset.resend }) })
    if (!response.ok) throw Error(((await response.json()) as { error?: string }).error ?? "The message was not sent again.")
    notice.textContent = "Message sent again."
  } catch (error) { notice.textContent = error instanceof Error ? error.message : String(error); button.disabled = false }
})
more.addEventListener("click", async () => {
  if (!nextCursor) return
  more.disabled = true
  try { const response = await fetch(`${base}history?before=${nextCursor.before_sequence}`); if (!response.ok) throw Error("History is unavailable; retry shortly."); apply(await response.json(), true) }
  catch (error) { notice.textContent = String(error) }
  finally { more.disabled = false }
})
form.addEventListener("submit", async event => {
  event.preventDefault()
  if (!input.value.trim()) return
  send.disabled = true
  try {
    const response = await fetch(base, { method: "POST", headers: { "Content-Type": "application/json", Accept: "application/json", ...csrfHeaders }, body: JSON.stringify({ prompt: input.value }) })
    const result = await response.json() as { error?: string; outcome?: string }
    if (!response.ok) throw Error(result.error ?? "The message was not accepted.")
    input.value = ""; notice.textContent = `Message ${result.outcome ?? "accepted"}.`
  } catch (error) { notice.textContent = error instanceof Error ? error.message : String(error) }
  finally { send.disabled = input.disabled }
})
const events = new EventSource(base + "events")
events.addEventListener("conversation", event => apply(JSON.parse((event as MessageEvent).data)))
events.addEventListener("output", event => {
  applyOutputRecords(live, JSON.parse((event as MessageEvent).data) as Array<Record<string, unknown>>)
  status.textContent = "working"
  render()
})
events.addEventListener("offline", () => { status.textContent = "agent offline"; input.disabled = send.disabled = true })
events.onerror = () => { status.textContent = "Reconnecting…" }

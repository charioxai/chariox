import { renderTranscriptHtml, renderTerminalEntriesByTurn, renderFreeformTranscriptEntry, type TerminalOutputEntry, type TerminalTurnState } from "@chariox/transcript-view"
import type { Conversation } from "./agent-trigger-client.js"
const main = document.querySelector<HTMLElement>("main[data-trigger-base]")!
const base = main.dataset.triggerBase!
const output = document.querySelector<HTMLElement>("#conversation")!
const status = document.querySelector<HTMLElement>("#status")!
const notice = document.querySelector<HTMLElement>("#notice")!
const form = document.querySelector<HTMLFormElement>("form")!
const input = document.querySelector<HTMLTextAreaElement>("#prompt")!
const send = document.querySelector<HTMLButtonElement>("#send")!
const more = document.querySelector<HTMLButtonElement>("#history-more")!
const decoders = new Map<string, TextDecoder>()
const entries = new Map<string, TerminalOutputEntry>(), turns = new Map<string, TerminalTurnState>(), expanded = new Set<string>()
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
  const ids = new Set(value.turns.map(turn => turn.id))
  const promptIds = new Set(value.turns.map(turn => turn.kernelPromptId).filter(Boolean))
  for (const [id, turn] of turns) {
    if (id.startsWith("live:") && turn.kernelPromptId && promptIds.has(turn.kernelPromptId)) { ids.add(id); turns.delete(id) }
  }
  for (const [key,entry] of entries) if (entry.turnId && ids.has(entry.turnId)) entries.delete(key)
  for (const [key] of decoders) if ([...promptIds].some(id => key.startsWith(`live:${id}:`))) decoders.delete(key)
  for (const entry of value.entries) entries.set(entry.key,entry)
  for (const turn of value.turns) turns.set(turn.id, { ...turn, expanded: turns.get(turn.id)?.expanded ?? turn.expanded })
  if (older || turns.size === value.turns.length) nextCursor = value.nextCursor
  more.hidden = !nextCursor
  status.textContent = !value.enabled ? "trigger disabled" : !value.online ? "agent offline" : value.busy ? "working" : "idle"
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
    const response = await fetch(base, { method: "POST", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: JSON.stringify({ prompt: input.value }) })
    const result = await response.json() as { error?: string; outcome?: string }
    if (!response.ok) throw Error(result.error ?? "The message was not accepted.")
    input.value = ""; notice.textContent = `Message ${result.outcome ?? "accepted"}.`
  } catch (error) { notice.textContent = error instanceof Error ? error.message : String(error) }
  finally { send.disabled = input.disabled }
})
const events = new EventSource(base + "events")
events.addEventListener("conversation", event => apply(JSON.parse((event as MessageEvent).data)))
events.addEventListener("output", event => {
  const records = JSON.parse((event as MessageEvent).data) as Array<Record<string, unknown>>
  const roles = { prompt_echo: "user", provider_output: "assistant", provider_reasoning: "reasoning", provider_tool: "tool", provider_error: "error", provider_status: "status" } as const
  for (const record of records) {
    const role = roles[record.kind as keyof typeof roles]
    if (!role || !Array.isArray(record.bytes) || typeof record.agent_id !== "string") continue
    const promptId = typeof record.prompt_id === "string" ? record.prompt_id : "untracked"
    const turn = [...turns.values()].find(turn => turn.kernelPromptId === promptId) ?? {
      id: `live:${promptId}`, agentId: record.agent_id, kernelPromptId: promptId, startedAtMs: Number(record.timestamp_ms),
      lastEntryAtMs: Number(record.timestamp_ms), kernelObservedActive: true, expanded: true,
    }
    turns.set(turn.id,turn)
    const merge = typeof record.merge_key === "string" ? record.merge_key : String(record.record_id ?? record.sequence ?? Date.now())
    const key = `live:${promptId}:${role}:${merge}`
    const previous = entries.get(key)
    const decoder = decoders.get(key) ?? new TextDecoder()
    decoders.set(key, decoder)
    const text = decoder.decode(new Uint8Array(record.bytes as number[]), { stream: true })
    entries.set(key, { key, turnId: turn.id, agentId: record.agent_id, role,
      createdAtMs: Number(record.timestamp_ms), promptId, text: role === "assistant" || role === "reasoning" ? (previous?.text ?? "") + text : text })
  }
  status.textContent = "working"
  render()
})
events.addEventListener("offline", () => { status.textContent = "agent offline"; input.disabled = send.disabled = true })
events.onerror = () => { status.textContent = "Reconnecting…" }

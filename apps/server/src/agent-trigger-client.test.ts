import test from "node:test"
import assert from "node:assert/strict"
import type { SessionHistoryPageEntry, SessionHistoryOutlineTurn } from "@chariox/kernel-client/kernel-types"
import { renderTerminalEntriesByTurn, type TerminalTurnState } from "@chariox/transcript-view"
import { AgentTriggerClient } from "@chariox/agent-trigger-gateway"
import { buildAgentTriggerServer } from "@chariox/agent-trigger-gateway/server"
import { agentTriggerHistoryEntries } from "./agent-trigger-client.js"
function row(entry_index: number, kind: "user_prompt" | "provider_output", text: string): SessionHistoryPageEntry {
  return { entry_index, fragment_start: 0, fragment_end: text.length, total_chars: text.length, entry: { kind, text, timestamp_ms: entry_index } }
}
test("MP-08/MP-10/MP-11: pre-trigger prompts and summaries retain distinct canonical entry identities", () => {
  const turns: SessionHistoryOutlineTurn[] = [1, 3].map(n => ({ turn_id: `turn-${n}`, prompt_id: `prompt-${n}`, started_at_ms: n,
    completed_at_ms: n + 1, lifecycle: "completed", user_prompt: row(n, "user_prompt", `Question ${n}`),
    entries: [], summary: row(n + 1, "provider_output", `Answer ${n}`), blobs: [] }))
  const entries = agentTriggerHistoryEntries("agent", turns, new Map())
  assert.deepEqual(entries.map(e => [e.key, e.role, e.text]), [["1", "user", "Question 1"], ["2", "assistant", "Answer 1"], ["3", "user", "Question 3"], ["4", "assistant", "Answer 3"]])
  assert.equal(entries.filter(e => e.historyTurnSummary).length, 2)
})
test("MP-08/MP-10/MP-11: a final answer past the inline limit is rebuilt from its blob rows", () => {
  const first = "a".repeat(10_000), second = "b".repeat(10_000)
  const inline = (first + second).slice(0, 16_384)
  const turn: SessionHistoryOutlineTurn = { turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 9, lifecycle: "completed",
    user_prompt: row(1, "user_prompt", "Question"), entries: [],
    summary: { ...row(2, "provider_output", inline), fragment_end: inline.length, total_chars: 20_000 },
    blobs: [{ blob_id: "2:3", title: "assistant output", summary: "", preview: null } as unknown as SessionHistoryOutlineTurn["blobs"][number]] }
  const entries = agentTriggerHistoryEntries("agent", [turn], new Map([["2:3", [row(2, "provider_output", first), row(3, "provider_output", second)]]]))
  assert.deepEqual(entries.map(e => [e.key, e.role]), [["1", "user"], ["2", "assistant"]])
  assert.equal(entries[1]!.text, first + second)
  assert.equal(entries[1]!.historyTurnSummary, true)
})
test("MP-08/MP-10/MP-11: commentary before a tool call stays beside the final answer", () => {
  const turn: SessionHistoryOutlineTurn = { turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 9, lifecycle: "completed",
    user_prompt: row(1, "user_prompt", "Question"), entries: [row(2, "provider_output", "Let me look."), row(3, "provider_output", "(tool)")],
    summary: row(4, "provider_output", "Done."), blobs: [] }
  const entries = agentTriggerHistoryEntries("agent", [turn], new Map())
  assert.deepEqual(entries.map(e => e.text), ["Question", "Let me look.", "(tool)", "Done."])
  assert.deepEqual(entries.filter(e => e.historyTurnSummary).map(e => e.text), ["Done."])
})
test("MP-08/MP-10/MP-11: a page that cannot undo or fork a turn renders no Undo or Fork", () => {
  const entries = agentTriggerHistoryEntries("agent", [{ turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 2, lifecycle: "completed",
    user_prompt: row(1, "user_prompt", "Question"), entries: [], summary: row(2, "provider_output", "Answer"), blobs: [] }], new Map())
  const turn: TerminalTurnState = { id: "turn-1", agentId: "agent", startedAtMs: 1, lastEntryAtMs: 2, completedAtMs: 2, kernelObservedActive: false, kernelSettled: true, kernelSettlementStatus: "completed", expanded: false }
  const options = { turns: new Map([[turn.id, turn]]), escapeHtml: (v: string) => v, renderTranscriptHtml: (v: string) => v, renderEntry: (e: { text: string }) => e.text }
  assert.match(renderTerminalEntriesByTurn(entries, options), /data-freeform-turn-action="fork"/)
  assert.doesNotMatch(renderTerminalEntriesByTurn(entries, { ...options, turnActions: false }), /data-freeform-turn-action/)
})
test("MP-08/MP-10: a trigger prompt in history keeps the origin the kernel labelled it with", () => {
  const prompt = row(1, "user_prompt", "Check the build.")
  prompt.entry.prompt_origin = "schedule"
  const entries = agentTriggerHistoryEntries("agent", [{ turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 2, lifecycle: "completed",
    user_prompt: prompt, entries: [], summary: row(2, "provider_output", "Done."), blobs: [] }], new Map())
  assert.deepEqual(entries.map(entry => entry.promptOrigin ?? null), ["schedule", null])
})


test("separate HTTP viewers keep separate kernel attachments", async () => {
  const { AgentTriggerClient: GatewayClient } = await import("@chariox/agent-trigger-gateway")
  const ids: string[] = []
  const kernel = {
    async send<T>(request: unknown): Promise<T> {
      ids.push((request as {AttachToSession: {client_id: string}}).AttachToSession.client_id)
      return {SessionAttached: {attachment: {id: String(ids.length)}}} as T
    },
    async subscribeToKernelEvents() {}, onKernelEvent() { return () => {} }, async close() {},
  }
  await new GatewayClient("session", "trigger", kernel).subscribe()
  await new GatewayClient("session", "trigger", kernel).subscribe()
  assert.equal(new Set(ids).size, 2)
})
test("MP-08/MP-10: a prompt past the inline limit is shown whole, read from its own history blob", async () => {
  const text = "p".repeat(20_000)
  const cut = text.slice(0, 16_384)
  const prompt = { ...row(1, "user_prompt", cut), fragment_end: cut.length, total_chars: text.length }
  const turn: SessionHistoryOutlineTurn = { turn_id: "turn-1", prompt_id: "prompt-1", started_at_ms: 1, completed_at_ms: 3, lifecycle: "completed",
    user_prompt: prompt, entries: [], summary: row(2, "provider_output", "Done."), blobs: [] }
  const requested: string[][] = []
  const kernel = { send: async (request: Record<string, any>) => {
    if (request.GetSessionHistoryBlobContents) {
      requested.push(request.GetSessionHistoryBlobContents.blob_ids)
      return { SessionHistoryBlobContents: { blobs: [{ blob_id: "history:1:1", entries: [row(1, "user_prompt", text)] }] } }
    }
    return { AgentTriggerTranscript: { trigger: { agent_id: "agent", enabled: true }, agent_alias: "a", online: true, busy: false, deliveries: [],
      history: { SessionHistoryOutline: { agents: [{ agent_id: "agent", turns: [turn], next_cursor: null }] } } } }
  } }
  const conversation = await new AgentTriggerClient("session", "trigger", kernel as never).conversation()
  assert.deepEqual(requested, [["history:1:1"]])
  assert.equal(conversation.entries[0]!.text, text)
})

test("#1263 P2 a workspace picture is read through the kernel from the trigger agent's own worktree, and nothing outside it", async () => {
  const png = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.from("chart")])
  const reads: Array<Record<string, unknown>> = []
  const kernel = {
    async send<T>(request: Record<string, unknown>): Promise<T> {
      if ("ReadAgentTriggerTranscript" in request) return { AgentTriggerTranscript: { trigger: { agent_id: "agent-1", enabled: true, source: { kind: "http", ingress_id: "chart" } }, agent_alias: "a", online: true, busy: false, history: { SessionHistoryOutline: { agents: [] } }, deliveries: [] } } as T
      if ("ReadAgentTriggerHttpOptions" in request) return { AgentTriggerHttpOptionsRead: { enabled: true, options: { route: "/", methods: ["GET", "POST"], parser: { kind: "json" }, mode: "async" } } } as T
      if ("GetSessionState" in request) return { SessionState: { session: { workspace_id: "/work/default", worktree_id: "/work/default-wt", agents: [{ id: "other", workspace_id: "/elsewhere", worktree_id: null }, { id: "agent-1", workspace_id: "/work/repo", worktree_id: "/work/repo-wt" }] } } } as T
      reads.push(request.GetWorkspaceFileContent as Record<string, unknown>)
      return { WorkspaceFileContent: { content: { content_base64: png.toString("base64"), truncated: false, size_bytes: png.length } } } as T
    },
    async subscribeToKernelEvents() {}, onKernelEvent() { return () => {} }, async close() {},
  }
  const client = new AgentTriggerClient("session", "trigger", kernel)
  const image = await client.workspaceImage("./charts/cost.png")
  assert.equal(image?.mime, "image/png")
  assert.deepEqual(image?.bytes, png)
  assert.equal(await client.workspaceImage("/work/repo-wt/charts/cost.png").then(found => found?.mime), "image/png")
  assert.deepEqual(reads.map(read => [read.workspace_id, read.worktree_id, read.path]), [["/work/repo", "/work/repo-wt", "charts/cost.png"], ["/work/repo", "/work/repo-wt", "charts/cost.png"]])
  for (const outside of ["../secret.png", "/etc/passwd", "charts//x.png"]) assert.equal(await client.workspaceImage(outside), null, outside)
  assert.equal(reads.length, 2, "nothing outside the worktree is asked for")
  const app = await buildAgentTriggerServer(client, "127.0.0.1")
  try {
    const served = await app.inject({ method: "GET", url: "/chart/workspace-image?path=charts%2Fcost.png" })
    assert.equal(served.statusCode, 200)
    assert.equal(served.headers["content-type"], "image/png")
    assert.deepEqual(served.rawPayload, png)
  } finally { await app.close() }
})

test("#1263 P2 workspace pictures use the session defaults for absent agent overrides", async () => {
  for (const overrides of [{}, { workspace_id: "/work/agent" }, { worktree_id: "/work/agent-wt" }]) {
    let read: Record<string, unknown> | undefined
    const client = new AgentTriggerClient("session", "trigger", {
      async send<T>(request: Record<string, unknown>): Promise<T> {
        if ("ReadAgentTriggerTranscript" in request) return { AgentTriggerTranscript: { trigger: { agent_id: "agent" } } } as T
        if ("GetSessionState" in request) return { SessionState: { session: { workspace_id: "/work/default", worktree_id: "/work/default-wt", agents: [{ id: "agent", workspace_id: null, worktree_id: null, ...overrides }] } } } as T
        read = request.GetWorkspaceFileContent as Record<string, unknown>
        return { WorkspaceFileContent: { content: null } } as T
      },
      async subscribeToKernelEvents() {}, onKernelEvent() { return () => {} }, async close() {},
    })
    await client.workspaceImage("charts/cost.png")
    assert.deepEqual(read && [read.workspace_id, read.worktree_id, read.path], [overrides.workspace_id ?? "/work/default", overrides.worktree_id ?? "/work/default-wt", "charts/cost.png"])
  }
})

test("#1263 P2 workspace image reads share redraws, invalidate on completion and discard late old blobs", async t => {
  const { AgentTriggerImages } = await import("@chariox/agent-trigger-gateway/images")
  const revoked: string[] = []
  t.mock.method(URL, "revokeObjectURL", (src: string) => { revoked.push(src) })
  type Image = Awaited<ReturnType<InstanceType<typeof AgentTriggerImages>["load"]>>
  const pending: Array<(image: Image) => void> = []
  const images = new AgentTriggerImages(() => new Promise(resolve => { pending.push(resolve) }))
  const element = {} as HTMLElement
  const first = images.load(element, "cost.png")
  assert.ok(first === images.load(element, "cost.png"), "concurrent redraws share the read")
  const turn = { id: "turn", agentId: "agent", startedAtMs: 1, lastEntryAtMs: 1, kernelObservedActive: true, expanded: false }
  images.refresh([turn])
  assert.ok(first === images.load(element, "cost.png"), "an open turn does not cause another read")
  images.refresh([{ ...turn, completedAtMs: 2, kernelSettled: true }])
  const second = images.load(element, "cost.png")
  assert.ok(second !== first, "completion starts a new read")
  pending[0]!({ kind: "loaded", src: "blob:old", sizeBytes: 10 })
  assert.equal((await first).kind, "unavailable")
  pending[1]!({ kind: "loaded", src: "blob:new", sizeBytes: 20 })
  assert.equal((await second).kind, "loaded")
  images.refresh([{ ...turn, completedAtMs: 2, kernelSettled: true }])
  assert.ok(second === images.load(element, "cost.png"), "the unchanged completion reuses its blob")
  images.clear()
  assert.deepEqual(revoked, ["blob:old", "blob:new"])
})

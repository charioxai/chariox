// MP-08 / MP-10 / MP-11: actual session application must show the accepted row before terminal echo.
import assert from "node:assert/strict"
import test from "node:test"
import { createStore } from "solid-js/store"
import { createCliQueuedPromptController } from "./cli-queued-prompt-controller.js"
import type { RuntimeSession, TranscriptEntry } from "./cli-types.js"

function harness() {
  let visible: TranscriptEntry[] = []
  const [panes, setPanes] = createStore<Record<string, TranscriptEntry[]>>({ "agent-1": [] })
  const controller = createCliQueuedPromptController({
    agentPaneEntries: () => panes,
    setAgentPaneEntries: setPanes,
    setAgentPanePreviews: () => {},
    visibleTranscriptAgentId: () => "agent-1",
    transcriptEntryProjectionController: { renderableEntries: () => visible },
    replaceTranscriptEntries: (next: TranscriptEntry[]) => { visible = next },
    renderAgentInteractions: () => {}, applyResponseLayout: () => {},
    appendSteeredPrompt: () => {}, applySessionState: () => {}, flashFooter: () => {},
    formatError: String, updateSessionChrome: () => {},
  })
  return { controller, panes, visible: () => visible }
}
function session(): RuntimeSession {
  return { id: "session-1", agents: [{ id: "agent-1" }],
    prompt_states: { "agent-1": { active_prompt: { id: "prompt-1", target_agent_id: "agent-1", source_attachment_id: "peer", prompt: "Read public source", status: "Running", prompt_origin: "chariox", created_at_ms: 1000 }, queued_prompts: [] } },
    agent_activity: { "agent-1": { status: "working", prompt_status: "running", busy: true, active_prompt_count: 1, queued_prompt_count: 0, active_turn: { prompt_id: "prompt-1", provider_run_id: "run-1", status: "running", phase: "streaming", started_at_ms: 1000 } } },
  } as unknown as RuntimeSession
}
test("accepted snapshot paints the user row without a terminal echo and repeats do not duplicate it", () => {
  const h = harness()
  h.controller.syncQueuedPromptsForSession(session())
  assert.deepEqual(h.visible().map(entry => [entry.role, entry.text, entry.promptId]), [["user", "Read public source", "prompt-1"]])
  assert.equal(h.panes["agent-1"]?.length, 1)
  h.controller.syncQueuedPromptsForSession(session())
  assert.equal(h.visible().length, 1)
  assert.equal(h.panes["agent-1"]?.length, 1)
})
test("authoritative idle does not revive a stale prompt, and unknown agents are ignored", () => {
  const h = harness(), state = session()
  state.agent_activity!["agent-1"] = { status: "idle", prompt_status: "none", busy: false, active_prompt_count: 0, queued_prompt_count: 0 } as any
  h.controller.syncQueuedPromptsForSession(state)
  assert.equal(h.visible().length, 0)
  const unknown = session(); unknown.agents = []
  h.controller.syncQueuedPromptsForSession(unknown)
  assert.equal(h.visible().length, 0)
})

test("mismatched active-turn identity cannot revive a stale user row", () => {
  const h = harness(), state = session()
  state.agent_activity!["agent-1"]!.active_turn!.prompt_id = "other-prompt"
  h.controller.syncQueuedPromptsForSession(state)
  assert.equal(h.visible().length, 0)
})
test("snapshot prompt attachment metadata is kept without inline file contents", () => {
  const h = harness(), state = session()
  state.prompt_states!["agent-1"]!.active_prompt!.attachments = [{ url: "chariox-terminal://public.png", mime: "image/png", filename: "public.png", contents_base64: "aGk=" }]
  h.controller.syncQueuedPromptsForSession(state)
  assert.deepEqual(h.visible()[0]?.attachments, [{ url: "chariox-terminal://public.png", mime: "image/png", filename: "public.png" }])
})

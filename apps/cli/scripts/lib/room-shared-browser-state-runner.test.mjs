import assert from "node:assert/strict"
import test from "node:test"

import { assertRoomSharedBrowserStateText } from "./room-shared-browser-state-fixture.mjs"
import {
  assertRoomRendererSandboxText,
  assertRoomSharedBrowserStatePhaseEvidence,
  runRoomSharedBrowserStatePhase,
  roomSharedBrowserStatePrompt,
} from "./room-shared-browser-state-runner.mjs"

const markers = {
  cookie: "PASS", auth: "PASS", localStorage: "PASS", indexedDB: "PASS",
  cacheStorage: "PASS", serviceWorker: "PASS",
}
const generation = "room-state-test-generation"
const fixtureUrl = "http://127.0.0.1:4321/click"

test("Browser state evidence requires every persisted API marker and authenticated fixture", () => {
  assert.deepEqual(assertRoomSharedBrowserStateText(stateText(markers)), markers)
  for (const field of Object.keys(markers)) {
    const changed = { ...markers, [field]: "FAIL" }
    assert.throws(() => assertRoomSharedBrowserStateText(stateText(changed)), new RegExp(`${field} marker`))
    const missing = { ...markers }
    delete missing[field]
    assert.throws(() => assertRoomSharedBrowserStateText(stateText(missing)), /unknown or malformed|omitted/)
  }
  assert.throws(() => assertRoomSharedBrowserStateText(`${stateText(markers)} cookie=PASS`), /duplicated the cookie marker/)
  assert.throws(() => assertRoomSharedBrowserStateText(`${stateText(markers)} cookie=FAIL`), /duplicated the cookie marker/)
})

test("renderer sandbox evidence requires active namespace and Seccomp-BPF isolation", () => {
  const active = "Namespace sandbox: Yes\nSeccomp-BPF sandbox: Yes"
  assert.equal(assertRoomRendererSandboxText(active), true)
  assert.throws(() => assertRoomRendererSandboxText("Namespace sandbox: Yes"), /Seccomp-BPF/)
  assert.throws(() => assertRoomRendererSandboxText("Seccomp-BPF sandbox: Yes"), /namespace/)
  assert.throws(() => assertRoomRendererSandboxText("Namespace sandbox: No\nSeccomp-BPF sandbox: No"), /namespace/)
})

test("state phase requires same-thread Browser, sandbox, command, and visible program evidence", () => {
  const evidence = assertRoomSharedBrowserStatePhaseEvidence({
    records: records("before-save"), phase: "before-save", fixtureUrl, generation,
    expectedProviderSessionId: "provider-thread-1", actualProviderSessionId: "provider-thread-1",
  })
  assert.deepEqual(evidence.browserMarkers, markers)
  assert.deepEqual(evidence.rendererSandbox, { namespace: "active", seccompBpf: "active" })
  assert.deepEqual(evidence.graphicalProgram, { commandCompleted: true, visible: true, retained: false })
  const earlyWait = records("before-save").map(tool => tool.name === "slice_browser_wait_for_text"
    ? { ...tool, input: { text: "ROOM_BROWSER_STATE" } } : tool)
  assert.throws(() => assertRoomSharedBrowserStatePhaseEvidence({
    records: earlyWait, phase: "before-save", fixtureUrl, generation,
    expectedProviderSessionId: "provider-thread-1", actualProviderSessionId: "provider-thread-1",
  }), /must wait for the state fixture/)
  assert.throws(() => assertRoomSharedBrowserStatePhaseEvidence({
    records: records("before-save"), phase: "before-save", fixtureUrl, generation,
    expectedProviderSessionId: "provider-thread-1", actualProviderSessionId: "different-thread",
  }), /different provider thread/)
})

test("state runner submits an exact public agent turn and validates its recorded tools", async () => {
  const events = []
  const turn = {
    turn_id: "provider-state-turn", prompt_id: "provider-state-prompt", lifecycle: "completed",
    external_provider_session_id: "provider-thread-1",
    entries: records("before-save").map((tool, entry_index) => ({
      entry_index,
      entry: { kind: "provider_tool", text: JSON.stringify({
        tool: tool.name, status: tool.status, input: tool.input, output: tool.output,
      }) },
    })),
  }
  const client = {
    async send(request) {
      events.push(request)
      if (request.kind === "attach") return { SessionAttached: { attachment: { id: "agent-attachment" } } }
      if (request.kind === "submit") {
        assert.equal(request.agentId, "agent-1")
        assert.match(request.prompt, /chrome:\/\/sandbox/)
        return { PromptSubmitted: { outcome: { Started: { prompt: { id: turn.prompt_id } } } } }
      }
      if (request.kind === "detach") return { SessionDetached: {} }
      if (request.kind === "outline") return { SessionHistoryOutline: { agents: [{ agent_id: "agent-1", turns: [turn] }] } }
      if (request.kind === "state") return { SessionState: { session: { agents: [{ id: "agent-1", is_processing: false }] } } }
      throw new Error(`unexpected request ${request.kind}`)
    },
  }
  const requests = {
    attachToSessionRequest: () => ({ kind: "attach" }),
    submitPromptRequest: (sessionId, attachmentId, agentId, prompt) => ({
      kind: "submit", sessionId, attachmentId, agentId, prompt,
    }),
    detachFromSessionRequest: () => ({ kind: "detach" }),
    getSessionHistoryOutlineRequest: () => ({ kind: "outline" }),
    getSessionStateRequest: () => ({ kind: "state" }),
  }
  const result = await runRoomSharedBrowserStatePhase({
    client, requests, sessionId: "room-1", providerAgentId: "agent-1",
    providerSessionId: "provider-thread-1", fixtureUrl, generation, phase: "before-save",
    withTimeout: async promise => promise,
    waitFor: async predicate => {
      const value = await predicate()
      assert.notEqual(value, false)
      return value
    },
  })
  assert.equal(result.turnId, turn.turn_id)
  assert.equal(result.providerSessionId, "provider-thread-1")
  assert.equal(result.browserMarkers.serviceWorker, "PASS")
  assert.deepEqual(events.map(event => event.kind), ["attach", "submit", "detach", "outline", "state", "outline"])
})

test("post-restore evidence rejects a missing file, recreated program, or unobserved graphic", () => {
  const valid = records("after-restore")
  assertRoomSharedBrowserStatePhaseEvidence({
    records: valid, phase: "after-restore", fixtureUrl, generation,
    expectedProviderSessionId: "provider-thread-1", actualProviderSessionId: "provider-thread-1",
  })
  const noFile = valid.map(tool => tool.name === "exec_command"
    ? { ...tool, output: "" } : tool)
  assert.throws(() => assertRoomSharedBrowserStatePhaseEvidence({
    records: noFile, phase: "after-restore", fixtureUrl, generation,
    expectedProviderSessionId: "provider-thread-1", actualProviderSessionId: "provider-thread-1",
  }), /omitted the installed-program marker/)
  const recreated = valid.map(tool => tool.name === "exec_command"
    ? { ...tool, input: `${tool.input}\ncat > "$HOME/.local/bin/chariox-room-state-${generation}"` }
    : tool)
  assert.throws(() => assertRoomSharedBrowserStatePhaseEvidence({
    records: recreated, phase: "after-restore", fixtureUrl, generation,
    expectedProviderSessionId: "provider-thread-1", actualProviderSessionId: "provider-thread-1",
  }), /without recreating/)
  const invisible = valid.filter(tool => tool.name !== "slice_find_text")
  assert.throws(() => assertRoomSharedBrowserStatePhaseEvidence({
    records: invisible, phase: "after-restore", fixtureUrl, generation,
    expectedProviderSessionId: "provider-thread-1", actualProviderSessionId: "provider-thread-1",
  }), /visible through the Room Computer observer/)
})

test("provider task uses the slice command and first-party tools without page script injection", () => {
  const before = roomSharedBrowserStatePrompt({ phase: "before-save", fixtureUrl, generation })
  const after = roomSharedBrowserStatePrompt({ phase: "after-restore", fixtureUrl, generation })
  assert.match(before, /\/usr\/bin\/xmessage/)
  assert.doesNotMatch(commandFromPrompt(after), /\/usr\/bin\/xmessage/)
  assert.match(before, /slice_browser_text/)
  assert.match(before, /chrome:\/\/sandbox/)
  assert.doesNotMatch(before, /Runtime\.evaluate|execute_script|javascript:/i)
  assert.match(after, /test -x/)
  assert.doesNotMatch(after, /cat >|chmod 700/)
})

function records(phase) {
  const programMarker = `ROOM_GRAPHICAL_PROGRAM_${generation}`
  const command = commandFromPrompt(roomSharedBrowserStatePrompt({ phase, fixtureUrl, generation }))
  const marker = phase === "before-save" ? "ROOM_GRAPHICAL_PROGRAM_INSTALLED" : "ROOM_GRAPHICAL_PROGRAM_PRESENT_AFTER_RESTORE"
  return [
    { name: "exec_command", status: "completed", input: command, output: marker, sourceKind: "provider_tool" },
    { name: "slice_find_text", status: "completed", input: { query: programMarker },
      output: JSON.stringify({ matches: [{ text: programMarker, center_x: 10, center_y: 10 }] }), sourceKind: "provider_tool" },
    { name: "slice_open_url", status: "completed", input: { url: "chrome://sandbox" }, output: "opened chrome://sandbox", sourceKind: "provider_tool" },
    { name: "slice_browser_text", status: "completed", input: {},
      output: "Namespace sandbox: Yes\nSeccomp-BPF sandbox: Yes", sourceKind: "provider_tool" },
    { name: "slice_open_url", status: "completed", input: { url: fixtureUrl }, output: `opened ${fixtureUrl}`, sourceKind: "provider_tool" },
    { name: "slice_browser_wait_for_text", status: "completed", input: { text: "ROOM_BROWSER_STATE cookie=PASS" }, output: "found", sourceKind: "provider_tool" },
    { name: "slice_browser_text", status: "completed", input: {}, output: stateText(markers), sourceKind: "provider_tool" },
  ]
}

function commandFromPrompt(prompt) {
  const matches = [...prompt.matchAll(/```sh\n([\s\S]*?)\n```/g)]
  assert.equal(matches.length, 1, "provider prompt must contain exactly one slice command")
  return matches[0][1]
}

function stateText(values) {
  return `ROOM_BROWSER_STATE ${Object.entries(values).map(([key, value]) => `${key}=${value}`).join(" ")}`
}

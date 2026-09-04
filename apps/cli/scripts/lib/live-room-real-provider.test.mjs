import assert from "node:assert/strict"
import test from "node:test"
import { roomRealProviderOptions, runRoomRealProvider, runRoomRealProviderAction } from "./live-room-real-provider.mjs"

const secret = "synthetic-secret-never-in-diagnostic"
const entry = (kind, text, entry_index = 1) => ({ entry_index, entry: { kind, text } })

test("Web real-provider mode requires an explicit opt-in and model", () => {
  const env = { CHARIOX_ROOM_DRILL_FOCUS: "web-companion", CHARIOX_ROOM_DRILL_PROVIDER: "codex" }
  assert.equal(roomRealProviderOptions(env), null)
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER: "1" }), /model/)
  assert.equal(roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER: "1", CHARIOX_ROOM_DRILL_MODEL: "gpt-5.4" }).provider, "codex")
})

test("shared action runner waits for Web readiness without claiming TUI observation", async () => {
  const run = fixture({ actions: [{ actor_id: "agent:agent-2", kind: "pointer_click", state: "completed", mode: "computer",
    action_id: "action-1", sequence: 1, arguments: { x: 640, y: 400, button: "left", click_count: 1 },
  }] })
  run.input.agent = { id: "agent-2" }
  let ready = false
  run.input.beforePrompt = async (agent) => {
    assert.equal(agent.id, "agent-2")
    assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
    ready = true
  }
  run.input.waitForTuis = async () => assert.fail("action runner must not claim TUI observation")
  run.input.expectedPhysicalEffect = "POINTER_CLICK_COUNT=1"
  const result = await runRoomRealProviderAction(run.input)
  assert.equal(ready, true)
  assert.equal(run.calls.some((call) => call.name === "spawnAgent"), false)
  assert.equal(result.expectedPhysicalEffect, "POINTER_CLICK_COUNT=1")
  assert.equal(result.localTuiObserved, undefined)
  assert.equal(run.checkpoints.at(-1).phase, "action-completed")
})

test("failed Web readiness cannot submit a provider prompt", async () => {
  const run = fixture()
  run.input.beforePrompt = async () => { throw new Error("Web not ready") }
  await assert.rejects(runRoomRealProviderAction(run.input), /Web not ready/)
  assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
})

for (const [field, value] of [["provider", "dev-stub"], ["model", "wrong"], ["account_profile", "wrong"], ["session_id", "other-room"]]) {
  test(`reused agent rejects authoritative ${field} mismatch before prompting`, async () => {
    const run = fixture({ state: { SessionState: { session: { agents: [{ id: "agent-2", provider: "opencode", model: "fixture",
      account_profile: "default", session_id: "room", [field]: value }] } } } })
    run.input.agent = { id: "agent-2", provider: "opencode", model: "fixture" }
    await assert.rejects(runRoomRealProviderAction(run.input), /provider configuration/)
    assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
  })
}

test("reused agent must belong to the intended slice", async () => {
  const run = fixture({ slices: [{ id: "other", agent_ids: ["agent-2"] }] })
  run.input.agent = { id: "agent-2" }
  await assert.rejects(runRoomRealProviderAction(run.input), /intended slice/)
  assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
})

test("a completed click from before this prompt cannot satisfy the action wait", async () => {
  const stale = { actor_id: "agent:agent-2", kind: "pointer_click", state: "completed", mode: "computer",
    action_id: "old", sequence: 7, arguments: { x: 640, y: 400, button: "left", click_count: 1 } }
  const run = fixture({ actions: [stale], priorActions: [stale] })
  run.input.agent = { id: "agent-2" }
  await assert.rejects(runRoomRealProviderAction(run.input), /fixture action timeout/)
})

function fixture({ turns = [], blobs = {}, submit, state, actions = [], priorActions = [], slices } = {}) {
  const checkpoints = []
  const calls = []
  const requests = Object.fromEntries([
    "spawnAgent", "attachToSession", "submitPrompt", "listRoomEnvironmentActionHistory",
    "getSessionState", "getSessionHistoryOutline", "getSessionHistoryBlobContent", "listSlices",
  ].map((name) => [`${name}Request`, (...args) => ({ name, args })]))
  const input = {
    requests, sessionId: "room", sliceId: "slice", workspace: "/fixture",
    options: { provider: "opencode", model: "fixture", accountProfile: "default" },
    client: { send: async (request) => {
      calls.push(request)
      switch (request.name) {
        case "spawnAgent": return { AgentSpawned: { agent: { id: "agent-2", session_id: "room", provider: "opencode", model: "fixture", account_profile: "default" } } }
        case "attachToSession": return { SessionAttached: { attachment: { id: "attachment" } } }
        case "submitPrompt": return submit ?? { PromptSubmitted: {} }
        case "listRoomEnvironmentActionHistory": return { RoomEnvironmentActionHistoryListed: { page: {
          actions: calls.some((call) => call.name === "submitPrompt") ? actions : priorActions,
        } } }
        case "listSlices": return { SlicesListed: { slices: slices ?? [{ id: "slice", agent_ids: ["agent-2"] }] } }
        case "getSessionState": return state ?? { SessionState: {
          session: { agents: [{ id: "agent-2", session_id: "room", provider: "opencode", model: "fixture", account_profile: "default", state: "Working" }] },
          agent_activity: { "agent-2": { status: "working", prompt_status: "running", active_turn: { phase: "awaiting_first_output" } } },
        } }
        case "getSessionHistoryOutline": return { SessionHistoryOutline: { agents: [{ agent_id: "agent-2", turns }] } }
        case "getSessionHistoryBlobContent": return { SessionHistoryBlobContent: { entries: blobs[request.args[2]] ?? [] } }
        default: throw new Error("unexpected fixture request")
      }
    } },
    checkpoint: async (value) => checkpoints.push(value),
    waitFor: async (check) => { const found = await check(); if (!found) throw new Error("fixture action timeout"); return found },
    withTimeout: async (promise) => promise,
    waitForPhysicalEffect: async () => {}, waitForTuis: async () => {}, screenshot: async () => {},
  }
  return { input, checkpoints, calls }
}

test("failure retains lifecycle and unrecognized provider error without copying text", async () => {
  const run = fixture({ turns: [{ lifecycle: "completed", entries: [], blobs: [],
    summary: entry("provider_error", `Unclassified provider failure ${secret}`),
  }] })
  await assert.rejects(runRoomRealProvider(run.input), /provider turn failed before/)
  const diagnostic = run.checkpoints.at(-1).diagnostic
  assert.equal(diagnostic.agentState, "Working")
  assert.equal(diagnostic.promptStatus, "running")
  assert.equal(diagnostic.activeTurnPhase, "awaiting_first_output")
  assert.equal(diagnostic.turns[0].lifecycle, "completed")
  assert.equal(diagnostic.entryCounts.provider_error, 1)
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("tool output and failed Room actions are distinguished from absent tool output", async () => {
  const run = fixture({ turns: [{ lifecycle: "open", entries: [entry("provider_tool", `slice_mouse ${secret}`)], blobs: [] }],
    actions: [{ actor_id: "agent:agent-2", kind: "pointer_click", state: "failed", error: secret },
      { actor_id: "agent:someone-else", kind: "pointer_click", state: "completed" }],
  })
  await assert.rejects(runRoomRealProvider(run.input), /fixture action timeout/)
  const diagnostic = run.checkpoints.at(-1).diagnostic
  assert.equal(diagnostic.entryCounts.provider_tool, 1)
  assert.equal(diagnostic.computerToolMentioned, true)
  assert.equal(diagnostic.actionCounts.failed, 1)
  assert.equal(diagnostic.actionCounts.completed, 0)
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("prompt rejection fails immediately rather than waiting for an impossible action", async () => {
  const run = fixture({ submit: { Error: { message: secret } } })
  let waited = false
  run.input.waitFor = async () => { waited = true; throw new Error("must not poll") }
  await assert.rejects(runRoomRealProvider(run.input), /PromptSubmitted/)
  assert.equal(waited, false)
  assert.equal(run.checkpoints.at(-1).phase, "action-failed")
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("oversized blobs are skipped and unknown enum values cannot escape into evidence", async () => {
  const run = fixture({ state: { SessionState: { session: { agents: [{ id: "agent-2", state: secret }] }, agent_activity: {} } },
    turns: [{ lifecycle: secret, entries: [entry(secret, secret)], blobs: [
      { blob_id: "oversized", total_chars: 1_000_000, kind: "provider_error", summary: `unauthorized ${secret}` },
      { blob_id: "small", total_chars: 100, kind: "provider_error", summary: "" },
    ] }], blobs: { small: [entry("provider_error", `rate limit ${secret}`, 2)] },
  })
  await assert.rejects(runRoomRealProvider(run.input), /fixture action timeout/)
  const diagnostic = run.checkpoints.at(-1).diagnostic
  assert.equal(diagnostic.agentState, "unknown")
  assert.equal(diagnostic.turns[0].lifecycle, "unknown")
  assert.ok(diagnostic.codes.includes("unauthorized"))
  assert.ok(diagnostic.codes.includes("rate_limit"))
  assert.equal(diagnostic.truncated, true)
  assert.deepEqual(run.calls.filter((r) => r.name === "getSessionHistoryBlobContent").map((r) => r.args[2]), ["small"])
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("partial evidence survives a blob timeout and starts no later requests", async (t) => {
  let clock = 1_000
  t.mock.method(Date, "now", () => clock)
  const run = fixture({ turns: [{ lifecycle: "open", entries: [entry("provider_output", secret)], blobs: [
    { blob_id: "stall", total_chars: 10, kind: "provider_tool" },
    { blob_id: "later", total_chars: 10, kind: "provider_error" },
  ] }] })
  let stalled = false
  const send = run.input.client.send
  run.input.client.send = (request) => {
    if (request.name === "getSessionHistoryBlobContent") {
      run.calls.push(request)
      stalled = true
      return new Promise(() => {})
    }
    return send(request)
  }
  run.input.withTimeout = async (promise, milliseconds) => {
    if (stalled) { clock += milliseconds; throw new Error(secret) }
    return promise
  }
  await assert.rejects(runRoomRealProvider(run.input), /fixture action timeout/)
  const diagnostic = run.checkpoints.at(-1).diagnostic
  assert.equal(diagnostic.entryCounts.provider_output, 1)
  assert.ok(diagnostic.codes.includes("blob_unavailable"))
  assert.equal(diagnostic.truncated, true)
  assert.equal(run.calls.filter((r) => r.name === "getSessionHistoryBlobContent").length, 1)
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("many small blobs still obey the total request budget", async () => {
  const run = fixture({ turns: [{ lifecycle: "open", entries: [], blobs:
    Array.from({ length: 40 }, (_, index) => ({ blob_id: `blob-${index}`, total_chars: 10, kind: "provider_output" })),
  }] })
  await assert.rejects(runRoomRealProvider(run.input), /fixture action timeout/)
  assert.equal(run.calls.filter((r) => r.name === "getSessionHistoryBlobContent").length, 8)
  assert.equal(run.checkpoints.at(-1).diagnostic.truncated, true)
})

test("successful provider action still requires physical and both TUI observations", async () => {
  const run = fixture({ actions: [{ actor_id: "agent:agent-2", kind: "pointer_click", state: "completed", mode: "computer",
    action_id: "action-1", sequence: 1, arguments: { x: 640, y: 400, button: "left", click_count: 1 },
  }] })
  const observed = []
  run.input.waitForPhysicalEffect = async (marker) => observed.push(marker)
  run.input.waitForTuis = async (pattern) => {
    assert.match("Room action #1: real-opencode · computer pointer_click · completed", pattern)
    observed.push("both-tuis")
  }
  const result = await runRoomRealProvider(run.input)
  assert.deepEqual(observed, ["POINTER_CLICK_COUNT=2", "both-tuis"])
  assert.equal(result.actionId, "action-1")
  assert.equal(run.calls.some((r) => r.name === "getSessionHistoryOutline"), false)
})

for (const [message, expected] of [
  ["API key is missing", "missing_api_key"],
  ["OpenCode MCP server is needs_client_registration", "mcp_setup"],
  ["OpenCode reported an unknown assistant error", "unknown_provider_error"],
  ["OpenCode request failed after 3 attempts", "provider_request_failed"],
  ["Invalid schema for function", "invalid_tool_schema"],
  ["ProviderModelNotFoundError", "model_unavailable"],
  ["Provider session failed: Token refresh failed: 401", "auth_refresh_failed"],
]) {
  test(`classifies ${expected} without retaining its payload`, async () => {
    const run = fixture({ turns: [{ lifecycle: "completed", entries: [entry("provider_error", `${message}: ${secret}`)], blobs: [] }] })
    await assert.rejects(runRoomRealProvider(run.input))
    assert.ok(run.checkpoints.at(-1).diagnostic.codes.includes(expected))
    assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
  })
}

test("completed error turn ends the action wait without exhausting its deadline", async () => {
  const run = fixture({ turns: [{ lifecycle: "completed", entries: [entry("provider_error", secret)], blobs: [] }] })
  run.input.waitFor = async (check) => {
    const terminal = await check()
    assert.ok(terminal, "a completed provider failure must stop the polling loop")
    return terminal
  }
  await assert.rejects(runRoomRealProvider(run.input), /provider turn failed before/)
})

test("full error blob is inspected even when its preview has the same entry index", async () => {
  const run = fixture({ turns: [{ lifecycle: "completed", entries: [],
    summary: entry("provider_error", "OpenCode error", 8),
    blobs: [{ blob_id: "full", kind: "provider_error", summary: "OpenCode error", total_chars: 100 }],
  }], blobs: { full: [entry("provider_error", `Invalid schema for function ${secret}`, 8)] } })
  await assert.rejects(runRoomRealProvider(run.input), /provider turn failed before/)
  assert.equal(run.checkpoints.at(-1).diagnostic.entryCounts.provider_error, 1)
  assert.ok(run.checkpoints.at(-1).diagnostic.codes.includes("invalid_tool_schema"))
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

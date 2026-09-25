import assert from "node:assert/strict"
import test from "node:test"
import {
  prepareRoomRealProviderAgent,
  roomProviderAgentContract,
  roomProviderAgentReadyMetadata,
  roomRealProviderOptions,
  runRoomRealProvider,
  runRoomRealProviderAction,
} from "./live-room-real-provider.mjs"

const secret = "synthetic-secret-never-in-diagnostic"
const entry = (kind, text, entry_index = 1) => ({ entry_index, entry: { kind, text } })

test("real provider drills honor the selected account and effort through preparation", async () => {
  const options = roomRealProviderOptions({ CHARIOX_ROOM_DRILL_FOCUS: "real-provider",
    CHARIOX_ROOM_DRILL_PROVIDER: "codex", CHARIOX_ROOM_DRILL_MODEL: "gpt-5.6-sol",
    CHARIOX_ROOM_DRILL_ACCOUNT_PROFILE: "codex-1", CHARIOX_ROOM_DRILL_EFFORT: "high",
    CHARIOX_ROOM_DRILL_IMPORT_FIRST: "1" })
  const agent = { id: "agent-2", session_id: "room", provider: "codex", model: "gpt-5.6-sol",
    account_profile: "codex-1", effort: "high", is_processing: false }
  const run = fixture({ state: { SessionState: { session: { id: "room", agents: [agent] } } } })
  run.input.options = options
  await prepareRoomRealProviderAgent(run.input)
  assert.deepEqual(run.calls.find(call => call.name === "importSliceProviderAuth").args,
    ["slice", "codex", "codex-1"])
  const spawn = run.calls.find(call => call.name === "spawnAgent").args
  assert.equal(spawn[5], "high")
  assert.equal(spawn[11], "codex-1")
  assert.equal(roomProviderAgentReadyMetadata({ agent, sessionId: "room", sliceId: "slice", options }).effort, "high")
  agent.effort = "low"
  await assert.rejects(prepareRoomRealProviderAgent(run.input), /authoritative provider effort/)
  assert.equal(run.calls.some(call => call.name === "submitPrompt"), false)
})

test("provider selection defaults remain explicit and empty overrides are rejected", () => {
  const env = { CHARIOX_ROOM_DRILL_FOCUS: "web-companion", CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER: "1",
    CHARIOX_ROOM_DRILL_PROVIDER: "codex", CHARIOX_ROOM_DRILL_MODEL: "gpt-5.6-sol" }
  assert.equal(roomRealProviderOptions(env).accountProfile, "default")
  assert.equal(roomRealProviderOptions(env).effort, "low")
  for (const key of ["CHARIOX_ROOM_DRILL_ACCOUNT_PROFILE", "CHARIOX_ROOM_DRILL_EFFORT"]) {
    assert.throws(() => roomRealProviderOptions({ ...env, [key]: " " }), /must not be empty/)
  }
})

test("office work explicitly selects Computer mode for standalone and Web companion drills", () => {
  const env = { CHARIOX_ROOM_DRILL_FOCUS: "real-provider", CHARIOX_ROOM_DRILL_PROVIDER: "codex",
    CHARIOX_ROOM_DRILL_MODEL: "gpt-5.6-sol", CHARIOX_ROOM_DRILL_COMPUTER_TASK: "office" }
  assert.equal(roomRealProviderOptions(env).computerTask, "office")
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_PROVIDER_MODE: "browser" }), /Computer task/)
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_COMPUTER_TASK: "unknown" }), /Computer task/)
  assert.equal(roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_FOCUS: "web-companion",
    CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER: "1" }).computerTask, "office")
})

test("a completed Room action cannot pass while its provider turn remains open", async () => {
  const run = fixture({ actions: [{ actor_id: "agent:agent-2", kind: "pointer_click", state: "completed", mode: "computer",
    action_id: "action-1", sequence: 1, arguments: { x: 640, y: 400, button: "left", click_count: 1 },
  }], turns: [{ turn_id: "current", prompt_id: "prompt-current", lifecycle: "open", entries: [], blobs: [] }] })
  await assert.rejects(runRoomRealProviderAction(run.input), /fixture action timeout/)
})

function settlementFixture(options = {}) {
  return fixture({ actions: [{ actor_id: "agent:agent-2", kind: "pointer_click", state: "completed", mode: "computer",
    action_id: "action-1", sequence: 1, arguments: { x: 640, y: 400, button: "left", click_count: 1 },
  }], ...options })
}

test("completed history for another prompt cannot prove settlement", async () => {
  const run = settlementFixture({ turns: [{ turn_id: "older", prompt_id: "prompt-other", lifecycle: "completed", entries: [], blobs: [] }] })
  await assert.rejects(runRoomRealProviderAction(run.input), /fixture action timeout/)
})

test("settlement waits for the matching completed turn and an idle agent", async () => {
  const current = { turn_id: "current", prompt_id: "prompt-current", lifecycle: "open", entries: [], blobs: [] }
  const state = { SessionState: { session: { id: "room", agents: [{ id: "agent-2", session_id: "room",
    provider: "opencode", model: "fixture", account_profile: "default", is_processing: false }] } } }
  const run = settlementFixture({ turns: [current], state })
  const wait = run.input.waitFor
  run.input.waitFor = async (check, timeout, message) => {
    if (!message.includes("settle after")) return wait(check)
    assert.equal(timeout, 60_000)
    assert.equal(await check(), false, "an open turn is not settled")
    current.lifecycle = "completed"
    state.SessionState.session.agents[0].is_processing = true
    assert.equal(await check(), false, "a still-processing agent is not idle")
    state.SessionState.session.agents[0].is_processing = false
    return check()
  }
  const result = await runRoomRealProviderAction(run.input)
  assert.deepEqual(result.settlement, { promptId: "prompt-current", turnId: "current", lifecycle: "completed", agentIdle: true })
})

test("missing submission identity cannot pass a completed action", async () => {
  const run = settlementFixture({ submit: { PromptSubmitted: {} } })
  await assert.rejects(runRoomRealProviderAction(run.input), /lacks an identity/)
})

test("a queued submission uses its own prompt identity for settlement", async () => {
  const run = settlementFixture({ submit: { PromptSubmitted: { outcome: { Queued: { prompt: { id: "prompt-current" } } } } } })
  assert.equal((await runRoomRealProviderAction(run.input)).settlement.promptId, "prompt-current")
})

test("post-action provider failure stops a retrying waiter without exposing error text", async () => {
  const run = settlementFixture({ turns: [{ turn_id: "current", prompt_id: "prompt-current", lifecycle: "completed",
    entries: [entry("provider_error", secret)], blobs: [] }] })
  const wait = run.input.waitFor
  run.input.waitFor = async (check, _timeout, message) => {
    if (!message.includes("settle after")) return wait(check)
    // Production polling retries read errors; a terminal failure must be a
    // result, rather than an exception swallowed until the deadline.
    const result = await check().catch(() => false)
    assert.ok(result, "terminal provider failure must stop settlement polling")
    return result
  }
  await assert.rejects(runRoomRealProviderAction(run.input), /official provider turn failed after the Room action/)
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("field replacement is an explicit form-only recovery drill", () => {
  const env = { CHARIOX_ROOM_DRILL_FOCUS: "real-provider", CHARIOX_ROOM_DRILL_PROVIDER: "codex", CHARIOX_ROOM_DRILL_MODEL: "gpt-5.4",
    CHARIOX_ROOM_DRILL_PROVIDER_MODE: "browser", CHARIOX_ROOM_DRILL_BROWSER_TASK: "form", CHARIOX_ROOM_DRILL_BROWSER_MUTATION: "replace-field" }
  assert.equal(roomRealProviderOptions(env).browserMutation, "replace-field")
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_BROWSER_MUTATION: "unknown" }), /mutation/)
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_BROWSER_TASK: "click" }), /form/)
})

const recoveryActions = () => [
  { actor_id: "agent:agent-2", mode: "browser", kind: "click", state: "completed", action_id: "replace", sequence: 2, targets: [{ kind: "browser_tab", id: "tab-1" }] },
  { actor_id: "agent:agent-2", mode: "browser", kind: "fill", state: "failed", outcome: { status: "failed", code: "controller_failure" }, action_id: "stale", sequence: 3, targets: [{ kind: "browser_tab", id: "tab-1" }] },
  { actor_id: "agent:agent-2", mode: "browser", kind: "fill", state: "completed", action_id: "fresh", sequence: 4, targets: [{ kind: "browser_tab", id: "tab-1" }] },
  { actor_id: "agent:agent-2", mode: "browser", kind: "submit", state: "completed", action_id: "submit", sequence: 5, targets: [{ kind: "browser_tab", id: "tab-1" }] },
]
const staleToolEntry = () => entry("provider_tool", JSON.stringify({ tool: "slice_browser_fill", status: "failed",
  input: { field_id: "old", text: "STALE ATTEMPT MUST NOT LAND" }, error: `stale_element_reference: ${secret}` }))

test("provider recovery requires a failed stale fill before a fresh successful form submission", async () => {
  const run = fixture({ priorActions: [{ sequence: 1 }], actions: recoveryActions(),
    turns: [{ turn_id: "current", prompt_id: "prompt-current", lifecycle: "completed", entries: [staleToolEntry()], blobs: [] }] })
  run.input.options = { ...run.input.options, mode: "browser", browserTask: "form", browserMutation: "replace-field" }
  const result = await runRoomRealProviderAction(run.input)
  assert.equal(result.browserMutation, "replace-field")
  assert.equal(result.replacementActionId, "replace")
  assert.equal(result.staleActionId, "stale")
  assert.equal(result.staleActionSequence, 3)
  assert.equal(result.staleErrorObserved, true)
  assert.equal(result.fillActionId, "fresh")
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("standalone recovery proves page acceptance and both TUI failure notices", async () => {
  const run = fixture({ priorActions: [{ sequence: 1 }], actions: recoveryActions(),
    turns: [{ turn_id: "current", prompt_id: "prompt-current", lifecycle: "completed", entries: [staleToolEntry()], blobs: [] }] })
  run.input.options = { ...run.input.options, mode: "browser", browserTask: "form", browserMutation: "replace-field" }
  const physical = [], notices = []
  run.input.waitForPhysicalEffect = async (value) => physical.push(value)
  run.input.waitForTuis = async (value) => notices.push(value)
  await runRoomRealProvider(run.input)
  assert.ok(physical.includes("BROWSER_STALE_RECOVERY_ACCEPTED"))
  assert.ok(notices.some((pattern) => pattern.test("Room action #3: real-opencode · browser fill · failed (controller_failure)")))
  assert.equal(notices.length, 4)
})

test("recovery rejects unrelated, stale, extra or successful attempts", async () => {
  for (const mutate of [
    (actions) => actions.splice(1, 1),
    (actions) => { actions[1].state = "completed" },
    (actions) => { actions[1].actor_id = "agent:other" },
    (actions) => { actions[1].sequence = 1 },
    (actions) => { actions[1].targets[0].id = "other-tab" },
    (actions) => { actions[1].outcome.code = "process_lost" },
    (actions) => actions.push({ ...actions[1], action_id: "duplicate" }),
  ]) {
    const actions = recoveryActions()
    mutate(actions)
    const run = fixture({ priorActions: [{ sequence: 1 }], actions })
    run.input.options = { ...run.input.options, mode: "browser", browserTask: "form", browserMutation: "replace-field" }
    await assert.rejects(runRoomRealProviderAction(run.input))
  }
})

test("recovery requires actual tool error output, not prompt or model claims", async () => {
  for (const item of [entry("user_prompt", "stale_element_reference"), entry("provider_output", "stale_element_reference"),
    ...[undefined, "completed", "running"].map((status) => entry("provider_tool", JSON.stringify({
      tool: "slice_browser_fill", status, input: { text: "STALE ATTEMPT MUST NOT LAND" }, error: "stale_element_reference",
    }))),
    entry("provider_tool", JSON.stringify({ tool: "slice_browser_fill", input: { text: "STALE ATTEMPT MUST NOT LAND", error: "stale_element_reference" } })),
    entry("provider_tool", JSON.stringify({ tool: "slice_browser_find", error: "stale_element_reference" })),
    entry("provider_tool", JSON.stringify({ tool: "slice_browser_fill", input: { text: "STALE ATTEMPT MUST NOT LAND" }, error: "browser_action_timeout" })),
  ]) {
    const run = fixture({ actions: recoveryActions(), turns: [{ turn_id: "current", entries: [item], blobs: [] }] })
    run.input.options = { ...run.input.options, mode: "browser", browserTask: "form", browserMutation: "replace-field" }
    await assert.rejects(runRoomRealProviderAction(run.input), /fixture action timeout/)
  }
})

test("nested Browser layouts must explicitly select the form task", () => {
  const env = { CHARIOX_ROOM_DRILL_FOCUS: "real-provider", CHARIOX_ROOM_DRILL_PROVIDER: "codex", CHARIOX_ROOM_DRILL_MODEL: "gpt-5.4",
    CHARIOX_ROOM_DRILL_PROVIDER_MODE: "browser", CHARIOX_ROOM_DRILL_BROWSER_TASK: "form" }
  assert.equal(roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_BROWSER_LAYOUT: "nested-frame" }).browserLayout, "nested-frame")
  assert.equal(roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_BROWSER_LAYOUT: "shadow-root" }).browserLayout, "shadow-root")
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_BROWSER_LAYOUT: "unknown" }), /layout/)
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_BROWSER_TASK: "click", CHARIOX_ROOM_DRILL_BROWSER_LAYOUT: "nested-frame" }), /form/)
})

test("Browser form task requires fresh fill then submit in one tab", async () => {
  const actions = [
    { actor_id: "agent:agent-2", kind: "fill", state: "completed", mode: "browser", action_id: "fill", sequence: 2, targets: [{ kind: "browser_tab", id: "tab-1" }] },
    { actor_id: "agent:agent-2", kind: "submit", state: "completed", mode: "browser", action_id: "submit", sequence: 3, targets: [{ kind: "browser_tab", id: "tab-1" }] },
  ]
  const run = fixture({ actions })
  run.input.options = { ...run.input.options, mode: "browser", browserTask: "form", browserLayout: "nested-frame" }
  const result = await runRoomRealProviderAction(run.input)
  assert.equal(result.actionId, "submit")
  assert.equal(result.fillActionId, "fill")
  assert.equal(result.browserTask, "form")
  assert.equal(result.browserLayout, "nested-frame")
  const prompt = run.calls.find((call) => call.name === "submitPrompt").args[3]
  assert.match(prompt, /slice_browser_fill/)
  assert.match(prompt, /slice_browser_submit/)
  assert.match(prompt, /Chariox form sample/)
})

test("standalone form drill verifies accepted navigation and both fill and submit TUI notices", async () => {
  const fill = { actor_id: "agent:agent-2", kind: "fill", mode: "browser", state: "completed", action_id: "fill", sequence: 2, targets: [{ kind: "browser_tab", id: "tab-1" }] }
  const run = fixture({ actions: [fill, { ...fill, action_id: "submit", kind: "submit", sequence: 3 }] })
  run.input.options = { ...run.input.options, mode: "browser", browserTask: "form" }
  const physical = []
  const notices = []
  run.input.waitForPhysicalEffect = async (value) => physical.push(value)
  run.input.waitForTuis = async (pattern) => notices.push(pattern)
  await runRoomRealProvider(run.input)
  assert.deepEqual(physical, ["POINTER_CLICK_COUNT=1", "BROWSER_FORM_ACCEPTED"])
  assert.equal(notices.length, 2)
  assert.match("Room action #2: real-opencode · browser fill · completed", notices[0])
  assert.match("Room action #3: real-opencode · browser submit · completed", notices[1])
})

test("structured Browser mode requires a fresh tab-targeted browser click", async () => {
  const run = fixture({ actions: [{ actor_id: "agent:agent-2", kind: "click", state: "completed", mode: "browser",
    action_id: "browser-action", sequence: 2, targets: [{ kind: "browser_tab", id: "tab-1" }],
  }] })
  run.input.options.mode = "browser"
  const physical = []
  run.input.waitForPhysicalEffect = async (value) => physical.push(value)
  run.input.waitForTuis = async (pattern) => assert.match("Room action #2: real-opencode · browser click · completed", pattern)
  const result = await runRoomRealProvider(run.input)
  assert.equal(result.mode, "browser")
  assert.equal(result.actionId, "browser-action")
  assert.deepEqual(physical, ["POINTER_CLICK_COUNT=2", "BROWSER_CLICK_ACCEPTED"])
  const prompt = run.calls.find((call) => call.name === "submitPrompt").args[3]
  assert.match(prompt, /slice_browser_find/)
  assert.match(prompt, /slice_browser_click/)
  assert.match(prompt, /field_id/)
  assert.doesNotMatch(prompt, /Call slice_mouse/)
})

test("Browser mode cannot be satisfied by Computer input or an unscoped click", async () => {
  for (const invalid of [
    { kind: "pointer_click", mode: "computer", targets: [{ kind: "desktop" }] },
    { kind: "click", mode: "browser", targets: [{ kind: "desktop" }] },
    { kind: "click", mode: "browser", targets: [{ kind: "browser_tab", id: "" }] },
    { kind: "click", mode: "browser", targets: [] },
  ]) {
    const run = fixture({ actions: [{ actor_id: "agent:agent-2", state: "completed", action_id: "bad", sequence: 2, ...invalid }] })
    run.input.options.mode = "browser"
    await assert.rejects(runRoomRealProviderAction(run.input))
  }
})

test("explicit provider mode selects Browser without silently changing the default", () => {
  const env = { CHARIOX_ROOM_DRILL_FOCUS: "real-provider", CHARIOX_ROOM_DRILL_PROVIDER: "codex", CHARIOX_ROOM_DRILL_MODEL: "gpt-5.4" }
  assert.equal(roomRealProviderOptions(env).mode, "computer")
  assert.equal(roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_PROVIDER_MODE: "browser" }).mode, "browser")
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_PROVIDER_MODE: "invalid" }))
})

test("form task must be explicitly selected in Browser mode", () => {
  const env = { CHARIOX_ROOM_DRILL_FOCUS: "real-provider", CHARIOX_ROOM_DRILL_PROVIDER: "codex", CHARIOX_ROOM_DRILL_MODEL: "gpt-5.4" }
  assert.equal(roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_PROVIDER_MODE: "browser", CHARIOX_ROOM_DRILL_BROWSER_TASK: "form" }).browserTask, "form")
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_BROWSER_TASK: "form" }), /Browser/)
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_PROVIDER_MODE: "browser", CHARIOX_ROOM_DRILL_BROWSER_TASK: "unknown" }), /task/)
})

test("form submission cannot reuse a stale, failed, later, foreign-actor or different-tab fill", async () => {
  const fill = { actor_id: "agent:agent-2", kind: "fill", mode: "browser", state: "completed", action_id: "fill", sequence: 2, targets: [{ kind: "browser_tab", id: "tab-1" }] }
  const submit = { ...fill, kind: "submit", action_id: "submit", sequence: 3 }
  for (const invalid of [null, { sequence: 1 }, { state: "failed" }, { sequence: 4 }, { actor_id: "agent:other" },
    { targets: [{ kind: "browser_tab", id: "other-tab" }] }, { mode: "computer" }]) {
    const run = fixture({ priorActions: [{ sequence: 1 }], actions: [submit, ...(invalid ? [{ ...fill, ...invalid }] : [])] })
    run.input.options = { ...run.input.options, mode: "browser", browserTask: "form" }
    await assert.rejects(runRoomRealProviderAction(run.input), /fresh completed fill/)
  }
})

test("Web real-provider mode requires an explicit opt-in and model", () => {
  const env = { CHARIOX_ROOM_DRILL_FOCUS: "web-companion", CHARIOX_ROOM_DRILL_PROVIDER: "codex" }
  assert.equal(roomRealProviderOptions(env), null)
  assert.throws(() => roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER: "1" }), /model/)
  assert.equal(roomRealProviderOptions({ ...env, CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER: "1", CHARIOX_ROOM_DRILL_MODEL: "gpt-5.4" }).provider, "codex")
})

test("provider preparation imports, spawns, and fences without attaching or prompting", async () => {
  const run = fixture()
  run.input.options = { ...run.input.options, importFirst: true }
  const prepared = await prepareRoomRealProviderAgent(run.input)
  assert.equal(prepared.agent.id, "agent-2")
  assert.deepEqual(run.calls.map((call) => call.name), [
    "importSliceProviderAuth", "spawnAgent", "getSessionState", "listSlices",
  ])
  assert.equal(run.calls.some((call) => ["attachToSession", "submitPrompt"].includes(call.name)), false)
  assert.equal(run.checkpoints.at(-1).phase, "agent-prepared")
})

test("provider preparation selects and verifies a home-kernel agent", async () => {
  const agent = providerAgent({ remote_execution: null })
  const run = fixture({
    spawnedAgent: agent,
    state: { SessionState: { session: { id: "room", agents: [agent] } } },
    slices: [{ id: "environment-slice", session_id: "room", agent_ids: [] }],
  })
  run.input.agentPlacement = { kind: "home_kernel" }

  const prepared = await prepareRoomRealProviderAgent(run.input)
  const spawn = run.calls.find((call) => call.name === "spawnAgent").args
  assert.equal(spawn[8], undefined, "home-kernel SpawnAgent must omit kernel_ref")
  assert.equal(spawn[10], undefined, "home-kernel SpawnAgent must omit slice_ref")
  assert.deepEqual(prepared.placement, { kind: "home_kernel" })
  assert.equal(prepared.slice, null)
  assert.equal(run.calls.some((call) => call.name === "listSlices"), true)
  assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
})

test("provider preparation selects and verifies an exact kernel_ref agent", async () => {
  const agent = providerAgent({ remote_execution: remoteBinding("worker-kernel-a") })
  const run = fixture({
    spawnedAgent: agent,
    state: { SessionState: { session: { id: "room", agents: [agent] } } },
    slices: [{ id: "environment-slice", session_id: "room", agent_ids: [] }],
  })
  run.input.agentPlacement = { kind: "kernel_ref", kernelRef: "worker-kernel-a" }

  const prepared = await prepareRoomRealProviderAgent(run.input)
  const spawn = run.calls.find((call) => call.name === "spawnAgent").args
  assert.equal(spawn[8], "worker-kernel-a")
  assert.equal(spawn[10], undefined, "kernel_ref SpawnAgent must omit slice_ref")
  assert.deepEqual(prepared.placement, run.input.agentPlacement)
  assert.equal(prepared.slice, null)
  assert.equal(run.calls.some((call) => call.name === "listSlices"), true)
})

test("explicit slice_ref targets the agent slice while preserving the Room Environment slice", async () => {
  const agent = providerAgent({ remote_execution: remoteBinding("agent-worker") })
  const run = fixture({
    spawnedAgent: { id: agent.id },
    state: { SessionState: { session: { id: "room", agents: [agent] } } },
    slices: [{ id: "agent-slice", session_id: "room", agent_ids: [agent.id], worker_kernel_id: "agent-worker" }],
  })
  run.input.sliceId = "environment-slice"
  run.input.agentPlacement = { kind: "slice_ref", sliceRef: "agent-slice" }
  run.input.options = { ...run.input.options, importFirst: true }

  const prepared = await prepareRoomRealProviderAgent(run.input)
  const spawn = run.calls.find((call) => call.name === "spawnAgent").args
  assert.equal(run.calls.find((call) => call.name === "importSliceProviderAuth").args[0], "agent-slice")
  assert.equal(spawn[8], undefined)
  assert.equal(spawn[10], "agent-slice")
  assert.equal(prepared.slice.id, "agent-slice")
  assert.deepEqual(prepared.placement, { kind: "slice_ref", sliceRef: "agent-slice" })
})

test("provider preparation refuses a kernel_ref response bound to another worker", async () => {
  const agent = providerAgent({ remote_execution: remoteBinding("worker-kernel-b") })
  const run = fixture({
    spawnedAgent: { id: agent.id },
    state: { SessionState: { session: { id: "room", agents: [agent] } } },
    slices: [{ id: "environment-slice", session_id: "room", agent_ids: [] }],
  })
  run.input.agentPlacement = { kind: "kernel_ref", kernelRef: "worker-kernel-a" }

  await assert.rejects(prepareRoomRealProviderAgent(run.input), /requested worker kernel/)
  assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
})

test("home-kernel preparation fails closed when authoritative remote_execution is missing", async () => {
  const missingBinding = fixture({
    spawnedAgent: { id: "agent-2" },
    state: { SessionState: { session: { id: "room", agents: [providerAgent()] } } },
  })
  missingBinding.input.agentPlacement = { kind: "home_kernel" }
  await assert.rejects(prepareRoomRealProviderAgent(missingBinding.input), /requested home kernel/)
  assert.equal(missingBinding.calls.some((call) => call.name === "submitPrompt"), false)
})

test("provider placement is mutually exclusive and unsupported imports fail before requests", async () => {
  const conflicting = fixture()
  conflicting.input.agentPlacement = {
    kind: "kernel_ref",
    kernelRef: "worker-kernel-a",
    sliceRef: "slice",
  }
  await assert.rejects(prepareRoomRealProviderAgent(conflicting.input), /exactly one placement target/)
  assert.equal(conflicting.calls.length, 0)

  for (const placement of [
    { kind: "home_kernel" },
    { kind: "kernel_ref", kernelRef: "worker-kernel-a" },
  ]) {
    const run = fixture()
    run.input.agentPlacement = placement
    run.input.options = { ...run.input.options, importFirst: true }
    await assert.rejects(prepareRoomRealProviderAgent(run.input), /account import is only supported for slice_ref/)
    assert.equal(run.calls.length, 0)
  }
})

test("official provider action uses explicit home and kernel_ref placements", async () => {
  const cases = [
    { placement: { kind: "home_kernel" }, binding: null, mode: "browser", actionKind: "click" },
    { placement: { kind: "kernel_ref", kernelRef: "worker-kernel-a" },
      binding: remoteBinding("worker-kernel-a"), mode: "computer", actionKind: "pointer_click" },
  ]
  for (const { placement, binding, mode, actionKind } of cases) {
    const agent = providerAgent({ remote_execution: binding })
    const run = fixture({
      spawnedAgent: { id: agent.id },
      state: { SessionState: { session: { id: "room", agents: [agent] } } },
      slices: [{ id: "environment-slice", session_id: "room", agent_ids: [] }],
      actions: [mode === "browser"
        ? { actor_id: "agent:agent-2", kind: actionKind, mode, state: "completed", action_id: "browser-action",
          sequence: 1, targets: [{ kind: "browser_tab", id: "tab-1" }] }
        : { actor_id: "agent:agent-2", kind: actionKind, mode, state: "completed", action_id: "computer-action",
          sequence: 1, arguments: { x: 640, y: 400, button: "left", click_count: 1 } }],
    })
    run.input.agentPlacement = placement
    run.input.options = { ...run.input.options, mode }

    const result = await runRoomRealProviderAction(run.input)
    assert.equal(result.actorId, "agent:agent-2")
    assert.equal(result.actionKind, actionKind)
    assert.deepEqual(result.placement, placement)
    const prompt = run.calls.find((call) => call.name === "submitPrompt")
    assert.equal(prompt.args[2], "agent-2", "the official provider prompt must target the prepared Room agent")
  }
})

test("provider preparation reuses an idle exact agent without import, spawn, attach, or prompt", async () => {
  const run = fixture()
  run.input.agent = { id: "agent-2" }
  const prepared = await prepareRoomRealProviderAgent(run.input)
  assert.equal(prepared.agent.id, "agent-2")
  assert.deepEqual(run.calls.map((call) => call.name), ["getSessionState", "listSlices"])
})

test("provider preparation rejects wrong authoritative session/configuration or slice before prompting", async () => {
  const validAgent = { id: "agent-2", session_id: "room", provider: "opencode", model: "fixture",
    account_profile: "default", is_processing: false }
  for (const state of [
    { SessionState: { session: { id: "other-room", agents: [validAgent] } } },
    { SessionState: { session: { id: "room", agents: [{ ...validAgent, provider: "wrong" }] } } },
    { SessionState: { session: { id: "room", agents: [{ ...validAgent, model: "wrong" }] } } },
    { SessionState: { session: { id: "room", agents: [{ ...validAgent, account_profile: "wrong" }] } } },
  ]) {
    const run = fixture({ state })
    await assert.rejects(prepareRoomRealProviderAgent(run.input), /authoritative provider/)
    assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
  }
  const run = fixture({ slices: [{ id: "slice", session_id: "other-room", agent_ids: ["agent-2"] }] })
  await assert.rejects(prepareRoomRealProviderAgent(run.input), /intended slice/)
  assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
})

test("provider ready metadata is an additive redaction-safe identity and task fence", () => {
  const metadata = roomProviderAgentReadyMetadata({
    agent: { id: "agent-2", session_id: "room", provider: "opencode", model: "fixture", account_profile: "default" },
    sessionId: "room",
    sliceId: "slice",
    options: {
      provider: "opencode", model: "fixture", accountProfile: "default", mode: "browser",
      browserTask: "form", browserLayout: "shadow-root", browserMutation: "replace-field",
    },
  })
  assert.deepEqual(metadata, {
    contract: roomProviderAgentContract,
    agentId: "agent-2",
    sessionId: "room",
    sliceId: "slice",
    provider: "opencode",
    model: "fixture",
    accountProfile: "default",
    mode: "browser",
    task: "form",
    browserTask: "form",
    browserLayout: "shadow-root",
    browserMutation: "replace-field",
  })
  assert.doesNotMatch(JSON.stringify(metadata), /prompt|relay|token|secret|credential|history|fill/i)
  assert.throws(() => roomProviderAgentReadyMetadata({
    agent: { id: "agent-2", session_id: "other-room", provider: "opencode", model: "fixture", account_profile: "default" },
    sessionId: "room", sliceId: "slice", options: { provider: "opencode", model: "fixture" },
  }), /session mismatch/)
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
    const run = fixture({ state: { SessionState: { session: { id: "room", agents: [{ id: "agent-2", provider: "opencode", model: "fixture",
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
  const run = fixture({ actions: [stale], priorActions: [stale], turns: [] })
  run.input.agent = { id: "agent-2" }
  await assert.rejects(runRoomRealProviderAction(run.input), /fixture action timeout/)
})

test("an older failed turn cannot abort the reused agent's current prompt", async () => {
  const old = { turn_id: "old", lifecycle: "completed", entries: [entry("provider_error", "old error")], blobs: [] }
  const actions = []
  const current = { turn_id: "current", prompt_id: "prompt-current", lifecycle: "open", entries: [], blobs: [] }
  const run = fixture({ actions, priorTurns: [old], turns: [current, old] })
  run.input.agent = { id: "agent-2" }
  run.input.waitFor = async (check, _timeout, message) => {
    if (message.includes("settle after")) {
      assert.equal(await check(), false)
      current.lifecycle = "completed"
      return check()
    }
    assert.equal(await check(), false, "old failure must not end the current action wait")
    actions.push({ actor_id: "agent:agent-2", kind: "pointer_click", state: "completed", mode: "computer",
      action_id: "new", sequence: 8, arguments: { x: 640, y: 400, button: "left", click_count: 1 } })
    return await check()
  }
  assert.equal((await runRoomRealProviderAction(run.input)).actionId, "new")
})

test("reused agent with an in-flight turn is rejected before prompt submission", async () => {
  const run = fixture({ state: { SessionState: { session: { id: "room", agents: [{ id: "agent-2", provider: "opencode", model: "fixture",
    account_profile: "default", session_id: "room", is_processing: true }] } } } })
  run.input.agent = { id: "agent-2" }
  await assert.rejects(runRoomRealProviderAction(run.input), /must be idle/)
  assert.equal(run.calls.some((call) => call.name === "submitPrompt"), false)
})

function fixture({ turns = [{ turn_id: "current", prompt_id: "prompt-current", lifecycle: "completed", entries: [], blobs: [] }], priorTurns = [], blobs = {}, submit, state, actions = [], priorActions = [], slices, spawnedAgent } = {}) {
  const checkpoints = []
  const calls = []
  const requests = Object.fromEntries([
    "importSliceProviderAuth", "spawnAgent", "attachToSession", "submitPrompt", "listRoomEnvironmentActionHistory",
    "getSessionState", "getSessionHistoryOutline", "getSessionHistoryBlobContent", "listSlices",
  ].map((name) => [`${name}Request`, (...args) => ({ name, args })]))
  const input = {
    requests, sessionId: "room", sliceId: "slice", workspace: "/fixture",
    options: { provider: "opencode", model: "fixture", accountProfile: "default" },
    client: { send: async (request) => {
      calls.push(request)
      switch (request.name) {
        case "importSliceProviderAuth": return { SliceProviderAuthImported: { status: "imported" } }
        case "spawnAgent": return { AgentSpawned: { agent: spawnedAgent ?? { id: "agent-2", session_id: "room", provider: "opencode", model: "fixture", account_profile: "default" } } }
        case "attachToSession": return { SessionAttached: { attachment: { id: "attachment" } } }
        case "submitPrompt": return submit ?? { PromptSubmitted: { outcome: { Started: { prompt: { id: "prompt-current" } } } } }
        case "listRoomEnvironmentActionHistory": return { RoomEnvironmentActionHistoryListed: { page: {
          actions: calls.some((call) => call.name === "submitPrompt") ? actions : priorActions,
        } } }
        case "listSlices": return { SlicesListed: { slices: slices ?? [{ id: "slice", session_id: "room", agent_ids: ["agent-2"] }] } }
        case "getSessionState": return state ?? { SessionState: {
          session: { id: "room", agents: [{ id: "agent-2", session_id: "room", provider: "opencode", model: "fixture", account_profile: "default", state: "Working", is_processing: false }] },
          agent_activity: { "agent-2": { status: "working", prompt_status: "running", active_turn: { phase: "awaiting_first_output" } } },
        } }
        case "getSessionHistoryOutline": return { SessionHistoryOutline: { agents: [{ agent_id: "agent-2",
          turns: calls.some((call) => call.name === "submitPrompt") ? turns : priorTurns,
        }] } }
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

function providerAgent(overrides = {}) {
  return {
    id: "agent-2",
    session_id: "room",
    provider: "opencode",
    model: "fixture",
    account_profile: "default",
    is_processing: false,
    ...overrides,
  }
}

function remoteBinding(workerKernelId) {
  return {
    worker_kernel_id: workerKernelId,
    worker_machine_id: `${workerKernelId}-machine`,
    execution_lease_id: `${workerKernelId}-lease`,
    leased_agent_id: `${workerKernelId}-agent`,
  }
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
  assert.deepEqual(diagnostic.providerErrorSignals, ["provider"])
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

test("diagnostics identify only allowlisted tool names from actual tool records", async () => {
  const tool = name => entry("provider_tool", JSON.stringify({ tool: name, input: secret, output: secret }))
  const run = fixture({ turns: [{ lifecycle: "open", blobs: [], entries: [
    tool("list_mcp_resources"), tool("mcp__chariox__slice_mouse"), tool("private-tool-" + secret),
    entry("user_prompt", JSON.stringify({ tool: "slice_browser_find" })),
    entry("provider_output", JSON.stringify({ tool: "slice_browser_click" })),
  ] }] })
  await assert.rejects(runRoomRealProvider(run.input))
  assert.deepEqual(run.checkpoints.at(-1).diagnostic.observedTools, ["list_mcp_resources", "slice_mouse"])
  assert.equal(run.checkpoints.at(-1).diagnostic.computerToolMentioned, true)
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("diagnostics classify Browser discovery outcomes without retaining queries or results", async () => {
  const discovery = (query, matches, index) => entry("provider_tool", JSON.stringify({
    tool: "mcp__chariox__slice_browser_find", status: "completed", input: { query },
    output: JSON.stringify({ browser: { matches } }),
  }), index)
  const run = fixture({ turns: [{ lifecycle: "completed", blobs: [], entries: [
    discovery("Submit Browser form", [], 1),
    discovery("Browser sample", [{ label: secret, field_id: secret }], 2),
    discovery(secret, [{ label: secret }], 3),
    entry("provider_tool", JSON.stringify({ tool: "slice_browser_find", status: "running",
      input: { query: "Submit Browser form" }, output: { browser: { matches: [] } } })),
    entry("provider_output", JSON.stringify({ tool: "slice_browser_find", status: "completed",
      input: { query: "Submit Browser form" }, output: { browser: { matches: [] } } })),
  ] }] })
  await assert.rejects(runRoomRealProvider(run.input))
  assert.deepEqual(run.checkpoints.at(-1).diagnostic.browserFindResults, [
    { query: "submit", matches: 0 }, { query: "field", matches: 1 }, { query: "other", matches: 1 },
  ])
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("Browser discovery diagnostics retain at most sixteen bounded counts", async () => {
  const record = entry("provider_tool", JSON.stringify({ tool: "slice_browser_find", status: "completed",
    input: { query: "Browser sample" }, output: { browser: { matches: Array(150).fill(null) } } }))
  const run = fixture({ turns: [{ lifecycle: "completed", blobs: [],
    entries: Array.from({ length: 17 }, (_, entry_index) => ({ ...record, entry_index })),
  }] })
  await assert.rejects(runRoomRealProvider(run.input))
  const diagnostic = run.checkpoints.at(-1).diagnostic
  assert.equal(diagnostic.browserFindResults.length, 16)
  assert.equal(diagnostic.browserFindResults[0].matches, 100)
  assert.equal(diagnostic.truncated, true)
})

test("Browser discovery counts one entry represented by both preview and hydrated history", async () => {
  const record = entry("provider_tool", JSON.stringify({ tool: "slice_browser_find", status: "completed",
    input: { query: "Submit Browser form" }, output: { browser: { matches: [] } } }), 7)
  const run = fixture({ turns: [{ lifecycle: "completed", entries: [], summary: record,
    blobs: [{ blob_id: "find", kind: "provider_tool", total_chars: 300 }],
  }], blobs: { find: [record] } })
  await assert.rejects(runRoomRealProvider(run.input))
  const diagnostic = run.checkpoints.at(-1).diagnostic
  assert.equal(diagnostic.entryCounts.provider_tool, 1)
  assert.deepEqual(diagnostic.browserFindResults, [{ query: "submit", matches: 0 }])
  assert.equal(diagnostic.truncated, false)
})

test("a truncated preview does not suppress its hydrated Browser discovery result", async () => {
  const record = entry("provider_tool", JSON.stringify({ tool: "slice_browser_find", status: "completed",
    input: { query: "Submit Browser form" }, output: { browser: { matches: [] } } }), 7)
  const run = fixture({ turns: [{ lifecycle: "completed", entries: [],
    summary: entry("provider_tool", '{"tool":"slice_browser_find"', 7),
    blobs: [{ blob_id: "find", kind: "provider_tool", total_chars: 300 }],
  }], blobs: { find: [record] } })
  await assert.rejects(runRoomRealProvider(run.input))
  assert.deepEqual(run.checkpoints.at(-1).diagnostic.browserFindResults, [{ query: "submit", matches: 0 }])
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
  const run = fixture({ state: { SessionState: { session: { id: "room", agents: [{ id: "agent-2", session_id: "room",
    provider: "opencode", model: "fixture", account_profile: "default", is_processing: false, state: secret }] }, agent_activity: {} } },
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

test("latest provider failure is inspected before an older screenshot-heavy turn consumes the blob budget", async () => {
  const olderBlobs = Array.from({ length: 8 }, (_, index) => ({
    blob_id: `older-${index}`, total_chars: 10, kind: "provider_tool", summary: "",
  }))
  const run = fixture({ turns: [
    { turn_id: "older", lifecycle: "completed", entries: [], blobs: olderBlobs },
    { turn_id: "latest", lifecycle: "completed", entries: [], blobs: [
      { blob_id: "latest-error", total_chars: 100, kind: "provider_error", summary: "" },
    ] },
  ], blobs: {
    ...Object.fromEntries(olderBlobs.map(blob => [blob.blob_id, [entry("provider_tool", "{}")]])),
    "latest-error": [entry("provider_error", `permission denied ${secret}`)],
  } })
  await assert.rejects(runRoomRealProvider(run.input))
  const diagnostic = run.checkpoints.at(-1).diagnostic
  assert.ok(diagnostic.codes.includes("permission_denied"))
  assert.equal(diagnostic.truncated, true)
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
  assert.deepEqual(result.settlement, { promptId: "prompt-current", turnId: "current", lifecycle: "completed", agentIdle: true })
})

for (const [message, expected] of [
  ["API key is missing", "missing_api_key"],
  ["OpenCode MCP server is needs_client_registration", "mcp_setup"],
  ["OpenCode reported an unknown assistant error", "unknown_provider_error"],
  ["OpenCode request failed after 3 attempts", "provider_request_failed"],
  ["Invalid schema for function", "invalid_tool_schema"],
  ["ProviderModelNotFoundError", "model_unavailable"],
  ["Provider session failed: Token refresh failed: 401", "auth_refresh_failed"],
  ["You've hit your limit · resets in 2 hours", "usage_limit"],
  ["You've hit your usage limit", "usage_limit"],
  ["You have hit your usage limit", "usage_limit"],
  ["Fable 5 now uses usage credits and you don't have usage credits", "usage_limit"],
  ["You're out of extra usage", "usage_limit"],
  ["API Error: 529 overloaded_error", "provider_request_failed"],
  ["Claude Code reported an error", "unknown_provider_error"],
  ["Codex reported an unknown error", "unknown_provider_error"],
  ["Codex turn failed", "unknown_provider_error"],
  ["The model gpt-example does not exist or you do not have access to it", "model_unavailable"],
  ["unexpected status 404 Not Found", "provider_request_failed"],
  ["error sending request: certificate verify failed", "tls_error"],
  ["error sending request: dns error", "dns_error"],
  ["Not logged in. Please run login", "auth_required"],
  ["JSON-RPC error: Invalid params", "provider_protocol_error"],
]) {
  test(`classifies ${expected} without retaining its payload`, async () => {
    const run = fixture({ turns: [{ lifecycle: "completed", entries: [entry("provider_error", `${message}: ${secret}`)], blobs: [] }] })
    await assert.rejects(runRoomRealProvider(run.input))
    assert.ok(run.checkpoints.at(-1).diagnostic.codes.includes(expected))
    assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
  })
}

test("unclassified provider errors retain only fixed diagnostic signals, not arbitrary words", async () => {
  const run = fixture({ turns: [{ lifecycle: "completed", entries: [
    entry("user_prompt", "quota credentials sandbox timeout", 1),
    entry("provider_error", `Thread expired at private.example with ${secret}`, 2),
  ], blobs: [] }] })
  await assert.rejects(runRoomRealProvider(run.input))
  assert.deepEqual(run.checkpoints.at(-1).diagnostic.providerErrorSignals, ["expired", "thread"])
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
  assert.equal(JSON.stringify(run.checkpoints).includes("private.example"), false)
})

test("completed error turn ends the action wait without exhausting its deadline", async () => {
  const run = fixture({ turns: [{ lifecycle: "completed", entries: [entry("provider_error", secret)], blobs: [] }] })
  run.input.waitFor = async (check) => {
    const terminal = await check()
    assert.ok(terminal, "a completed provider failure must stop the polling loop")
    return terminal
  }
  await assert.rejects(runRoomRealProvider(run.input), /provider turn failed before/)
})

test("completed provider turn without the requested action fails without exhausting its deadline", async () => {
  const run = fixture({ turns: [{ turn_id: "current", lifecycle: "completed",
    entries: [entry("provider_output", secret)], blobs: [] }] })
  await assert.rejects(runRoomRealProvider(run.input), /provider turn completed without the required Room action/)
  assert.equal(run.checkpoints.at(-1).phase, "action-failed")
  assert.equal(JSON.stringify(run.checkpoints).includes(secret), false)
})

test("action committed while reading the completed turn is rechecked before failing", async () => {
  const actions = []
  const run = fixture({ actions, turns: [{ turn_id: "current", prompt_id: "prompt-current", lifecycle: "completed", entries: [], blobs: [] }] })
  const send = run.input.client.send
  run.input.client.send = async (request) => {
    if (request.name === "getSessionHistoryOutline") actions.push({
      actor_id: "agent:agent-2", kind: "pointer_click", state: "completed", mode: "computer",
      action_id: "arrived", sequence: 1, arguments: { x: 640, y: 400, button: "left", click_count: 1 },
    })
    return send(request)
  }
  assert.equal((await runRoomRealProviderAction(run.input)).actionId, "arrived")
})

test("older completed success cannot abort a reused agent's open turn", async () => {
  const old = { turn_id: "old", lifecycle: "completed", entries: [], blobs: [] }
  const run = fixture({ priorTurns: [old], turns: [
    { turn_id: "current", lifecycle: "open", entries: [], blobs: [] }, old,
  ] })
  run.input.agent = { id: "agent-2" }
  await assert.rejects(runRoomRealProviderAction(run.input), /fixture action timeout/)
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

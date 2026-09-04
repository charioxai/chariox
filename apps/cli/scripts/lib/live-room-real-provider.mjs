import assert from "node:assert/strict"

// Opt-in only: this runs a paid, official provider through the kernel, not a
// driver impersonating an agent by calling its MCP endpoint.
export function roomRealProviderOptions(env) {
  if (env.CHARIOX_ROOM_DRILL_FOCUS !== "real-provider") return null
  const provider = env.CHARIOX_ROOM_DRILL_PROVIDER
  assert.ok(["codex", "claude", "opencode"].includes(provider), "select an official Room drill provider")
  const model = env.CHARIOX_ROOM_DRILL_MODEL?.trim()
  assert.ok(model, "CHARIOX_ROOM_DRILL_MODEL must explicitly select a provider model")
  return { provider, model, accountProfile: "default", importFirst: env.CHARIOX_ROOM_DRILL_IMPORT_FIRST === "1" }
}

export async function runRoomRealProvider(input) {
  const { client, requests, sessionId, sliceId, options } = input
  if (options.importFirst) {
    await input.checkpoint({ phase: "importing-account", provider: options.provider })
    unwrap(await client.send(requests.importSliceProviderAuthRequest(
      sliceId, options.provider, options.accountProfile,
    )), "SliceProviderAuthImported")
  }
  await input.checkpoint({ phase: "spawning", provider: options.provider, importFirst: options.importFirst })
  const alias = `real-${options.provider}`
  const agent = unwrap(await client.send(requests.spawnAgentRequest(
    sessionId, options.provider, alias, options.model, input.workspace,
    "low", "build", "yolo", undefined, undefined, sliceId, options.accountProfile,
  )), "AgentSpawned").agent
  const attachment = unwrap(await client.send(requests.attachToSessionRequest(
    sessionId, "real-provider-drill",
  )), "SessionAttached").attachment
  await input.checkpoint({ phase: "prompting", provider: options.provider, agentId: agent.id })
  await client.send(requests.submitPromptRequest(sessionId, attachment.id, agent.id, [
    "You are validating the Chariox Room computer. Use only the Chariox runtime MCP tools.",
    "Call slice_mouse exactly once with action=click, x=640, y=400, button=left.",
    "The Room desktop is already running. Do not launch a browser, navigate, use shell commands,",
    "edit any files, or call any external service. Do not use a provider-native browser tool.",
    "After that single click, stop and report whether the tool succeeded.",
  ].join(" "), []))
  const actorId = `agent:${agent.id}`
  const action = await input.waitFor(async () => {
    const actions = unwrap(await client.send(requests.listRoomEnvironmentActionHistoryRequest(
      sessionId, null, 100,
    )), "RoomEnvironmentActionHistoryListed").page.actions
    return actions.find((item) => item.actor_id === actorId
      && item.kind === "pointer_click" && item.state === "completed") ?? false
  }, 180_000, "official provider did not complete a Room computer click")
  assert.equal(action.mode, "computer")
  assert.equal(action.arguments.x, 640)
  assert.equal(action.arguments.y, 400)
  assert.equal(action.arguments.button, "left")
  await input.waitForPhysicalEffect("POINTER_CLICK_COUNT=2")
  await input.waitForTuis(new RegExp(`^Room action #\\d+: ${alias} · computer pointer_click · completed$`))
  await input.screenshot("after-real-provider-click")
  const result = {
    provider: options.provider, model: options.model, accountProfile: options.accountProfile, importFirst: options.importFirst,
    agentId: agent.id, actorId, actionId: action.action_id,
    physicalEffect: "POINTER_CLICK_COUNT=2", localTuiObserved: true, remoteTuiObserved: true,
    coverage: "Official provider calls Chariox Computer input in the shared Room",
    skipped: ["structured Browser actions", "Web observation of the provider action", "provider save and resume"],
  }
  await input.checkpoint({ phase: "passed", ...result })
  return result
}

function unwrap(response, variant) {
  assert.ok(response && variant in response, `kernel did not return ${variant}`)
  return response[variant]
}

import assert from "node:assert/strict"
import { captureRoomProviderDiagnostic } from "./live-room-provider-diagnostic.mjs"

// Opt-in only: this runs a paid, official provider through the kernel, not a
// driver impersonating an agent by calling its MCP endpoint.
export function roomRealProviderOptions(env) {
  const web = env.CHARIOX_ROOM_DRILL_FOCUS === "web-companion"
    && env.CHARIOX_ROOM_DRILL_WEB_REAL_PROVIDER === "1"
  if (env.CHARIOX_ROOM_DRILL_FOCUS !== "real-provider" && !web) return null
  const provider = env.CHARIOX_ROOM_DRILL_PROVIDER
  assert.ok(["codex", "claude", "opencode"].includes(provider), "select an official Room drill provider")
  const model = env.CHARIOX_ROOM_DRILL_MODEL?.trim()
  assert.ok(model, "CHARIOX_ROOM_DRILL_MODEL must explicitly select a provider model")
  return { provider, model, accountProfile: "default", importFirst: env.CHARIOX_ROOM_DRILL_IMPORT_FIRST === "1" }
}

export async function runRoomRealProvider(input) {
  const result = await runRoomRealProviderAction(input)
  await input.waitForPhysicalEffect(result.expectedPhysicalEffect)
  await input.waitForTuis(new RegExp(`^Room action #\\d+: real-${result.provider} · computer pointer_click · completed$`))
  await input.screenshot("after-real-provider-click")
  const verified = {
    ...result, physicalEffect: result.expectedPhysicalEffect, localTuiObserved: true, remoteTuiObserved: true,
    coverage: "Official provider calls Chariox Computer input in the shared Room",
    skipped: ["structured Browser actions", "Web observation of the provider action", "provider save and resume"],
  }
  await input.checkpoint({ phase: "passed", ...verified })
  return verified
}

// Shared by the headless physical/TUI drill and the Web companion. This only
// proves the attributed kernel action; each caller must verify its own viewers.
export async function runRoomRealProviderAction(input) {
  const { client, requests, sessionId, sliceId, options } = input
  assert.ok(!(input.agent && options.importFirst), "import-first must precede agent creation")
  if (options.importFirst) {
    await input.checkpoint({ phase: "importing-account", provider: options.provider })
    unwrap(await client.send(requests.importSliceProviderAuthRequest(
      sliceId, options.provider, options.accountProfile,
    )), "SliceProviderAuthImported")
  }
  await input.checkpoint({ phase: "spawning", provider: options.provider, importFirst: options.importFirst })
  const alias = `real-${options.provider}`
  const agent = input.agent ?? unwrap(await client.send(requests.spawnAgentRequest(
    sessionId, options.provider, alias, options.model, input.workspace,
    "low", "build", "yolo", undefined, undefined, sliceId, options.accountProfile,
  )), "AgentSpawned").agent
  const attachment = unwrap(await client.send(requests.attachToSessionRequest(
    sessionId, "real-provider-drill",
  )), "SessionAttached").attachment
  await input.checkpoint({ phase: "prompting", provider: options.provider, agentId: agent.id })
  const actorId = `agent:${agent.id}`
  let action
  let verifiedAgent = agent
  let baselineSequence = 0
  let lastFailureProbe = 0
  try {
    await input.beforePrompt?.(agent)
    if (input.agent) {
      const state = unwrap(await input.withTimeout(client.send(requests.getSessionStateRequest(sessionId)),
        5_000, "provider configuration lookup"), "SessionState")
      verifiedAgent = state.session?.agents?.find((item) => item.id === agent.id)
      const slices = unwrap(await input.withTimeout(client.send(requests.listSlicesRequest()),
        5_000, "provider slice lookup"), "SlicesListed").slices
      assert.ok(slices.some((slice) => slice.id === sliceId && slice.agent_ids?.includes(agent.id)),
        "real provider must belong to the intended slice")
    }
    assert.ok(verifiedAgent && verifiedAgent.session_id === sessionId
      && verifiedAgent.provider === options.provider && verifiedAgent.model === options.model
      && (verifiedAgent.account_profile ?? "default") === options.accountProfile,
    "authoritative provider configuration does not match the requested provider/model/profile/Room")
    const baseline = unwrap(await input.withTimeout(client.send(requests.listRoomEnvironmentActionHistoryRequest(
      sessionId, null, 100)), 5_000, "provider action baseline"), "RoomEnvironmentActionHistoryListed").page.actions
    baselineSequence = baseline.reduce((latest, item) => Number.isSafeInteger(item.sequence)
      ? Math.max(latest, item.sequence) : latest, 0)
    unwrap(await client.send(requests.submitPromptRequest(sessionId, attachment.id, agent.id, [
    "You are validating the Chariox Room computer. Use only the Chariox runtime MCP tools.",
    "Call slice_mouse exactly once with action=click, x=640, y=400, button=left.",
    "The Room desktop is already running. Do not launch a browser, navigate, use shell commands,",
    "edit any files, or call any external service. Do not use a provider-native browser tool.",
    "After that single click, stop and report whether the tool succeeded.",
    ].join(" "), [])), "PromptSubmitted")
    action = await input.waitFor(async () => {
      const actions = unwrap(await client.send(requests.listRoomEnvironmentActionHistoryRequest(
        sessionId, null, 100,
      )), "RoomEnvironmentActionHistoryListed").page.actions
      const completed = actions.find((item) => item.actor_id === actorId
        && Number.isSafeInteger(item.sequence) && item.sequence > baselineSequence
        && item.kind === "pointer_click" && item.state === "completed")
      if (completed) return completed
      // This drill creates a fresh agent and submits exactly one prompt. Stop
      // waiting if that turn has already ended with an error, not on a mere
      // warning/error while the provider is still running.
      if (Date.now() - lastFailureProbe >= 2_000) {
        lastFailureProbe = Date.now()
        const outline = await input.withTimeout(client.send(requests.getSessionHistoryOutlineRequest(
          sessionId, [agent.id], 2,
        )), 2_000, "provider failure probe").catch(() => null)
        const turns = outline?.SessionHistoryOutline?.agents?.find((item) => item.agent_id === agent.id)?.turns ?? []
        if (turns.slice(0, 2).some((turn) => turn.lifecycle === "completed" && (
          turn.summary?.entry?.kind === "provider_error"
          || (turn.entries ?? []).slice(0, 256).some((item) => item.entry?.kind === "provider_error")
          || (turn.blobs ?? []).slice(0, 16).some((item) => item.kind === "provider_error")
        ))) return { providerFailed: true }
      }
      return false
    }, 180_000, "official provider did not complete a Room computer click")
    if (action.providerFailed) throw new Error("official provider turn failed before completing the Room action")
  } catch (error) {
    const diagnostic = await captureRoomProviderDiagnostic({ ...input, agentId: agent.id })
      .catch(() => ({ codes: ["diagnostic_unavailable"] }))
    await input.checkpoint({ phase: "action-failed", provider: options.provider, agentId: agent.id, diagnostic })
    throw error
  }
  assert.equal(action.mode, "computer")
  assert.equal(action.arguments.x, 640)
  assert.equal(action.arguments.y, 400)
  assert.equal(action.arguments.button, "left")
  assert.equal(action.arguments.click_count, 1)
  const result = {
    provider: verifiedAgent.provider, model: verifiedAgent.model, accountProfile: verifiedAgent.account_profile ?? "default", importFirst: options.importFirst,
    agentId: agent.id, actorId, actionId: action.action_id,
    baselineSequence, actionSequence: action.sequence,
    expectedPhysicalEffect: input.expectedPhysicalEffect ?? "POINTER_CLICK_COUNT=2",
  }
  await input.checkpoint({ phase: "action-completed", ...result })
  return result
}

function unwrap(response, variant) {
  assert.ok(response && variant in response, `kernel did not return ${variant}`)
  return response[variant]
}

// Row 525 live cells, one real kernel and real Claude agents (own state, never the lane kernel). The account comes
// through the product setup-token flow: a drill Vault, then SetProviderAccountCredential with the authorized token file
// read in memory (never printed, never in argv or logs).
//   D  a timer armed in a turn that ends WITHOUT events.yield still fires and starts a turn.
//   B  a provider `run_in_background` command survives the agent's model switch and its notification reaches the agent.
//   A  a parent spawns a child; the child starts a background call in a wake turn it does not acknowledge: the call is
//      held through the turn end and the notification turn runs.
// Run: node scripts/drills/background-call-survival.mjs --kernel-binary <path> --token-file <file>
//   --state-root /root/.chariox/dev/<lane>/state --evidence-dir <dir> [--cells d,b,a] [--model claude-haiku-5-5] [--next-model claude-sonnet-5-5]
import { randomBytes } from "node:crypto"
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { listDescendantsRequest, updateAgentProfileRequest } from "../../packages/kernel-client/dist/ipc-agent-requests.js"
import { drillFlags, startDrillKernel, until } from "./own-kernel.mjs"

const { flags, need } = drillFlags(process.argv)
const evidence = need("--evidence-dir")
const cells = (flags.get("--cells") ?? "d,b,a").split(",")
const model = flags.get("--model") ?? "claude-haiku-5-5"
const nextModel = flags.get("--next-model") ?? "claude-sonnet-5-5"
const tokenFile = need("--token-file")
const k = await startDrillKernel({ label: "stallrca", binary: need("--kernel-binary"), stateRoot: need("--state-root"), evidence, extraEnv: { CHARIOX_ROOM_AGENT_TOOLS: "1" } })
const results = []
const cell = (id, pass, detail = {}) => {
  results.push({ id, status: pass === null ? "INCONCLUSIVE" : pass ? "PASS" : "FAIL", ...detail })
  writeFileSync(join(evidence, "cells.json"), JSON.stringify(results, null, 1) + "\n", { mode: 0o600 })
  console.log(id, results.at(-1).status, JSON.stringify(detail))
}
let exitCode = 0
try {
  // The drill Vault and the account, through the product operations.
  const passphrase = randomBytes(32).toString("hex")
  await k.rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
  await k.rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  await k.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: readFileSync(tokenFile, "utf8").trim(), overwrite: true } })
  const profile = (await k.rpc({ GetProviderAccountProfile: { provider: "claude", account_profile: "default" } })).ProviderAccountProfile.profile.profile_id
  k.note("account-stored", { profile })

  async function room(alias, agentModel = model) {
    const work = join(k.dir, "work", alias)
    mkdirSync(work, { recursive: true })
    const made = (await k.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias, agent_defaults: { provider: "claude", account_profile: profile, model: agentModel, effort: "low", permission_level: "yolo" } } })).SessionCreated
    const R = made.session.id, agent = made.agent.id
    const attached = (await k.rpc({ AttachToSession: { session_id: R, client_id: `stallrca-${alias}`, capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    await k.client.subscribeToKernelEvents(R, attached)
    await k.rpc({ LaunchProviderRun: { session_id: R, agent_id: agent, adapter_key: "claude", provider: "claude", account_profile: profile, model: agentModel, variant: "low", native_tui: false } })
    return { R, agent, attached, work }
  }
  const turnsOf = async (R, agent) => (await k.rpc({ GetSessionHistoryOutline: { session_id: R, agent_ids: [agent], latest_prompt_count: 40, include_prefetched_content: true } })).SessionHistoryOutline.agents[0]?.turns ?? []
  const done = turns => turns.filter(turn => turn.lifecycle === "completed")
  const text = turn => JSON.stringify(turn.summary ?? turn).slice(0, 4000)
  const save = (name, value) => writeFileSync(join(evidence, name), JSON.stringify(value, null, 1), { mode: 0o600 })
  const attempt = async (id, run) => {
    try { await run() } catch (error) { cell(id, false, { refused: String(error.message ?? error).slice(0, 400) }); exitCode = 1 }
  }

  if (cells.includes("d")) await attempt("D", async () => {
    const { R, agent, attached } = await room("cell-d")
    const ask = `Call the chariox events_timer tool once with delay_ms 30000 and label "STALL-D". Do NOT call events_yield and do not wait. End your turn at once with the single word ARMED. If a wake for STALL-D later starts a turn for you, reply with the single word STALL-D-WOKE.`
    await k.rpc(submitPromptRequest(R, attached, agent, ask, []))
    await until(async () => done(await turnsOf(R, agent)).length >= 1, "the arming turn completes", 240000)
    const armedAt = Date.now()
    const woke = await until(async () => {
      const turns = done(await turnsOf(R, agent))
      return turns.length >= 2 && turns.slice(1).some(turn => text(turn).includes("STALL-D-WOKE")) ? turns : false
    }, "the timer wakes the agent", 180000).then(turns => turns, () => null)
    save("turns-d.json", await turnsOf(R, agent))
    cell("D-timer-without-yield-wakes-the-agent", !!woke, { seconds_after_arming: Math.round((Date.now() - armedAt) / 1000) })
    if (!woke) exitCode = 1
  })

  if (cells.includes("b")) await attempt("B", async () => {
    const { R, agent, attached, work } = await room("cell-b")
    const marker = join(work, "b.done")
    const ask = `Run exactly this one command with the Bash tool and run_in_background true: sleep 75; echo STALL-B-DONE > ${marker}. Then end your turn with the single word STARTED. When the background command's notification arrives, reply with the single word STALL-B-NOTIFIED.`
    await k.rpc(submitPromptRequest(R, attached, agent, ask, []))
    await until(async () => done(await turnsOf(R, agent)).length >= 1, "the launching turn completes", 240000)
    const launchedAt = Date.now()
    await k.rpc(updateAgentProfileRequest({ sessionId: R, agentId: agent, model: nextModel }))
    k.note("model-switched", { model: nextModel })
    await k.rpc(submitPromptRequest(R, attached, agent, "Reply with the single word PING and do nothing else.", []))
    await until(async () => done(await turnsOf(R, agent)).length >= 2, "the next turn completes", 180000)
    const finished = await until(() => existsSync(marker), "the background command finishes", 180000).then(() => true, () => false)
    const notified = finished && await until(async () => done(await turnsOf(R, agent)).some(turn => text(turn).includes("STALL-B-NOTIFIED")), "the notification turn runs", 180000).then(() => true, () => false)
    save("turns-b.json", await turnsOf(R, agent))
    cell("B-command-survives-the-model-switch", finished, { seconds_after_launch: Math.round((Date.now() - launchedAt) / 1000) })
    cell("B-notification-reaches-the-agent", notified)
    if (!finished || !notified) exitCode = 1
  })

  if (cells.includes("a")) await attempt("A", async () => {
    const { R, agent, attached, work } = await room("cell-a")
    const marker = join(work, "kid.done")
    const kidBrief = `Call the chariox events_timer tool with delay_ms 20000 and label KID-T, then call events_yield on that registration and end your turn. When the KID-T wake arrives you must NOT call events_ack. Instead run exactly this command with the Bash tool and run_in_background true: sleep 70; echo KID-DONE > ${marker}. Then end your turn with the single word STARTED. When the background command's notification arrives, reply with the single word KID-NOTIFIED.`
    const ask = `Run this room command: agent spawn kid --provider claude --model ${model} --effort low . Then run the room command: prompt kid "${kidBrief.replaceAll('"', "'")}" . Then end your turn with the single word SPAWNED. When the child's finish report arrives, reply with the single word PARENT-HEARD.`
    await k.rpc(submitPromptRequest(R, attached, agent, ask, []))
    const kid = await until(async () => {
      const tree = await k.rpc(listDescendantsRequest(R, agent)).then(r => r.AgentTree.tree, () => null)
      const node = tree?.nodes.find(n => n.alias === "kid")
      return node ? { id: node.agent_id } : null
    }, "the child exists", 240000).catch(() => null)
    if (!kid) {
      const parentTurns = await turnsOf(R, agent)
      save("turns-a-parent.json", parentTurns)
      cell("A-child-held-and-notified", false, { why: "the parent never spawned the child", parent_turns: parentTurns.map(turn => `${turn.lifecycle}: ${text(turn).slice(0, 300)}`) })
      exitCode = 1
      return
    }
    const started = await until(async () => done(await turnsOf(R, kid.id)).length >= 2, "the child's wake turn completes", 240000).then(() => true, () => false)
    const finished = started && await until(() => existsSync(marker), "the child's background call finishes", 180000).then(() => true, () => false)
    const notified = finished && await until(async () => done(await turnsOf(R, kid.id)).some(turn => text(turn).includes("KID-NOTIFIED")), "the child's notification turn runs", 180000).then(() => true, () => false)
    const kidTurns = await turnsOf(R, kid.id)
    save("turns-a-child.json", kidTurns)
    save("turns-a-parent.json", await turnsOf(R, agent))
    // A tool call of the child, not the word in its brief: the acknowledgement would void the cell.
    const acked = kidTurns.some(turn => (turn.blobs ?? []).some(blob => blob.kind === "provider_tool" && /"tool":"[^"]*events_ack"/.test(blob.preview ?? "")))
    cell("A-child-held-and-notified", acked ? null : notified, { wake_turn_completed: started, call_finished: finished, notification_turn: notified, child_acknowledged_the_wake: acked })
    if (!notified) exitCode = 1
  })
} catch (error) {
  k.note("error", { message: String(error.message ?? error).slice(0, 400) })
  exitCode = 1
} finally {
  await k.stop()
}
process.exit(exitCode)

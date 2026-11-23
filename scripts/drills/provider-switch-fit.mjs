// Run inside a private transient systemd unit with a clean environment.
// CLAUDE_TOKEN_FILE names the drill account credential, read only for enrollment.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHash, randomBytes } from "node:crypto"
import { appendFileSync, createReadStream, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import * as requests from "../../packages/kernel-client/dist/ipc-requests.js"

const [, , binary, evidenceDirectory, stateRoot, ...specified] = process.argv
assert(binary && evidenceDirectory && stateRoot?.startsWith("/root/.chariox/dev/"), "Provide the kernel binary, evidence directory and isolated state root.")
assert(process.env.CLAUDE_TOKEN_FILE, "The drill account is required.")
const cells = specified.length ? specified : ["F1", "F2", "F3", "F4", "F5"]
const evidence = resolve(evidenceDirectory)
mkdirSync(evidence, { recursive: true, mode: 0o700 })
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
const root = mkdtempSync(join(stateRoot, "switch-fit-"))
const socketRoot = mkdtempSync("/var/tmp/switch-fit-")
for (const name of ["home", "logs", "history", "tmp", "chariox", "config", "cache", "state"]) {
  mkdirSync(join(root, name), { mode: 0o700 })
}
let anyFailed = false
const results = join(evidence, "cells.jsonl")
const report = (cell, status, details = {}) => {
  appendFileSync(results, JSON.stringify({ cell, status, ...details }) + "\n", { mode: 0o600 })
  console.log(cell, status, JSON.stringify(details))
  if (status === "FAIL" || status === "BLOCKED") anyFailed = true
}
async function freePort() {
  const listener = createServer()
  await new Promise(resolve => listener.listen(0, "127.0.0.1", resolve))
  const { port } = listener.address()
  await new Promise(resolve => listener.close(resolve))
  return port
}
async function until(probe, label, timeout = 60_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    const value = await probe()
    if (value) return value
    await delay(350)
  }
  throw Error(`Timed out waiting for ${label}`)
}
const [kernelPort, mcpPort, codexPort, opencodePort, browserPort] = await Promise.all(Array.from({ length: 5 }, freePort))
writeFileSync(join(root, "chariox/config.toml"), `[credential_vault]
backend = "chariox_encrypted"
path = "${root}/chariox/chariox-secret/vault.json"
unlock_policy = "kernel_init"
`, { mode: 0o600 })
const environment = {
  PATH: process.env.PATH, HOME: join(root, "home"), SHELL: "/bin/bash", TMPDIR: join(root, "tmp"), IS_SANDBOX: "1",
  XDG_CONFIG_HOME: join(root, "config"), XDG_CACHE_HOME: join(root, "cache"), XDG_STATE_HOME: join(root, "state"),
  CHARIOX_HOME: join(root, "chariox"), CHARIOX_LOG_DIR: join(root, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(root, "history"),
  CHARIOX_DAEMON_SOCKET: join(socketRoot, "kernel.sock"), CHARIOX_DAEMON_ALIAS: "switch-fit", CHARIOX_MACHINE_ALIAS: "switch-fit",
  CHARIOX_ROOM_AGENT_TOOLS: "1", CHARIOX_PROVIDER_DEV_STUB: "1", CLAUDE_CODE_DISABLE_1M_CONTEXT: "1",
  CHARIOX_KERNEL_PORT: String(kernelPort), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_CODEX_PORT: String(codexPort),
  CHARIOX_OPENCODE_PORT: String(opencodePort), CHARIOX_KERNEL_BROWSER_PORT: String(browserPort),
}
const digest = createHash("sha256")
for await (const chunk of createReadStream(binary)) digest.update(chunk)
report("kernel", "START", { sha256: digest.digest("hex") })
const kernelLog = openSync(join(evidence, "kernel.log"), "w", 0o600)
const child = spawn(binary, [], { env: environment, stdio: ["ignore", kernelLog, kernelLog] })
const client = new LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`, {
  localAuthEnvironment: environment, controlRequestRetryDeadlineMs: 1000,
})
const rpc = async request => {
  const response = await client.send(request)
  if (response.Error) throw Error(`${Object.keys(request)[0]}: ${response.Error.message}`)
  return response
}
const skills = async request => (await rpc({ SkillsV2: request })).SkillsV2.result
const stub = { provider: "dev-stub", model: "terminal-echo-a", permission_level: "yolo" }
const claude = { provider: "claude", model: "claude-sonnet-5-5", account_profile: "default", effort: "low" }
const rooms = []

async function room(label) {
  const path = join(root, label)
  mkdirSync(path)
  const made = (await rpc({ CreateSession: { workspace_id: path, worktree_id: path, alias: label, agent_defaults: stub } })).SessionCreated
  const room = made.session.id
  rooms.push(room)
  const attachment = (await rpc({ AttachToSession: { session_id: room, client_id: label, capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  await client.subscribeToKernelEvents(room, attachment)
  return { room, attachment, parent: made.agent.id, project: made.session.project_id }
}
const state = async ({ room }) => (await rpc({ GetSessionState: { session_id: room } })).SessionState
async function launch({ room, attachment }, agent, model) {
  const req = requests.launchProviderRunsRequest([{ sessionId: room, agentId: agent, provider: "dev-stub", model,
    accountProfile: "default", effort: "", native: { nativeTui: false } }], 1)
  const accepted = (await rpc(req)).ProviderRunsLaunchAccepted
  assert.equal(accepted.provider_runs.length, 1, accepted.failures[0]?.message)
  const id = accepted.provider_runs[0].provider_run.id
  return until(async () => {
    const run = (await rpc({ GetProviderRun: { provider_run_id: id } })).ProviderRun.provider_run
    return run.state === "Running" && run
  }, "the dev stub")
}
async function submit({ room, attachment }, agent, text, selection) {
  const turn = selection
    ? (await skills({ action: "submit", session_id: room, attachment_id: attachment,
      target_agent_id: agent, prompt: text, attachments: [], selections: [selection] })).outcome
    : (await rpc(requests.submitPromptRequest(room, attachment, agent, text, []))).PromptSubmitted.outcome
  return Object.values(turn)[0].prompt.id
}
async function turn({ room }, agent, prompt) {
  const result = (await rpc({ GetSessionHistoryOutline: {
    session_id: room, agent_ids: [agent], latest_prompt_count: 3, include_prefetched_content: true,
  } })).SessionHistoryOutline
  return result.agents[0]?.turns.find(row => row.prompt_id === prompt)
}
async function seedHistory(rig, agent, count) {
  await rpc({ FocusAgent: { session_id: rig.room, agent_id: agent } })
  let bytes = 0
  for (let index = 0; index < count; index++) {
    const marker = index === 0 ? "ORIGINAL_BRIEF_KUMQUAT" : index === count - 1 ? "LATEST_TURN_JUNIPER" : `Old entry ${index}`
    const text = count === 1 ? `${marker} LATEST_TURN_JUNIPER` : `${marker}\n${"Synthetic archived draft line. ".repeat(index === 0 ? 10 : 140)}`
    bytes += Buffer.byteLength(text)
    const prompt = await submit(rig, agent, text)
    const history = await until(async () => {
      await rpc(requests.pumpTerminalOutputRequest(rig.room, rig.attachment))
      return turn(rig, agent, prompt)
    }, "the stub turn in history", 15_000)
    if (history.lifecycle === "open") {
      try {
        await rpc({ CompletePrompt: { session_id: rig.room } })
      } catch (error) {
        // The stub can finish while the completion request enters the kernel.
        if ((await turn(rig, agent, prompt))?.lifecycle !== "completed") throw error
      }
    } else {
      assert.equal(history.lifecycle, "completed", "the history seed did not complete")
    }
  }
  return bytes
}
async function install({ project }, name, instructionBytes) {
  const scope = { kind: "project", project_id: project }
  const revision = (await skills({ action: "catalog", project_id: project, agent_id: null })).revision
  const skillMd = `---\nname: ${name}\ndescription: Supplies a fixed marker for the provider switch drill.\n---\nThe marker is SKILL_BODY_CINNAMON.\n${"Filler for sizing the instruction body.\n".repeat(Math.floor(instructionBytes / 40))}`
  const mutation = { scope, expected_revision: revision, actions: [
    { action: "install", source: { kind: "content", skill_md: skillMd }, collision: { action: "reject" }, package_id: null },
  ] }
  const preview = await skills({ action: "preview", mutation })
  const published = await skills({ action: "mutate", mutation, review_receipt_id: preview.review_receipt.id })
  assert.equal(published.changed_skill_ids.length, 1)
  return { source: "chariox", skill_id: published.changed_skill_ids[0], name, catalog_revision: published.revision }
}
function fitLog(prompt) {
  const records = readdirSync(join(root, "logs")).filter(file => file.endsWith(".ndjson")).flatMap(file =>
    readFileSync(join(root, "logs", file), "utf8").split("\n").filter(line => line.includes(prompt))
      .flatMap(line => { try { return [JSON.parse(line)] } catch { return [] } }))
  const keys = ["handoff_bytes_before", "handoff_bytes_after", "handoff_room_bytes",
    "skill_bytes_before", "skill_bytes_after", "skills_by_reference", "target_provider_run_id"]
  return Object.assign({}, ...records.filter(row => /^fitted provider switch /.test(row.message))
    .map(row => Object.fromEntries(keys.filter(key => key in row).map(key => [key, row[key]]))))
}
async function runSwitch(large) {
  const rig = await room(large ? "switch-large" : "switch-small")
  const target = rig.parent
  await launch(rig, target, stub.model)
  const priorBytes = await seedHistory(rig, target, large ? 57 : 1)
  console.log("Built synthetic history", priorBytes)
  const selection = await install(rig, large ? "switch-large-instructions" : "switch-small-instructions", large ? 120_000 : 0)
  // The agent settings command sends this same kernel profile update.
  await rpc({ UpdateAgentProfile: { session_id: rig.room, agent_id: target, ...claude } })
  if (large) {
    await rpc({ UpdateAgentConfig: { session_id: rig.room, agent_id: target, max_context_tokens: 100_000 } })
  }
  console.log("Selected Claude for the next turn")
  const appendix = large ? `\nIgnore this synthetic sizing appendix.\n${"Sizing appendix text. ".repeat(2_100)}` : ""
  const prompt = await submit(rig, target,
    `$${selection.name} Reply with the original brief marker, latest prior turn marker, and the skill marker. If a skill is listed by reference, read the start of its SKILL.md for the marker.${appendix}`, selection)
  console.log("Submitted Claude turn", prompt)
  const settled = await until(async () => {
    await rpc(requests.pumpTerminalOutputRequest(rig.room, rig.attachment))
    const row = await turn(rig, target, prompt)
    return row && row.lifecycle !== "open" && row
  }, "first Claude turn", 210_000)
  const output = [settled.summary, ...settled.entries]
    .filter(entry => entry?.entry.kind === "provider_output")
    .map(entry => entry.entry.text)
    .join("\n")
  const fit = fitLog(prompt)
  const decision = (await state(rig)).session.active_interactions?.find(card => card.related_agent_id === target)
  const details = { priorBytes, prompt, lifecycle: settled.lifecycle, outcome: settled.outcome?.kind,
    decision: decision?.message, choices: decision?.choices.map(choice => choice.label), fit }
  const name = large ? "F1" : "F2"
  const markers = ["ORIGINAL_BRIEF_KUMQUAT", "LATEST_TURN_JUNIPER", "SKILL_BODY_CINNAMON"]
  const sizing = large
    ? fit.skills_by_reference && fit.handoff_bytes_before > fit.handoff_bytes_after
    : !fit.skills_by_reference && fit.handoff_bytes_before === fit.handoff_bytes_after &&
      fit.skill_bytes_before === fit.skill_bytes_after
  const seen = Object.fromEntries(markers.map(marker => [marker, output.includes(marker)]))
  details.markers = seen
  const passed = settled.lifecycle === "completed" && sizing && Object.values(seen).every(Boolean)
  const status = passed ? "PASS" : settled.outcome?.kind === "usage_limit" ? "BLOCKED" : "FAIL"
  report(name, status, details)
  if (large && cells.includes("F3")) {
    const sized = fit.handoff_bytes_before > fit.handoff_bytes_after && fit.handoff_bytes_after > 0 &&
      fit.skill_bytes_before > fit.skill_bytes_after && fit.skill_bytes_after > 0 && fit.skills_by_reference
    report("F3", sized ? "PASS" : "FAIL", fit)
  }
  if (large && cells.includes("F5") && fit.handoff_bytes_after > 0) {
    await rpc({ UpdateAgentConfig: { session_id: rig.room, agent_id: target, clear_max_context_tokens: true } })
    await rpc({ UpdateAgentProfile: {
      session_id: rig.room, agent_id: target, provider: stub.provider, model: stub.model,
      account_profile: "default", clear_effort: true,
    } })
    const returned = await submit(rig, target, "RETURNED_TO_ORIGINAL_PROVIDER")
    const active = await until(async () => {
      const active = (await state(rig)).agent_activity[target]?.active_turn
      return active?.prompt_id === returned && active.provider_run_id && active
    }, "the return turn")
    const run = (await rpc({ GetProviderRun: { provider_run_id: active.provider_run_id } })).ProviderRun.provider_run
    assert.equal(run.provider, "dev-stub")
    await rpc({ FocusAgent: { session_id: rig.room, agent_id: target } })
    await rpc({ CompletePrompt: { session_id: rig.room } })
    report("F5", "PASS", { prompt: returned, run: run.id, provider: run.provider })
  } else if (large && cells.includes("F5")) {
    report("F5", "BLOCKED", { reason: "The first turn on Claude did not pass." })
  }
}
async function minimum() {
  const rig = await room("switch-minimum")
  await launch(rig, rig.parent, stub.model)
  await seedHistory(rig, rig.parent, 1)
  // This is the same profile update used by agent settings set. Use the room's
  // top-level agent so the owner receives the decision.
  await rpc({ UpdateAgentProfile: { session_id: rig.room, agent_id: rig.parent, ...claude } })
  await rpc({ UpdateAgentConfig: {
    session_id: rig.room, agent_id: rig.parent, max_context_tokens: 100_000, permission_level: "required",
  } })
  const prompt = await submit(rig, rig.parent, `Reply with the brief's original marker.\n${"Required request data. ".repeat(5_000)}`)
  const card = await until(async () => (await state(rig)).session.active_interactions?.find(card =>
    card.id.startsWith("task-blocked-") && card.related_agent_id === rig.parent), "the minimum-size recovery decision", 150_000)
  assert.match(card.message, /cannot fit.*message|cannot fit.*handoff/i)
  assert.deepEqual(card.choices.map(c => c.id).sort(), ["cancel", "start_fresh_agent"])
  await rpc({ RespondToInteraction: { session_id: rig.room, interaction_id: card.id, choice_id: "start_fresh_agent" } })
  const fresh = await until(async () => (await rpc({ ListAgents: { session_id: rig.room } })).AgentsListed.agents
    .find(agent => agent.id !== rig.parent), "the fresh agent")
  assert.equal(fresh.permission_level_override, "required")
  assert.equal(fresh.max_context_tokens, 100_000)
  const row = await until(async () => {
    await rpc(requests.pumpTerminalOutputRequest(rig.room, rig.attachment))
    const outline = (await rpc({ GetSessionHistoryOutline: {
      session_id: rig.room, agent_ids: [fresh.id], latest_prompt_count: 1, include_prefetched_content: true,
    } })).SessionHistoryOutline
    const row = outline.agents[0]?.turns[0]
    return row
  }, "the fresh agent’s request with the brief")
  assert.match(row.user_prompt.entry.text, /ORIGINAL_BRIEF_KUMQUAT/)
  report("F4", "PASS", { prompt, decision: card.message, fresh: fresh.id,
    recoveryPrompt: row.prompt_id, recoveryLifecycle: row.lifecycle })
}

try {
  await until(async () => { try { return (await rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel readiness")
  await rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase: randomBytes(24).toString("hex") } } })
  await rpc({ SetProviderAccountCredential: {
    provider: "claude", account_profile: "default", value: readFileSync(process.env.CLAUDE_TOKEN_FILE, "utf8").trim(), overwrite: true,
  } }).catch(() => { throw Error("The drill account could not be enrolled.") })
  for (const [id, execute] of [["F1", () => runSwitch(true)], ["F2", () => runSwitch(false)], ["F4", minimum]]) {
    if (!cells.includes(id)) continue
    try { await execute() } catch (error) { report(id, "FAIL", { message: error.message }); process.exitCode = 1 }
  }
} catch (error) {
  report("kernel", "FAIL", { message: error.message })
  process.exitCode = 1
} finally {
  for (const room of rooms) await rpc({ EndSession: { session_id: room } }).catch(() => {})
  await client.close()
  if (child.exitCode === null && child.signalCode === null) {
    child.kill("SIGTERM")
    await until(() => child.exitCode !== null || child.signalCode !== null, "kernel exit", 30_000)
  }
  rmSync(root, { recursive: true, force: true })
  rmSync(socketRoot, { recursive: true, force: true })
  if (anyFailed) process.exitCode = 1
}

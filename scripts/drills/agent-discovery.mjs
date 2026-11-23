// Run with --old-kernel, --old-cli, --kernel, --state-root and --evidence. Build the CLI
// first. The drill uses OpenCode and its no-sign-in model by default.
import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { randomBytes } from "node:crypto"
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { drillFlags, until } from "./own-kernel.mjs"

const { flags, need } = drillFlags(process.argv)
const oldKernel = need("--old-kernel")
const oldCli = need("--old-cli")
const kernel = need("--kernel")
const stateRoot = need("--state-root")
const evidence = need("--evidence")
const provider = "opencode"
assert(stateRoot.startsWith("/root/.chariox/dev/"), "Use a private drill state root")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true, mode: 0o700 })
const root = mkdtempSync(join(stateRoot, "agent-discovery-"))
const socketRoot = mkdtempSync("/var/tmp/agent-discovery-")
const work = join(root, "work")
const read = path => readFileSync(path, "utf8")
const note = read(new URL("../../apps/kernel/agent-release-notes/13.md", import.meta.url))
const cli = fileURLToPath(new URL("../../apps/cli/dist/index.js", import.meta.url))
const model = flags.get("--model") ?? "opencode/big-pickle"
const bun = process.env.CHARIOX_DRILL_BUN ?? "/usr/local/bin/bun"
const checks = []
const passphrase = randomBytes(24).toString("hex")
let client, unit, pid, room, agent, attachment, account, providerRunId

function cell(id, value, detail = {}) {
  const status = value === null ? "BLOCKED" : value ? "PASS" : "FAIL"
  checks.push({ id, status, ...detail })
  writeFileSync(join(evidence, "cells.json"), JSON.stringify(checks, null, 2) + "\n")
  console.log(id, status, JSON.stringify(detail))
}

async function port() {
  const server = createServer()
  await new Promise((resolve, reject) => server.listen(0, "127.0.0.1", resolve).once("error", reject))
  const { port } = server.address()
  await new Promise(resolve => server.close(resolve))
  return String(port)
}

for (const dir of ["home", "tmp", "chariox", "logs", "history", "config", "cache", "state", "work"]) {
  mkdirSync(join(root, dir), { mode: 0o700 })
}
execFileSync("git", ["init", "-q", "-b", "main", work])
const env = {
  PATH: "/usr/local/bin:/usr/bin:/bin",
  SHELL: "/bin/bash",
  HOME: join(root, "home"),
  TMPDIR: join(root, "tmp"),
  XDG_CONFIG_HOME: join(root, "config"),
  XDG_CACHE_HOME: join(root, "cache"),
  XDG_STATE_HOME: join(root, "state"),
  CHARIOX_HOME: join(root, "chariox"),
  CHARIOX_LOG_DIR: join(root, "logs"),
  CHARIOX_SESSION_HISTORY_DIR: join(root, "history"),
  CHARIOX_DAEMON_SOCKET: join(socketRoot, "kernel.sock"),
  CHARIOX_DAEMON_ALIAS: "agent-discovery",
  CHARIOX_MACHINE_ALIAS: "agent-discovery",
  CHARIOX_ROOM_AGENT_TOOLS: "1",
  CHARIOX_KERNEL_PORT: await port(),
  CHARIOX_MCP_PORT: await port(),
  CHARIOX_OPENCODE_PORT: await port(),
  CHARIOX_OPENCODE_BIN: process.env.CHARIOX_DRILL_OPENCODE ?? "/usr/local/bin/opencode",
  CHARIOX_CODEX_PORT: await port(),
}
function mainPid() {
  return Number(execFileSync("systemctl", ["show", unit, "-p", "MainPID", "--value"], { encoding: "utf8" }))
}

async function rpc(request) {
  const response = await client.send(request)
  if (response.Error) throw new Error(`${Object.keys(request)[0]}: ${response.Error.message}`)
  return response
}

async function start(binary, phase) {
  unit = `agent-discovery-${process.pid}-${phase}.service`
  const log = join(evidence, `kernel-${phase}.log`)
  execFileSync("systemd-run", [
    "--quiet", `--unit=${unit}`, "-p", "RuntimeMaxSec=8min", "-p", "KillMode=mixed",
    "-p", `StandardOutput=append:${log}`, "-p", `StandardError=append:${log}`,
    "/usr/bin/env", "-i", ...Object.entries(env).map(([key, value]) => `${key}=${value}`), binary,
  ])
  pid = await until(() => { const found = mainPid(); return found > 1 ? found : null }, phase + " PID")
  client = new LocalIpcClient(`ws://127.0.0.1:${env.CHARIOX_KERNEL_PORT}/kernel`, {
    localAuthEnvironment: env,
    controlRequestRetryDeadlineMs: 1000,
    controlResponseStallMs: 10000,
  })
  const status = await until(async () => {
    assert.equal(mainPid(), pid, "Drill kernel exited during startup")
    try { return (await rpc({ RelayStatus: null })).RelayStatus?.status } catch { return null }
  }, phase + " ready", 60_000)
  console.log(phase, JSON.stringify({ pid, protocol: status.local_daemon_protocol_version, kernel: status.daemon_id }))
}

async function stop() {
  await client?.close()
  client = undefined
  if (pid > 1 && mainPid() === pid) {
    process.kill(pid, "SIGTERM")
    await until(() => mainPid() !== pid, "drill kernel shutdown", 60_000)
  }
  pid = undefined
}

async function attach() {
  attachment = (await rpc({ AttachToSession: {
    session_id: room, client_id: "agent-discovery-owner", capability_level: "FullTerminal",
  } })).SessionAttached.attachment.id
}

async function notices() {
  const result = await rpc({ PollRuntimeNotices: { session_id: room, attachment_id: attachment } })
  return result.RuntimeNotices.notices.filter(row => row.message === note.trim()).length
}

function runCli(...args) {
  const result = spawnSync(bun, [cli, ...args, "--socket", env.CHARIOX_DAEMON_SOCKET], {
    env, encoding: "utf8", timeout: 35_000,
  })
  if (result.error) throw result.error
  return result
}

function runProgram(entry, ...args) {
  const result = spawnSync(bun, [entry, ...args, "--socket", env.CHARIOX_DAEMON_SOCKET, "--", "/usr/bin/printf", "direct-exec-ok"], {
    env, encoding: "utf8", timeout: 35_000,
  })
  assert.equal(result.status, 0, result.error?.message ?? result.stderr)
  return result
}

async function grantCliAccess() {
  const holder = ["--holder-pid", String(process.pid)]
  const current = runCli("access", "status", ...holder)
  assert.equal(current.status, 0, current.stderr)
  if (JSON.parse(current.stdout).status === "granted") return
  const requested = runCli("access", "request", "--until-exit", "--no-wait", ...holder)
  if (requested.status === 0) return
  assert.equal(requested.status, 6, requested.stderr)
  const card = await until(async () => {
    const { interactions } = (await rpc({ ListOwnerInteractions: {} })).OwnerInteractionsListed
    return interactions.find(card => card.session_id === "kernel-access")
  }, "access request for the drill CLI")
  await rpc({ RespondToInteraction: {
    session_id: "kernel-access", interaction_id: card.interaction_id, choice_id: "approve", passkey: passphrase,
  } })
  const status = runCli("access", "status", ...holder)
  assert.equal(status.status, 0, status.stderr)
  assert.equal(JSON.parse(status.stdout).status, "granted")
}

async function launchProvider() {
  const launched = await rpc({ LaunchProviderRun: {
    session_id: room, agent_id: agent, adapter_key: provider, provider,
    account_profile: account, model, variant: null, native_tui: false,
  } })
  providerRunId = launched.ProviderRunLaunchAccepted.provider_run.id
}

async function unlockVault() {
  await rpc({ UnlockCredentialVault: {
    passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal",
  } })
}

// Inspect only this drill message's system field through OpenCode's local API.
async function providerNoteCopies(marker) {
  const { provider_run: run } = (await rpc({ GetProviderRun: { provider_run_id: providerRunId } })).ProviderRun
  const endpoint = new URL(run.structured_endpoint)
  assert.equal(endpoint.hostname, "127.0.0.1")
  const path = `/session/${encodeURIComponent(run.provider_session_id)}/message?limit=20`
  const response = await fetch(new URL(path, endpoint), { signal: AbortSignal.timeout(10_000) })
  assert.equal(response.status, 200, "OpenCode returned the actual turn messages")
  const messages = await response.json()
  const submitted = messages.find(message => message.info.role === "user"
    && message.parts.some(part => part.type === "text" && part.text.startsWith(marker)))
  assert(submitted, "OpenCode stored the drill's submitted message")
  assert.equal(typeof submitted.info.system, "string", "OpenCode retained this turn's system context")
  return submitted.info.system.split(note.trim()).length - 1
}

async function turn(phase, extra = "Reply READY.") {
  const marker = `discovery-${phase}`
  const prompt = `${marker}. ${extra}`
  const submitted = await rpc(submitPromptRequest(room, attachment, agent, prompt, []))
  const promptId = Object.values(submitted.PromptSubmitted.outcome)[0].prompt.id
  const rows = await until(async () => {
    await rpc({ PumpTerminalOutput: { session_id: room, attachment_id: attachment } })
    const outline = (await rpc({ GetSessionHistoryOutline: {
      session_id: room, agent_ids: [agent], latest_prompt_count: 3, include_prefetched_content: true,
    } })).SessionHistoryOutline
    const history = outline.agents.find(row => row.agent_id === agent)
    const turn = history?.turns.find(row => row.prompt_id === promptId)
    if (!turn || turn.lifecycle === "open") return null
    const blobIds = new Set(turn.blobs.map(row => row.blob_id))
    return { turn, pages: history.prefetched_blobs?.filter(row => blobIds.has(row.blob_id)) ?? [] }
  }, marker + " completed", 120_000)
  if (rows.turn.outcome?.kind === "usage_limit") {
    return { usageLimit: rows.turn.outcome.reason }
  }
  assert.equal(rows.turn.lifecycle, "completed", JSON.stringify(rows.turn.outcome))
  writeFileSync(join(evidence, phase + "-turn.json"), JSON.stringify(rows, null, 2))
  const records = [
    ...rows.turn.entries,
    ...rows.pages.flatMap(page => page.entries),
  ].filter(row => row.entry.kind === "provider_tool").map(row => JSON.parse(row.entry.text))
  const refusal = records.find(row => row.tool?.endsWith("_room_run_command") && row.status === "error")
  const releaseNote = records.find(row => row.tool?.endsWith("_whats_new") && row.status === "completed")
  return {
    noteCopies: await providerNoteCopies(marker),
    refusal: refusal && JSON.parse(refusal.error).error,
    releaseNote: releaseNote && JSON.parse(releaseNote.output).text,
  }
}

const refusalPrompt = `Use the connected Chariox MCP command ending in room_run_command (chariox.room.run_command) with {"command":"session attach missing-room"}. Quote the returned error exactly. Do nothing else.`
const refused = /^(?:agents in Meta mode|Room agents) cannot (?:create, )?attach to, switch, or delete sessions/i
const prefixed = /^local transport `runtime_tool_meta\.run_command` failed: /

try {
  await start(oldKernel, "old")
  await rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
  account = (await rpc({ GetProviderAccountProfile: { provider, account_profile: "default" } })).ProviderAccountProfile.profile.profile_id
  const created = (await rpc({ CreateSession: {
    workspace_id: work, worktree_id: work, alias: "agent-discovery",
    agent_defaults: { provider, model, effort: "low", account_profile: account, permission_level: "yolo" },
  } })).SessionCreated
  room = created.session.id
  agent = created.agent.id
  await attach()
  await unlockVault()
  await grantCliAccess()
  await launchProvider()
  const old = await turn("old", refusalPrompt)
  const oldRead = runCli("whats-new")
  cell("D2-red-client-and-cli", oldRead.status !== 0 && await notices() === 0)
  cell("D2-red-agent", old.usageLimit ? null : old.noteCopies === 0,
    old.usageLimit ? { reason: old.usageLimit } : {})
  cell("D3-red", runProgram(oldCli, "terminal", "open").stderr.trim() === "")
  cell("D5-red", old.usageLimit ? null : prefixed.test(old.refusal) && refused.test(old.refusal.replace(prefixed, "")),
    old.usageLimit ? { reason: old.usageLimit } : { refusal: old.refusal })
  await stop()

  await start(kernel, "upgraded")
  await attach()
  const firstNotices = await notices()
  const repeatedNotices = await notices()
  await unlockVault()
  await grantCliAccess()
  const fromCli = runCli("whats-new")
  const heavier = runProgram(cli, "terminal", "open")
  const direct = runProgram(cli, "exec")
  cell("D3", /^For a one-off command, use chariox exec [^\n]+\n$/.test(heavier.stderr)
    && direct.stderr === "" && direct.stdout.trim() === "direct-exec-ok",
  { hint: heavier.stderr.trim() })

  if (!old.usageLimit) await launchProvider()
  const first = old.usageLimit ? old : await turn("first")
  const second = first.usageLimit ? first : await turn("second", `Read the latest note with the connected Chariox MCP command ending in whats_new (chariox.whats_new), and report its heading. ${refusalPrompt.replace("Do nothing else.", "")}`)
  const operationLogged = readdirSync(env.CHARIOX_LOG_DIR).some(file =>
    file.endsWith(".ndjson") && read(join(env.CHARIOX_LOG_DIR, file)).split("\n").some(line =>
      line.includes("runtime MCP request failed") && line.includes("runtime_tool_meta.run_command")))
  cell("D5", second.usageLimit ? null : refused.test(second.refusal) && operationLogged,
    second.usageLimit ? { reason: second.usageLimit } : { refusal: second.refusal, operationLogged })

  await stop()
  await start(kernel, "restarted")
  await attach()
  const restartNotices = await notices()
  await unlockVault()
  await grantCliAccess()
  const stillRead = runCli("whats-new")
  const cliMatches = fromCli.status === 0 && fromCli.stdout === note && stillRead.status === 0 && stillRead.stdout === note
  cell("D2-client-and-cli", firstNotices === 1 && repeatedNotices === 0 && restartNotices === 0 && cliMatches,
    { firstNotices, repeatedNotices, restartNotices, cliMatches })

  if (!second.usageLimit) await launchProvider()
  const resumed = second.usageLimit ? second : await turn("restarted")
  if (resumed.usageLimit) {
    cell("D2-agent-and-mcp", null, { reason: resumed.usageLimit })
  } else {
    const noteCopies = [first.noteCopies, second.noteCopies, resumed.noteCopies]
    const mcpMatches = second.releaseNote === note
    cell("D2-agent-and-mcp", noteCopies[0] === 1 && noteCopies[1] === 0 && noteCopies[2] === 0 && mcpMatches,
      { noteCopies, mcpMatches })
  }
} catch (error) {
  console.error(error.message)
  process.exitCode = 1
} finally {
  await stop()
  rmSync(socketRoot, { recursive: true, force: true })
  rmSync(root, { recursive: true, force: true })
}
if (checks.some(row => row.status !== "PASS")) process.exitCode = 1

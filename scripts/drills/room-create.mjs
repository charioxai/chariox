// RC1-RC3, RC5, RC6: a Room agent creates Rooms of its own through the real chariox.room.run_command MCP tool, starts an
// agent in one, deletes only the Rooms it made, and the Room survives a kernel restart. Real kernel binary, real runtime MCP
// endpoint and run binding; the calling agent is the private dev-stub MCP fixture, so the drill needs no provider account.
// node scripts/drills/room-create.mjs --kernel-binary <kernel> --state-root <~/.chariox/dev/task> --evidence-dir <outside-checkout>
// Results come from kernel RPCs and the tool's own replies, never from the kernel's state files.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { existsSync, mkdirSync, openSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { setTimeout as delay } from "node:timers/promises"
const tree = fileURLToPath(new URL("../../", import.meta.url)).replace(/\/$/, "")
const { LocalIpcClient } = await import(`${tree}/packages/kernel-client/dist/ipc.js`)
const requests = await import(`${tree}/packages/kernel-client/dist/ipc-requests.js`)
const { bindProviderMcpFixture } = await import(`${tree}/apps/cli/scripts/lib/provider-mcp-fixture.mjs`)

const flags = new Map()
for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
const binary = need("--kernel-binary"), root = need("--state-root"), evidence = need("--evidence-dir")
assert(root.includes("/.chariox/dev/"), "state must be lane-owned")
assert(!(evidence + "/").startsWith(tree + "/"), "evidence must be outside the repository")
for (const sub of ["home", "tmp", "chariox", "logs", "history", "work", "config", "cache", "state"]) mkdirSync(join(root, sub), { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const steps = [], checks = []
const t0 = Date.now()
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), s: Math.round((Date.now() - t0) / 1000), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 1) + "\n", { mode: 0o600 })
  console.log(`RC +${Math.round((Date.now() - t0) / 1000)}s ${name} ${JSON.stringify(data).slice(0, 400)}`)
}
function check(id, passed, detail) {
  checks.push({ id, status: passed ? "PASS" : "FAIL", detail })
  writeFileSync(join(evidence, "verdict.json"), JSON.stringify(checks, null, 2) + "\n")
  note(id, { passed })
}
const port = async () => { const s = createServer(); await new Promise(r => s.listen(0, "127.0.0.1", r)); const n = s.address().port; await new Promise(r => s.close(r)); return n }
const ports = await Promise.all(Array.from({ length: 4 }, port))
writeFileSync(join(root, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${root}/chariox/state.db"\n`, { mode: 0o600 })
// The kernel starts from this environment only: nothing of the calling shell reaches it.
const env = { PATH: process.env.PATH, HOME: join(root, "home"), TMPDIR: join(root, "tmp"), IS_SANDBOX: "1", SHELL: "/bin/bash", CHARIOX_PROVIDER_DEV_STUB: "1", CHARIOX_ROOM_AGENT_TOOLS: "1",
  CHARIOX_HOME: join(root, "chariox"), CHARIOX_LOG_DIR: join(root, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(root, "history"), XDG_CONFIG_HOME: join(root, "config"), XDG_CACHE_HOME: join(root, "cache"), XDG_STATE_HOME: join(root, "state"),
  CHARIOX_DAEMON_SOCKET: join(root, "kernel.sock"), CHARIOX_DAEMON_ALIAS: "rc-drill", CHARIOX_MACHINE_ALIAS: "rc-drill-machine",
  CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
let child, client, rpc
async function start(name) {
  const log = openSync(join(evidence, `${name}.log`), "a", 0o600)
  child = spawn(binary, [], { env, stdio: ["ignore", log, log] })
  assert(Number.isSafeInteger(child.pid) && child.pid > 1, "bad pid")
  note("process-start", { label: name, pid: child.pid })
  client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  rpc = async request => {
    const value = await client.send(request)
    if (value.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${value.Error.code} ${value.Error.message ?? ""}`)
    return value
  }
  for (let i = 0; i < 240; i++) {
    assert(child.exitCode === null && child.signalCode === null, "kernel exited during start")
    try { if ((await client.send({ RelayStatus: null })).RelayStatus) return } catch { /* kernel still starting */ }
    await delay(250)
  }
  throw new Error("kernel start deadline")
}
async function stop() {
  await client?.close().catch(() => {})
  if (child && child.exitCode === null && child.signalCode === null) {
    assert(Number.isSafeInteger(child.pid) && child.pid > 1, "invalid owned PID")
    child.kill("SIGTERM")
    for (let i = 0; i < 200 && child.exitCode === null && child.signalCode === null; i++) await delay(100)
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL")
    note("process-exit", { pid: child.pid, code: child.exitCode, signal: child.signalCode })
  }
}
const rooms = async () => (await rpc({ ListSessions: null })).SessionsListed.sessions
const live = room => room.status !== "Ended"
const work = join(root, "work")

async function main() {
  note("binary", { kernel_sha256: createHash("sha256").update(readFileSync(binary)).digest("hex") })
  await start("kernel")
  const fixtureDefaults = { provider: "dev-stub", account_profile: "default", model: "runtime-mcp-fixture", permission_level: "yolo" }
  const made = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "rc-home", agent_defaults: fixtureDefaults } })).SessionCreated
  const home = made.session.id, creator = made.agent.id
  const bystander = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "rc-bystander", agent_defaults: fixtureDefaults } })).SessionCreated.session.id
  const webRoom = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "rc-web", agent_defaults: fixtureDefaults } })).SessionCreated.session
  const attachment = (await rpc({ AttachToSession: { session_id: home, client_id: "rc-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  const launch = await rpc(requests.launchProviderRunsRequest([{ sessionId: home, provider: "dev-stub", accountProfile: "default", effort: "low", model: "runtime-mcp-fixture", agentId: creator, native: { nativeTui: false } }], 1))
  let run = launch.ProviderRunsLaunchAccepted.provider_runs[0].provider_run
  for (let i = 0; i < 120; i++) {
    const r = Object.values(await rpc(requests.getProviderRunRequest(run.id)))[0], p = r.provider_run ?? r
    if (p.state === "Running") { run = p; break }
    assert(!["Stopped", "Failed", "Exited"].includes(p.state), "provider fixture ended during launch")
    await delay(250)
  }
  const fixture = bindProviderMcpFixture(run, { client, requests, sessionId: home, attachmentId: attachment }).fixtureMcp
  await delay(1500)
  // The agent's own tool: chariox.room.run_command through the runtime MCP endpoint of the kernel.
  const roomCommand = async command => {
    // Each command runs inside a fresh turn of the creating agent: the tool binding lives as long as its turn.
    await rpc(requests.submitPromptRequest(home, attachment, creator, `go ${command}`, []))
    let task
    for (let i = 0; i < 40 && !task; i++) {
      await delay(250)
      const state = (await rpc({ GetSessionState: { session_id: home } })).SessionState
      task = (state.agent_tasks ?? state.session?.agent_tasks ?? []).filter(t => t.agent_id === creator && t.state === "working").at(-1)
    }
    assert(task, "the creator has a running turn")
    let origin = task.prompt_id
    for (let attempt = 0; attempt < 3; attempt++) {
      const envelope = await fixture.call("tools/call", { name: "chariox.room.run_command", arguments: { command, task_id: task.task_id, origin_prompt_id: origin } }, { timeoutMs: 25000, rawEnvelope: true })
      const next = envelope.error?.message?.match(/origin_prompt_id is `([^`]+)`/)
      if (next) { origin = next[1]; continue }
      if (envelope.error) return { ok: false, error: envelope.error.message }
      const value = envelope.result ?? envelope
      const text = value.structuredContent ?? JSON.parse(value.content.find(c => c.type === "text").text)
      return { ok: !value.isError && text.ok !== false, ...text }
    }
    throw new Error("turn authority changed repeatedly")
  }
  note("rooms-ready", { home, bystander, web_project: webRoom.project_id })

  // RC1: the agent creates a Room under the default Project with a name and defaults.
  const rc1 = await roomCommand("room create --name rc-made --provider dev-stub --model room-model --effort low --account-profile default")
  const createdId = rc1.response?.room?.id
  const made1 = (await rooms()).find(room => room.id === createdId)
  check("RC1-room-created-like-web", rc1.ok && !!made1 && made1.alias === "rc-made" && JSON.stringify(made1.project_id) === JSON.stringify(webRoom.project_id)
    && made1.agent_defaults?.provider === "dev-stub" && made1.agent_defaults?.model === "room-model" && made1.agent_defaults?.account_profile === "default",
  { reply: rc1, room: made1 && { id: made1.id, alias: made1.alias, project_id: made1.project_id, agent_defaults: made1.agent_defaults }, web_project_id: webRoom.project_id })
  const listed = await roomCommand("room list")
  check("RC1-listed-for-the-owner", listed.response?.rooms?.some(room => room.id === createdId) === true && (await rooms()).filter(live).some(room => room.id === createdId), { id: createdId, listed })
  check("RC1-creator-recorded", made1?.created_by_agent_id === creator, { created_by_agent_id: made1?.created_by_agent_id, creator })

  // RC2: the same agent starts an agent in that Room under an account profile, and the agent answers.
  const rc2 = await roomCommand(`agent spawn rc-worker --provider dev-stub --model room-model --account-profile default --room ${createdId} --task "RC2-PING"`)
  let worker
  for (let i = 0; i < 120 && !worker; i++) {
    worker = ((await rooms()).find(room => room.id === createdId)?.agents ?? []).find(agent => agent.alias === "rc-worker")
    if (!worker) await delay(500)
  }
  let answered = ""
  for (let i = 0; i < 120 && !answered.includes("RC2-PING"); i++) {
    const outline = (await rpc({ GetSessionHistoryOutline: { session_id: createdId, agent_ids: worker ? [worker.id] : undefined, latest_prompt_count: 3, include_prefetched_content: true } })).SessionHistoryOutline
    answered = JSON.stringify(outline)
    if (!answered.includes("RC2-PING")) await delay(1000)
  }
  check("RC2-agent-in-new-room-prompted-and-answers", rc2.ok && !!worker && answered.includes("RC2-PING"),
    { spawn: rc2, worker_fields: worker && Object.keys(worker), worker: worker && { id: worker.id, alias: worker.alias, provider: worker.provider, profile: worker.provider_account_profile ?? worker.account_profile ?? null }, outline_head: answered.slice(0, 300) })

  // RC3 / RC6: it deletes the Room it made; Rooms it did not make, and session attach/use/delete, stay refused.
  const scratch = await roomCommand("room create --name rc-scratch --provider dev-stub --model room-model --account-profile default")
  const scratchId = scratch.response?.room?.id
  const deleted = await roomCommand(`room delete ${scratchId}`)
  const afterOwn = await rooms()
  check("RC3a-own-room-deleted", scratch.ok && deleted.ok && !afterOwn.filter(live).some(room => room.id === scratchId), { deleted, rooms: afterOwn.map(room => ({ alias: room.alias, status: room.status })) })
  const refusedDelete = await roomCommand(`room delete ${bystander}`)
  const refusedHome = await roomCommand(`room delete ${home}`)
  const afterOther = await rooms()
  check("RC3b-other-room-refused-and-untouched", !refusedDelete.ok && /owner/i.test(refusedDelete.error ?? "") && !refusedHome.ok && /owner/i.test(refusedHome.error ?? "")
    && [bystander, home].every(id => afterOther.filter(live).some(room => room.id === id)), { refusedDelete, refusedHome })
  const denied = []
  for (const command of [`session attach ${bystander}`, `session use ${bystander}`, `session delete ${bystander}`]) denied.push({ command, ...(await roomCommand(command)) })
  check("RC6-attach-use-delete-denied", denied.every(r => !r.ok && /cannot attach to, switch, or delete sessions/.test(r.error ?? "")) && (await rooms()).filter(live).some(room => room.id === bystander), { denied })

  // RC5: the created Room, its creator record and its agent survive a kernel restart.
  await stop()
  await start("kernel-restarted")
  const restarted = (await rooms()).find(room => room.id === createdId)
  check("RC5-room-and-agent-survive-restart", !!restarted && live(restarted) && restarted.created_by_agent_id === creator && restarted.alias === "rc-made" && restarted.agent_defaults?.account_profile === "default"
    && (restarted.agents ?? []).some(agent => agent.alias === "rc-worker"), { room: restarted && { id: restarted.id, alias: restarted.alias, created_by_agent_id: restarted.created_by_agent_id, agents: (restarted.agents ?? []).map(agent => agent.alias) } })
}
try { await main() } catch (error) { check("drill-completed", false, { message: String(error.message).slice(0, 600) }); process.exitCode = 1 }
finally {
  await stop().catch(() => {})
  rmSync(root, { recursive: true, force: true })
  writeFileSync(join(evidence, "cleanup.json"), JSON.stringify({ pid: child?.pid, exit_code: child?.exitCode, signal: child?.signalCode, state_removed: !existsSync(root) }, null, 2))
  note("cleanup-finished", { root })
  if (checks.some(c => c.status === "FAIL")) process.exitCode = 1
}

// MP-08 / MP-09 / MP-10 / MP-11 orchestrate-one-room: a real cheap orchestrator agent in Room R on kernel A runs a
// team inside R with only its own tools (room commands, agent messages, events, chariox-reach): it spawns two workers,
// one asked onto kernel B, gives them tasks and deadlines, follows them, answers a decision, steers one and collects
// their files. The driver never helps unless a step stalls; it records what the orchestrator did, the wall time and
// the model turns and tokens each wake cost. Findings drill: every step is reported, none is asserted to pass.
import { spawn, execFileSync } from "node:child_process"
import http from "node:http"
import { createHmac, createHash, randomBytes, randomUUID } from "node:crypto"
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, statSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { resolve, join, dirname } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { startOwnerCloud } from "./local-owner-cloud.test-support.mjs"
import { hiddenBrief, runHidden } from "./hidden-subagents-scenario.mjs"
import { kindsBrief, orchBrief, decisionsTwoBrief, kindsDriver, sudoProbe, lateParentAnswer, controlProbes } from "./subagents-landing5-scenarios.mjs"
import { listDescendantsRequest } from "../../packages/kernel-client/dist/ipc-agent-requests.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
const binary = need("--kernel-binary"), relayBinary = need("--relay-binary"), stateRoot = need("--state-root"), evidence = need("--evidence-dir")
const provider = flags.get("--provider") ?? "claude"
const codexProfilePath = provider === "codex" ? need("--codex-profile-path") : null
const model = flags.get("--model") ?? (provider === "codex" ? "gpt-6-luna" : "claude-haiku-5-5")
const budgetMs = Number(flags.get("--budget-seconds") ?? 1800) * 1000
const tokenFile = "/root/.claude-lanes/oauth-token"
const controlPath = flags.get("--control-file") ? resolve(flags.get("--control-file")) : null
if (!stateRoot.startsWith("/root/.chariox/dev/")) throw new Error("state must be lane-owned")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const root = mkdtempSync(join(stateRoot, "orch-"))
const children = [], clients = [], steps = []
const t0 = Date.now()
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), s: Math.round((Date.now() - t0) / 1000), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 1) + "\n", { mode: 0o600 })
  console.log(`MP-08/09/10/11 +${Math.round((Date.now() - t0) / 1000)}s ${name} ${JSON.stringify(data).slice(0, 300)}`)
}
const redact = text => String(text).replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")
const env = { PATH: process.env.PATH, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash" }
const secret = randomBytes(32).toString("base64url"), issuer = `orch-${randomUUID()}`
function token(subject, kind, actions, thumbprint, targets) {
  const head = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const body = Buffer.from(JSON.stringify({ iss: issuer, sub: subject, subject_kind: kind, realm_id: issuer, allowed_actions: actions,
    allowed_targets: targets ?? null, ...(thumbprint ? { public_key_thumbprint: thumbprint } : {}), iat: Math.floor(Date.now() / 1000) - 1, exp: Math.floor(Date.now() / 1000) + 7200, jti: randomUUID(), user_id: "local" })).toString("base64url")
  return `${head}.${body}.${createHmac("sha256", secret).update(`${head}.${body}`).digest("base64url")}`
}
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
async function until(check, what, timeout = 30000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await check()) return; await delay(500) }
  throw new Error(`timeout: ${what}`)
}
function start(program, extra, name) {
  const log = openSync(join(evidence, `${name}.log`), "a", 0o600)
  const child = spawn(program, [], { env: { ...env, ...extra }, stdio: ["ignore", log, log] })
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("bad pid")
  children.push(child)
  note("process-start", { label: name, pid: child.pid })
  return child
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("invalid owned PID")
  child.kill("SIGTERM")
  try { await until(() => child.exitCode !== null || child.signalCode !== null, `PID ${child.pid} exits`, 30000) } catch {
    child.kill("SIGKILL")
    await until(() => child.exitCode !== null || child.signalCode !== null, `PID ${child.pid} exits after kill`, 5000)
  }
  note("process-exit", { pid: child.pid, code: child.exitCode, signal: child.signalCode })
}
async function role(name, extraEnv = {}) {
  const dir = join(root, name), ports = []
  for (let i = 0; i < 4; i++) ports.push(await freePort())
  for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[workflow]\nsession_default_max_agents = ${flags.get("--max-agents") ?? 1024}\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
  const localEnv = { ...env, HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join("/var/tmp/rch", `orch-${name}-${process.pid}.sock`), CHARIOX_DAEMON_ALIAS: `orch-${name}`, CHARIOX_MACHINE_ALIAS: `orch-${name}-machine`,
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]), ...extraEnv }
  mkdirSync("/var/tmp/rch", { recursive: true })
  const r = { name, dir, env: localEnv, passkey: randomBytes(24).toString("hex"), mcpPort: ports[1] }
  r.url = `ws://127.0.0.1:${ports[0]}/kernel`
  r.child = start(binary, localEnv, name)
  r.client = new LocalIpcClient(r.url, { localAuthEnvironment: localEnv, controlRequestRetryDeadlineMs: 1000 })
  clients.push(r.client)
  r.rpc = async request => {
    const result = await r.client.send(request)
    if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code} ${result.Error.message ?? ""}`)
    return result
  }
  await until(async () => { try { return !!(await r.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, `${name} ready`, 60000)
  r.status = (await r.rpc({ RelayStatus: null })).RelayStatus.status
  note("kernel-ready", { role: name, kernel: r.status.daemon_id })
  return r
}
// The same kernel home started again: what a kernel upgrade or crash recovery does to a Room in the middle of its work.
async function restart(r) {
  await r.client.close().catch(() => {})
  await stop(r.child)
  r.child = start(binary, r.env, `${r.name}-restarted`)
  r.client = new LocalIpcClient(r.url, { localAuthEnvironment: r.env, controlRequestRetryDeadlineMs: 1000 })
  clients.push(r.client)
  await until(async () => { try { return !!(await r.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, `${r.name} ready again`, 90000)
  note("kernel-restarted", { role: r.name })
}
const state = async (r, session) => (await r.rpc({ GetSessionState: { session_id: session } })).SessionState.session
async function provision(r, oauth) {
  const made = (await r.rpc({ CreateSession: { workspace_id: join(r.dir, "work"), worktree_id: join(r.dir, "work"), alias: "vault-room" } })).SessionCreated
  const att = (await r.rpc({ AttachToSession: { session_id: made.session.id, client_id: "orch-setup", capability_level: "FullTerminal" } })).SessionAttached
  await r.client.subscribeToKernelEvents(made.session.id, att.attachment.id)
  const pending = r.rpc({ ManageCredentialVault: { session_id: made.session.id, agent_id: made.agent.id } })
  pending.catch(() => {})
  let interaction
  await until(async () => (interaction = (await state(r, made.session.id)).active_interactions?.find(x => x.id.startsWith("vault-unlock-"))), "vault prompt")
  await r.rpc({ RespondToInteraction: { session_id: made.session.id, interaction_id: interaction.id, choice_id: "passphrase", custom_reply: r.passkey } })
  await pending
  await r.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: oauth, overwrite: true } })
  note("claude-profile-stored", { kernel: r.name })
}
// The orchestrator's Claude transcript: one JSONL per provider session; turns are split at each real user prompt.
function transcripts(home) {
  const files = []
  const walk = dir => { for (const e of readdirSync(dir, { withFileTypes: true })) { const f = join(dir, e.name); if (e.isDirectory()) walk(f); else if (e.name.endsWith(".jsonl")) files.push(f) } }
  try { walk(join(home, ".claude/projects")) } catch {}
  return files.map(file => ({ file, lines: readFileSync(file, "utf8").split("\n").flatMap(l => { try { return [JSON.parse(l)] } catch { return [] } }) }))
}
function turnStats(lines) {
  const turns = []
  let cur = null
  for (const e of lines) {
    const content = e.message?.content
    const isPrompt = e.type === "user" && (typeof content === "string" || (Array.isArray(content) && content.some(p => p.type === "text")))
    if (isPrompt) { cur = { at: e.timestamp, prompt: redact(typeof content === "string" ? content : content.find(p => p.type === "text").text).slice(0, 160), assistant_messages: 0, tool_calls: [], input: 0, cache_read: 0, cache_write: 0, output: 0 }; turns.push(cur); continue }
    if (!cur || e.type !== "assistant") continue
    cur.assistant_messages++
    const u = e.message?.usage ?? {}
    cur.input += u.input_tokens ?? 0; cur.cache_read += u.cache_read_input_tokens ?? 0; cur.cache_write += u.cache_creation_input_tokens ?? 0; cur.output += u.output_tokens ?? 0
    for (const p of Array.isArray(content) ? content : []) if (p.type === "tool_use") cur.tool_calls.push(p.name.replace(/^mcp__/, ""))
  }
  return turns
}
// Codex rollouts written under the linked profile during this run: one entry per user turn (prompt start, tool calls, tokens).
function codexTurns(profileDir, sinceMs) {
  const files = []
  const walk = dir => { for (const e of readdirSync(dir, { withFileTypes: true })) { const f = join(dir, e.name); if (e.isDirectory()) walk(f); else if (e.name.startsWith("rollout-") && e.name.endsWith(".jsonl") && statSync(f).mtimeMs >= sinceMs) files.push(f) } }
  try { walk(join(profileDir, "sessions")) } catch {}
  return files.map(file => {
    const turns = []
    let cur = null, model = null
    for (const l of readFileSync(file, "utf8").split("\n")) {
      let e; try { e = JSON.parse(l) } catch { continue }
      const p = e.payload ?? {}
      if (p.model) model = p.model
      if (e.type === "response_item" && p.type === "message" && p.role === "user") {
        const text = redact((p.content ?? []).map(c => c.text ?? "").join(" "))
        if (/^<(environment_context|user_instructions|permissions)/.test(text.trim())) continue
        cur = { at: e.timestamp, prompt: text.slice(0, 160), tool_calls: [], output_tokens: 0, total_tokens: 0 }; turns.push(cur)
      } else if (cur && e.type === "response_item" && (p.type === "function_call" || p.type === "custom_tool_call")) cur.tool_calls.push({ name: p.name, input: redact(String(p.arguments ?? p.input ?? "")).slice(0, 260) })
      else if (cur && e.type === "event_msg" && p.type === "token_count" && p.info?.total_token_usage) { cur.output_tokens = p.info.total_token_usage.output_tokens ?? 0; cur.total_tokens = p.info.total_token_usage.total_tokens ?? 0 }
    }
    return { file: file.split("/").pop(), model, turns }
  })
}
function toolLog(lines) {
  const rows = []
  for (const e of lines) for (const p of Array.isArray(e.message?.content) ? e.message.content : []) {
    if (p.type === "tool_use") rows.push({ at: e.timestamp, call: p.name, input: p.input })
    else if (p.type === "tool_result") rows.push({ at: e.timestamp, answer: redact(typeof p.content === "string" ? p.content : JSON.stringify(p.content)).slice(0, 900) })
    else if (p.type === "text" && e.message.role === "assistant") rows.push({ at: e.timestamp, said: p.text.slice(0, 400) })
  }
  return rows
}

async function main() {
  note("binaries", { kernel: createHash("sha256").update(readFileSync(binary)).digest("hex"), relay: existsSync(relayBinary) ? createHash("sha256").update(readFileSync(relayBinary)).digest("hex") : null })
  const relayPort = await freePort()
  mkdirSync(join(root, "relay"), { mode: 0o700 })
  if (flags.get("--single") !== "yes") start(relayBinary, { HOME: root, CHARIOX_HOME: join(root, "relay"), CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_SCOPED_ISSUER: issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }, "relay")
  const single = flags.get("--single") === "yes"
  // --owner-profile: the kernels start the way a managed kernel does, with their owner's profile in the environment and a loopback stand-in for the few Cloud endpoints
  // they use (local-owner-cloud.test-support.mjs), so peer agents of other kernels act for the one owner of the local relay's realm.
  const profileEnv = flags.get("--owner-profile") === "yes" ? await startOwnerCloud({ token, note, realm: issuer, relayPort }) : {}
  const a = await role("a", profileEnv), b = single ? { name: "b", dir: join(root, "b"), status: { daemon_id: "single-kernel" }, rpc: async () => ({}) } : await role("b", profileEnv)
  if (!single) for (const w of ["w1", "w2", "w3", "w4", "w5"]) mkdirSync(join(root, "b/work", w), { recursive: true })
  const owned = flags.get("--owner-profile") === "yes"
  if (!single && !owned) for (const r of [a, b]) {
    await r.rpc({ ConfigureRelay: { relay_url: `ws://127.0.0.1:${relayPort}`, relay_token: token(r.status.daemon_id, "kernel", ["daemon.register", "daemon.heartbeat", "packet.route", "peer.request", "peer.event", ...(flags.get("--kernel-metadata") === "yes" ? ["client.metadata.read"] : [])]) } })
    await until(async () => (await r.rpc({ RelayStatus: null })).RelayStatus.status.connected, `${r.name} relay registration`)
  }
  if (!single && owned) for (const r of [a, b]) await until(async () => (await r.rpc({ RelayStatus: null })).RelayStatus.status.connected, `${r.name} relay registration`, 60000)
  if (!single && !owned) {
  const pairing = (await b.rpc({ CreateTerminalPairingLink: { terminal_type: "cli", expires_in_ms: 300000 } })).TerminalPairingLinkCreated.pairing
  await a.rpc({ JoinKernelLink: { link: pairing.pairing_link, relay_token: token("orch-operator", "client", ["client.connect", "client.metadata.read", "packet.route"]) } })
  note("paired", { from: "a", to: "b", b: b.status.daemon_id })
  }
  let profile = "default", profileB = "default"
  if (provider === "codex") {
    // The profile directory already holds a product login; each kernel links it itself (its registry is its own) and never reads the credential.
    const link = async kernel => {
      const linked = (await kernel.rpc({ LinkProviderAccountProfile: { provider: "codex", label: "orch-codex", path: codexProfilePath } })).ProviderAccountProfile.profile
      await kernel.rpc({ RefreshProviderAccountProfile: { provider: "codex", account_profile: linked.profile_id } })
      return linked.profile_id
    }
    profile = await link(a)
    profileB = single ? profile : await link(b)
    note("codex-profile-linked", { profile, profileB })
  } else {
    const oauth = readFileSync(tokenFile, "utf8").trim()
    await provision(a, oauth)
    if (!single) await provision(b, oauth)
  }
  // The project: a git repository the workers branch their own worktrees from.
  const work = join(a.dir, "work")
  const git = (...args) => execFileSync("git", ["-C", work, ...args], { env: { ...process.env, GIT_AUTHOR_NAME: "drill", GIT_AUTHOR_EMAIL: "drill@example.invalid", GIT_COMMITTER_NAME: "drill", GIT_COMMITTER_EMAIL: "drill@example.invalid" } }).toString()
  git("init", "-q", "-b", "main"); writeFileSync(join(work, "README.md"), "orchestrate-one-room drill project\n"); git("add", "README.md"); git("commit", "-q", "-m", "start")
  if (!single) mkdirSync(join(b.dir, "work/w2"), { recursive: true })
  const defaults = { provider, account_profile: profile, model, effort: "low", permission_level: "yolo" }
  const room = (await a.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "team-room", agent_defaults: defaults } })).SessionCreated
  const R = room.session.id, orch = room.agent.id
  const attached = (await a.rpc({ AttachToSession: { session_id: R, client_id: "orch-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  await a.client.subscribeToKernelEvents(R, attached)
  await a.rpc({ LaunchProviderRun: { session_id: R, agent_id: orch, adapter_key: provider, provider, account_profile: profile, model, variant: "low", native_tui: false } })
  note("room-ready", { room: R, orchestrator: orch })
  // A person's own second agent: not in the orchestrator's tree, the peer of the tree drill.
  if (["authority", "messages", "decisions"].includes(flags.get("--scenario"))) await a.rpc({ SpawnAgent: { session_id: R, alias: "peer", provider, account_profile: profile, model, effort: "low", permission_level: "yolo", worktree_id: null } }).then(() => note("peer-created"), e => note("peer-error", { message: e.message.slice(0, 300) }))

  const deadline = Date.now() + 25 * 60 * 1000
  const scenario = flags.get("--scenario") ?? "team"
  if (scenario === "hidden") {
    writeFileSync(join(work, "BRIEF.md"), hiddenBrief({ model, work }))
    const verdicts = await runHidden({ a, R, orch, attached, model, work, note, until, restart, evidence, kill: flags.get("--hidden-kill") !== "no" })
    if (!verdicts.every(v => v.ok)) process.exitCode = 1
    return
  }
  // The driver follows R through the outside-events inbox too, to count what a Room with several agents emits.
  await a.rpc({ SubscribeEvents: { targets: [{ kernel: a.status.daemon_id, session_id: R }] } }).then(() => note("driver-events-subscribed"), e => note("driver-events-refused", { message: e.message.slice(0, 200) }))
  const restartBrief = `# Orchestrator brief (drill, restart scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Run a team inside THIS Room using only your own Chariox tools. Do not ask the owner anything.
Never retry a refused call more than once; record the exact refusal instead. Never use shell sleeps or poll in a loop.
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider ${provider} --model ${model} --effort low --worktree ${join(work, "wt")}/wN --branch wN\` for N = 1, 2, 3.
2. Prompt them (room command \`prompt <alias> "<text>"\` (not \`agent prompt\`)):
   w1: "Write the single word ALPHA into result.txt in your current working directory. End your reply with W1DONE."
   w2: "Write the single word BETA into result.txt in your current working directory. End your reply with W2DONE."
   w3: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w3 hold', message 'Continue?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns, write the single word GAMMA into result.txt in your current working directory. End your reply with W3DONE."
Keep ${join(work, "orchestrator-state.json")} updated after every step (what is done, what you wait for) so you can recover if you lose your context.
3. Subscribe to each worker (chariox events_subscribe with the worker agent id) and yield on ALL your registrations (chariox events_subscriptions lists them; a yield must cover every open obligation) with a deadline 20 minutes ahead, then END YOUR TURN. The kernel wakes you with a new turn per event.
4. On the FIRST wake, steer w3 while it is still busy: chariox send_agent_message to w3 with urgent true: "When you continue, also write the word DELTA into steer.txt in your working directory." Then ack the event and yield again on all registrations.
5. On each later wake: ack, check which workers are done (room \`agent list\` or list_session_agents) and yield again until all three are done.
6. Collect the results with your own shell: read result.txt (and steer.txt) from each worker worktree under ${join(work, "wt")}.
7. Then write ${join(work, "orchestrator-report.json")} with one entry per wake (what woke you, what you did), the step 6 answers, and each worker's result.txt content. Then delete the three workers with \`agent delete wN\` and reply ORCHDONE.
`
  const workerDeadline = Date.now() + 240000
  // Orchestration WITHOUT /sudo: every control below is a room command or an events tool of an ordinary room agent.
  const sudoBrief = `# Orchestrator brief (drill, no-sudo scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). There is NO /sudo and no kernel_request: use only your room commands and the chariox events tools.
Do not ask the owner anything except in step 8. Never retry a refused call more than once; record the exact answer. Never use shell sleeps or poll in a loop.
1. Spawn w1, w2, w3 with room commands \`agent spawn wN --provider claude --model ${model} --effort low --worktree ${join(work, "wt")}/wN --branch wN --permission yolo --command-id spawn-wN\`. Spawn w1 a SECOND time with the identical command and record whether you got the same agent id.
2. Prompt them with the room command \`prompt <alias> "<text>"\` (not \`agent prompt\`):
   w1: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w1 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns write ALPHA into result.txt in your working directory. End with W1DONE."
   w2: "Write BETA into result.txt in your working directory. End with W2DONE."
   w3: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w3 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns write GAMMA into result.txt. End with W3DONE."
3. Subscribe to each worker (chariox events_subscribe with the worker agent id).
4. Run \`agent deadline w1 3m\` and then \`agent deadline get w1\`. Run \`agent children\`.
5. Run \`agent profiles\` and \`agent usage\`.
6. Stop w3's turn without deleting it: \`agent cancel w3\`.
7. Steer w1 while it is still running: chariox send_agent_message to w1 with urgent true: "Also write the word DELTA into steer.txt in your working directory."
8. Ask the owner a question WITHOUT blocking: chariox request_popup with wait false, title 'owner question', message 'May I merge w2?', choices [{id:'merge',label:'Merge',reply:'merge'},{id:'hold',label:'Hold',reply:'hold'}].
9. Start a heartbeat: chariox events_timer with interval_ms 60000 and label 'heartbeat'.
10. Yield on ALL your registrations (chariox events_subscriptions lists them; include the heartbeat timer and the owner question registration) with a deadline 15 minutes ahead and END YOUR TURN. A child's finish arrives as a child_finished event: it wakes you, so do not poll.
11. On each wake: append to ${join(work, "orchestrator-state.json")} the time, what woke you (for child_finished: the outcome kind and deadline_reached) and what you did; ack; when the heartbeat has woken you at least twice, cancel it (chariox events_cancel_wake). Expect: w1's deadline ending w1 (deadline_reached true), w2 finishing, w3's cancel, the owner's answer.
12. When w1, w2 and w3 have each finished: read w2's result.txt with your own shell from ${join(work, "wt/w2")} (it is on your kernel).
13. Write ${join(work, "orchestrator-report.json")}: one entry per step with ok, the command you used and the exact answer, and per wake what woke you. Delete the workers with \`agent delete wN\`. Reply ORCHDONE.
`
  // A heartbeat (30 minutes unless --heartbeat-seconds) and a worker turn that really fails: what wakes a parked orchestrator, and what it can read about the failure.
  const heartbeatMs = Number(flags.get("--heartbeat-seconds") ?? 1800) * 1000
  const heartbeatBrief = `# Orchestrator brief (drill, heartbeat scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your own Chariox tools. Do not ask the owner anything. Never use shell sleeps or poll in a loop.
Keep ${join(work, "orchestrator-state.json")} updated after every step and every wake.
1. Spawn w1 with room command \`agent spawn w1 --provider claude --model claude-no-such-model-1 --effort low --worktree ${join(work, "wt/w1")} --branch w1\` and w2 with \`agent spawn w2 --provider ${provider} --model ${model} --effort low --worktree ${join(work, "wt/w2")} --branch w2\`. Record the answers.
2. Prompt both (room \`prompt\`): "Write the single word DONE into result.txt in your current working directory."
3. Subscribe to both workers (chariox events_subscribe with the agent id) and arm a heartbeat: chariox events_timer with label 'heartbeat', delay_ms ${heartbeatMs} and interval_ms ${heartbeatMs}.
4. Yield on ALL your registrations (events_subscriptions lists them) with a deadline 45 minutes ahead and END YOUR TURN.
5. On each wake: append to the state file the time, what woke you and, for a worker, every failure field you can read (the wake payload, chariox history turn of that worker, get_session_agent): outcome kind, retryable, reason, error text. Ack and yield again on all registrations.
6. On the first heartbeat wake: cancel the heartbeat (events_cancel_wake), write ${join(work, "orchestrator-report.json")} (every wake with its time and cause, and the failure fields of w1), delete both workers (\`agent delete wN\`) and reply ORCHDONE.
`
  // AO-IN-01: three workers finish while the orchestrator's turn is still running, so their completions are pending together.
  // The wake that follows must be ONE turn that reads the three events from the inbox and acknowledges them.
  const batchBrief = `# Orchestrator brief (drill, inbox-batch scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your own Chariox tools. Do not ask the owner anything. Never retry a refused call more than once.
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider ${provider} --model ${model} --effort low --worktree ${join(work, "wt")}/wN --branch wN\` for N = 1, 2, 3.
2. Prompt each (room \`prompt\` command): "Write the single word ALPHA (w1) / BETA (w2) / GAMMA (w3) into result.txt in your current working directory. End your reply with W1DONE / W2DONE / W3DONE."
3. Subscribe to each worker (chariox events_subscribe with the worker agent id).
4. Hold this turn with ONE shell command: \`sleep 150\`. The three workers finish while it runs.
5. Then END YOUR TURN with your final answer WAITING. Do not poll.
6. When the kernel wakes you, its prompt only names counts and kinds. Call chariox events_inbox ONCE and read every event, then call chariox events_ack ONCE with through = the last sequence you read and handled true.
7. Write ${join(work, "orchestrator-report.json")} with: what woke you, how many events your inbox read returned, the ack answer. Delete the workers (\`agent delete wN\`) and reply ORCHDONE.
`
  // The same three workers, but the orchestrator parks (yield) and a barrier file releases the workers together: the
  // first completion wakes the parked orchestrator; every completion pending when that wake is built rides with it.
  const goPath = join(root, "go")
  const batchYieldBrief = `# Orchestrator brief (drill, inbox-batch-yield scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your own Chariox tools. Do not ask the owner anything. Never retry a refused call more than once.
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider ${provider} --model ${model} --effort low --worktree ${join(work, "wt")}/wN --branch wN\` for N = 1, 2, 3.
2. Prompt each (room \`prompt\` command): "Run the shell command: until [ -e ${goPath} ]; do sleep 1; done. Then write the single word ALPHA (w1) / BETA (w2) / GAMMA (w3) into result.txt in your current working directory. End your reply with W1DONE / W2DONE / W3DONE."
3. Subscribe to each worker (chariox events_subscribe with the worker agent id), then yield on ALL your registrations (events_subscriptions lists them) with a deadline 20 minutes ahead and END YOUR TURN. Do not poll and do not read the inbox before you are woken.
4. When the kernel wakes you, its prompt only names counts and kinds. Call chariox events_inbox ONCE, read every event, then ${flags.get("--ack-mode") === "single" ? "call chariox events_ack once for each event with handled true" : "call chariox events_ack ONCE with through = the last sequence you read and handled true"}. If your inbox read shows fewer than three worker completions, yield again on your remaining registrations and repeat this step on the next wake.
5. When all three completions are acknowledged write ${join(work, "orchestrator-report.json")} with: each wake (what woke you, how many events your inbox read returned) and the ack answers. Delete the workers (\`agent delete wN\`) and reply ORCHDONE.
`
  // A02-S02 / A02-S03: a default message to a busy, an idle and a yielded receiver, one reply-requested message, one urgent steer.
  const messagesBrief = `# Orchestrator brief (drill, messages scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your own Chariox tools. Do not ask the owner anything. Never retry a refused call more than once; record the exact answer.
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider ${provider} --model ${model} --effort low --worktree ${join(work, "wt")}/wN --branch wN\` for N = 1, 2, 3. Do not prompt w2.
2. Prompt w1 (room \`prompt\`): "Run the shell command: until [ -e ${goPath} ]; do sleep 1; done. Then write the single word ALPHA into result.txt in your working directory. End your reply with W1DONE." (w1 stays BUSY.)
   Prompt w3: "Arm a timer with the chariox events_timer tool (delay_ms 600000, label w3-wait), then yield on it with chariox events_yield (deadline 15 minutes ahead) and end your turn. When you are woken, read your inbox with chariox events_inbox, do what each message asks as you would for any request from your parent, write the inbox text into result.txt, and end with W3DONE." (w3 is YIELDED once it has parked; wait until room \`agent list\` shows w3 is no longer processing before you send to it.)
3. Send, with chariox send_agent_message (default, not urgent, no reply wanted):
   to w1 (busy): "Also write the word BUSYMSG into msg.txt in your working directory after result.txt."
   to w2 (idle): "Write the word IDLEMSG into msg.txt in your working directory."
   to w3 (yielded): "Also write the word YIELDEDMSG into msg.txt in your working directory."
   Then send to w2 ONE message with reply_requested true: "What is 2 plus 3? Answer with the number as your final answer." Your wake carries its single answer.
   Then send to w1 (still busy) ONE urgent message (urgent true): "Also write the word STEERED into steer.txt in your working directory."
4. Subscribe to w1, w2 and w3 (chariox events_subscribe) and yield on ALL your registrations (events_subscriptions) with a deadline 20 minutes ahead; END YOUR TURN. Do not poll.
5. On each wake: chariox events_inbox once, read the events, events_ack them (handled true), and yield again until w1, w2 and w3 reported. Expect: no courtesy replies for the default messages, one reply for the reply_requested message.
6. Write ${join(work, "orchestrator-report.json")} with each wake (what woke you, events read, acks). Delete the workers (\`agent delete wN\`) and reply ORCHDONE.
`
  // AM-ASK-03: a non-blocking owner question stays pending across a kernel restart and the answer wakes the agent once.
  const askBrief = `# Orchestrator brief (drill, ask scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your own Chariox tools. Do not ask the owner anything in plain text. Never retry a refused call more than once.
1. Call chariox request_popup with wait false, title 'ask-restart', message 'Go on?', choices [{id:'go',label:'Go',reply:'go ahead'}], timeout_sec 900.
2. Yield on its registration (events_yield with the registration id, a deadline 15 minutes ahead, this turn's metadata) and END YOUR TURN with the text ASKARMED. Do not poll.
3. When the kernel wakes you, call chariox events_inbox ONCE, read the answer, call chariox events_ack with handled true, and write ${join(work, "orchestrator-report.json")} with: what woke you, the exact answer text, the ack answer. Then reply ASKDONE.
`
  const authorityBrief = `# Orchestrator brief (drill, authority scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}); your agent id is ${orch}. A person's other agent called "peer" is in the room but is not yours. Use only your room commands (chariox room run_command) and chariox events tools. No /sudo. Do not ask the owner anything. Never retry a refused call more than once: record the exact answer. Never use shell sleeps or poll.
Keep ${join(work, "orchestrator-state.json")} updated after every step.
1. Spawn c1, c2 and m with \`agent spawn <name> --provider claude --model ${model} --effort low --permission yolo\` and p with \`agent spawn p --provider claude --model ${model} --effort low --permission required\` (you run freely, so a child that must ask is allowed; c2 must stay alive and idle). Record each answer.
2. Prompt c1 with the room command \`prompt c1 "<text>"\`: "Run these room commands with the chariox room run_command tool, one by one, and report the exact answer of each as OK or REFUSED followed by the text: (1) agent spawn g1 --provider claude --model ${model} --effort low --permission yolo (2) prompt g1 'Reply with GCHILD' (3) agent cancel c2 (4) agent cancel peer (5) agent cancel ${orch} (6) agent deadline c2 10m. Do not retry a refused command. Do not subscribe or wait for anything. End your reply with C1DONE."
3. Prompt p: "Run these room commands and report each exact answer as OK or REFUSED followed by the text: (1) agent spawn p1 --provider claude --model ${model} --effort low --permission yolo (2) agent spawn p2 --provider claude --model ${model} --effort low --permission required (3) agent delete p2 (4) agent delete p1 if it exists. Do not retry a refused command. Do not subscribe or wait for anything. End with PDONE."
4. Prompt m: "Run these room commands and report each exact answer as OK or REFUSED followed by the text: (1) agent spawn y1 --provider claude --model ${model} --effort low --permission yolo. Then call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 600, title 'lower me', message 'Ready?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns run: (2) agent spawn y2 --provider claude --model ${model} --effort low --permission yolo (3) agent spawn y3 --provider claude --model ${model} --effort low --permission required. (4) agent delete y1, agent delete y2, agent delete y3 for the ones that exist. Report each answer. Do not subscribe or wait for anything. End with MDONE."
4b. Prompt c2: "Reply with the single word C2DONE and nothing else."
5. Subscribe to c1, c2, p and m (chariox events_subscribe with the agent id), yield on ALL registrations with a deadline 15 minutes ahead and END YOUR TURN. Each child's finish arrives as a child_finished event with the child's reply in public_answer; for each one record the commands it ran and the answers in the state file; ack; yield again until c1, c2, p and m have finished.
6. When c1 has finished, YOU control c1's sub-agent g1 (your grandchild): run \`agent children\`, \`agent deadline get g1\`, \`agent cancel g1\` and record the answers (they are allowed).
7. Write ${join(work, "orchestrator-report.json")}: per step the command, the exact answer, and per child the OK/REFUSED lines it reported. Do NOT delete c1, c2, p or m (the person still needs them). Reply ORCHDONE.
`
  // One card, one holder: a worker's decision goes to the parent that may answer it, else to the person as the same interaction.
  const decisionsBrief = `# Orchestrator brief (drill, decisions scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}); your agent id is ${orch}. Use only your room commands (chariox room run_command) and chariox events tools. No /sudo. Do not ask the owner anything. Never retry a refused call more than once: record the exact answer. Never use shell sleeps or poll.
Keep ${join(work, "orchestrator-state.json")} updated after every step.
1. Spawn w1, w2, w3 with \`agent spawn <name> --provider claude --model ${model} --effort low --permission required --worktree ${join(work, "wt")}/<name> --branch <name>\` and p with \`agent spawn p --provider claude --model ${model} --effort low --permission yolo --worktree ${join(work, "wt")}/p --branch p\`. A worker at the required level asks before it acts: that is the point of the drill.
2. FIRST subscribe to w1, w2, w3 and p (chariox events_subscribe with each agent id), THEN prompt them with the room command \`prompt <alias> "<text>"\` (not \`agent prompt\`):
   w1: "Use your shell tool to run exactly: echo ALPHA > result.txt . Then reply W1DONE."
   w2: "Call the chariox request_credential_secret tool with these arguments: credential {id 'drill-key', injection {kind 'header', name 'X-Drill-Key', value '{{secret}}'}, use_binding {target {kind 'site', origin 'https://example.invalid'}}}, prompt {message 'Drill: type any text', timeout_sec 900}. If it is refused for an argument, retry ONCE with injection {kind 'env', name 'DRILL_KEY'}. When it returns reply W2DONE."
   w3: "Use your shell tool to run exactly: echo GAMMA > result.txt . Then reply W3DONE."
   p: "Run these room commands with the chariox room run_command tool and report each exact answer: (1) agent spawn c --provider claude --model ${model} --effort low --permission required --worktree ${join(work, "wt")}/c --branch c (2) prompt c 'Use your shell tool to run exactly: echo DELTA > result.txt . Then reply CDONE.' Then END YOUR TURN at once with PDONE: do not wait for c and do not subscribe. IMPORTANT: if you are woken later by a child_decision event, you must NOT answer it or pass it and you must call no tool: reply PDONE2 only."
3. Yield on your registrations (events_subscriptions lists them) with a deadline 15 minutes ahead and END YOUR TURN. If events_yield is refused, do not retry: just END YOUR TURN, because every event wakes you anyway. Each worker decision that you may answer arrives as an urgent child_decision event (payload: interaction_id, child_agent_id, child_alias, question, choices, needs_answer).
4. On each wake: read the events (chariox events_inbox). For every child_decision event run the room command \`agent answer <interaction_id> <id of the choice that allows the action>\` (take the choice ids from the event; the one meaning allow) and append the interaction_id, the child_alias, the time and the EXACT typed answer to the state file. The answer may come back as settled_by_user or not_holder: record it and go on, never retry. Ack the events; for a child_finished event record the child and the outcome. After you answered a decision of c (p's child), subscribe to c with the child_agent_id of the event. Keep waking until you have seen the child_finished of w1, w2, w3, p AND c. Do NOT write the report before that.
5. Then write ${join(work, "orchestrator-report.json")}: one entry per child_decision event (interaction_id, child_alias, answer status) and one per child_finished event (child, outcome kind). Note whether you ever received a child_decision for w2. Then delete w1, w2, w3, p with \`agent delete <name>\` and reply ORCHDONE.
`
  const payPort = scenario === "kinds" ? await freePort() : 0
  if (payPort) http.createServer((_q, r) => r.end("ok")).listen(payPort, "127.0.0.1").unref()
  let otherRoom = { R2: "", q: "" }
  if (flags.has("--other-room")) {
    const host = flags.get("--other-room-kernel") === "b" && !single ? b : a
    const hostWork = host === b ? join(b.dir, "work") : work
    const hostProfile = host === b ? profileB : profile
    const other = (await host.rpc({ CreateSession: { workspace_id: hostWork, worktree_id: hostWork, alias: "other-room", agent_defaults: { provider, account_profile: hostProfile, model, effort: "low", permission_level: "yolo" } } })).SessionCreated
    const att2 = (await host.rpc({ AttachToSession: { session_id: other.session.id, client_id: "orch-owner-r2", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    await host.client.subscribeToKernelEvents(other.session.id, att2)
    await host.rpc({ LaunchProviderRun: { session_id: other.session.id, agent_id: other.agent.id, adapter_key: provider, provider, account_profile: hostProfile, model, variant: "low", native_tui: false } })
    otherRoom = { R2: other.session.id, q: other.agent.id }
    note("other-room", { room: otherRoom.R2, agent: otherRoom.q })
    // The person gives the other Room's agent its own work, so a peer sees a turn it can follow and cancel.
    if (flags.has("--other-room-prompt")) await host.rpc(submitPromptRequest(otherRoom.R2, att2, otherRoom.q, flags.get("--other-room-prompt"), [])).then(() => note("other-room-prompted"), e => note("other-room-prompt-refused", { message: e.message.slice(0, 200) }))
  }
  const briefFile = flags.get("--brief-file")
  const brief = briefFile ? readFileSync(briefFile, "utf8").replaceAll("{{R}}", R).replaceAll("{{WORK}}", work).replaceAll("{{MODEL}}", model).replaceAll("{{ORCH}}", orch).replaceAll("{{ROOT}}", root).replaceAll("{{Q}}", otherRoom.q).replaceAll("{{R2}}", otherRoom.R2).replaceAll("{{B}}", b.status.daemon_id).replaceAll("{{GO}}", goPath).replaceAll("{{CTL}}", dirname(controlPath ?? join(root, "ctl"))) : scenario === "decisions" ? decisionsTwoBrief({ R, model, work }) : scenario === "kinds" ? kindsBrief({ R, model, work, payPort }) : scenario === "orch" ? orchBrief({ R, model, work }) : scenario === "decisions" ? decisionsBrief : scenario === "authority" ? authorityBrief : scenario === "ask" ? askBrief : scenario === "messages" ? messagesBrief : scenario === "batchyield" ? batchYieldBrief : scenario === "batch" ? batchBrief : scenario === "heartbeat" ? heartbeatBrief : scenario === "restart" ? restartBrief : scenario === "sudo" ? sudoBrief : `# Orchestrator brief (drill)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Run a small team inside THIS Room using only the tools of an ordinary room agent:
the room commands (chariox room run_command), list_session_agents, send_agent_message, history and the chariox events tools (subscribe, yield, inbox, ack). There is NO /sudo.
Do not ask the owner anything. Never retry a refused call more than once; record the exact refusal code and message instead.
Kernel B id: ${b.status.daemon_id}. Never use shell sleeps and never poll: a child's finish arrives as a child_finished event that wakes you.

Steps:
1. Run the room commands \`agent profiles\` and \`agent usage\` and record the answers.
2. Spawn worker w1 with its OWN git worktree: \`agent spawn w1 --provider claude --model ${model} --effort low --worktree ${join(work, "wt/w1")} --branch w1 --permission yolo --account-profile default --deadline 20m --command-id spawn-w1\`. Run the identical command a second time and record whether the same agent id came back.
3. Spawn worker w2 (same flags, worktree ${join(work, "wt/w2")}, branch w2) and worker w4 (worktree ${join(work, "wt/w4")}, branch w4). Spawn worker w3 with the model claude-no-such-model-1 (worktree ${join(work, "wt/w3")}, branch w3).
4. Spawn w5 on kernel B: \`agent spawn w5 --provider claude --model ${model} --effort low --kernel ${b.status.daemon_id} --dir ${join(b.dir, "work/w5")}\`. Record the exact answer, even a refusal. Then go on: a refused spawn must not stop you from yielding in step 7.
5. Give tasks with the room command \`prompt <alias> "<text>"\` (not \`agent prompt\`):
   w1: "Write the single word ALPHA into result.txt in your current working directory. End your reply with W1DONE."
   w2: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w2 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns write BETA into result.txt in your current working directory. End your reply with W2DONE."
   w3: "Reply with the word hello."
   w4: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w4 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns write DELTA into result.txt. End your reply with W4DONE."
6. Run \`agent deadline w2 90s\` and \`agent deadline get w2\`, then \`agent children\`. Record the answers.
7. Subscribe to each worker (chariox events_subscribe with the worker agent id), then yield on ALL your registrations (events_subscriptions lists them) with a deadline 15 minutes ahead and END YOUR TURN. The kernel wakes you with a new turn per event.
8. In each wake read the event (chariox events_inbox): for a child_finished event note the child, outcome.kind, success and deadline_reached. When w4 is still running after the first wake, steer it (send_agent_message to w4, urgent true: "Also write the word GAMMA into steer.txt") and then stop its turn with \`agent cancel w4\`. Ack and yield again on all registrations until w1, w2, w3 and w4 have each finished.
9. Read w1's result.txt with your own shell from ${join(work, "wt/w1")} (the worker is on your kernel).
10. Write ${join(work, "orchestrator-report.json")}: one entry per step 1-9 with ok true/false, the command you used and the exact answer, and one entry per child_finished event (child, outcome kind, deadline_reached). Then delete the workers with \`agent delete wN\`, record those answers too, and reply ORCHDONE.
`
  writeFileSync(join(work, "BRIEF.md"), brief)
  let restarted = false
  const since = Date.now()
  {
    await a.rpc(submitPromptRequest(R, attached, orch, `Read ${join(work, "BRIEF.md")} and carry it out now.`, []))
  }
  note("brief-submitted", { deadline })

  // Follow R like an outside driver would, recording every change; the owner answers a stalled decision only as a fallback.
  const seen = { agents: new Map(), interactions: new Map(), prompts: new Map(), shown: new Map() }
  const orchTurns = []
  const treeNoted = new Set()
  const raceCards = new Map()
  let peerProbed = false, lateDone = false, sudoAt = null, probes = null, probesAt = null
  let bRestartAt = null, remoteSeenAt = null, reportAt = null, fallbackAnswers = 0, peerSentAt = null, goAt = null, idleSince = null, fenceProbeAt = null, fenceProbeDone = false
  const reportPath = join(work, "orchestrator-report.json")
  let attachedNow = attached
  // An outside observer drives faults: one JSON line per op ({"op":"stop|cont|kill|start","kernel":"a|b"}), run in order.
  let controlRead = 0
  const controlOps = async () => {
    if (!controlPath || !existsSync(controlPath)) return
    const lines = readFileSync(controlPath, "utf8").split("\n").filter(Boolean)
    for (; controlRead < lines.length; controlRead++) {
      const op = JSON.parse(lines[controlRead]), r = op.kernel === "b" ? b : a
      if (op.op === "stop" || op.op === "cont") r.child.kill(op.op === "stop" ? "SIGSTOP" : "SIGCONT")
      else if (op.op === "kill") {
        await r.client.close().catch(() => {})
        r.child.kill("SIGKILL")
        await until(() => r.child.exitCode !== null || r.child.signalCode !== null, `${r.name} exits after kill`, 10000)
      } else if (op.op === "start") {
        r.child = start(binary, r.env, `${r.name}-started`)
        r.client = new LocalIpcClient(r.url, { localAuthEnvironment: r.env, controlRequestRetryDeadlineMs: 1000 })
        clients.push(r.client)
        await until(async () => { try { return !!(await r.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, `${r.name} ready again`, 90000)
        if (r === a) {
          attachedNow = (await a.rpc({ AttachToSession: { session_id: R, client_id: "orch-owner-ctl", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
          await a.client.subscribeToKernelEvents(R, attachedNow)
        }
      } else { note("control-refused", op); continue }
      note("control", { ...op, pid: r.child.pid })
    }
  }
  while (Date.now() - since < budgetMs) {
    await controlOps()
    await a.rpc({ PumpTerminalOutput: { session_id: R, attachment_id: attachedNow } }).catch(() => {})
    const s = await state(a, R).catch(() => null)
    if (s) {
      for (const ag of s.agents ?? []) {
        const view = JSON.stringify({ alias: ag.alias, state: ag.state, processing: ag.is_processing, kernel: ag.kernel_ref ?? ag.remote_kernel_id ?? ag.location ?? null, worktree: ag.worktree_id ?? null })
        if (seen.agents.get(ag.id) !== view) { seen.agents.set(ag.id, view); note("agent", { id: ag.id, ...JSON.parse(view) }) }
        const active = s.prompt_states?.[ag.id]?.active_prompt
        const pid = active ? (active.id ?? active.prompt_id ?? JSON.stringify(active).slice(0, 60)) : null
        if (seen.prompts.get(ag.id) !== pid) {
          seen.prompts.set(ag.id, pid)
          note("prompt", { agent: ag.id, alias: ag.alias, active: pid })
          if (ag.id === orch && pid) orchTurns.push({ started_s: Math.round((Date.now() - since) / 1000), prompt: pid })
        }
      }
      for (const it of s.active_interactions ?? []) {
        if (!seen.interactions.has(it.id)) { seen.interactions.set(it.id, Date.now()); note("interaction-open", { id: it.id, agent: it.agent_id, title: it.title, choices: (it.choices ?? []).map(c => c.id) }) }
        else if (/w1 commit/.test(it.title ?? "") && Date.now() - seen.interactions.get(it.id) > Number(flags.get("--owner-fallback-seconds") ?? 300) * 1000) {
          await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: "yes" } }).then(() => { fallbackAnswers++; note("owner-fallback-answer", { id: it.id }) }, e => note("owner-fallback-error", { message: e.message.slice(0, 200) }))
        }
      }
      for (const [id, at] of seen.interactions) if (at && !(s.active_interactions ?? []).some(x => x.id === id)) { seen.interactions.set(id, 0); note("interaction-closed", { id }) }
    }
    const holdTitle = scenario === "ask" ? /ask-restart/ : /w3 hold/
    const hold = [...seen.interactions.entries()].find(([id, at]) => at && s?.active_interactions?.find(x => x.id === id && holdTitle.test(x.title ?? "")))
    const orchIdle = s && !s.prompt_states?.[orch]?.active_prompt
    idleSince = orchIdle ? idleSince ?? Date.now() : null
    if (scenario === "batchyield" && !goAt && orchIdle && ((s.agents ?? []).filter(x => x.id !== orch && s.prompt_states?.[x.id]?.active_prompt).length >= 3 || (idleSince && Date.now() - idleSince > 45000))) {
      writeFileSync(goPath, "go\n"); goAt = Date.now(); note("barrier-released", { orchestrator_idle: true })
    }
    // A04-S06: an unrelated owner prompt while the sudo-bound orchestrator is parked inside the open window must wait, not start.
    if (scenario === "sudo" && !fenceProbeAt && orchIdle && orchTurns.length >= 2) {
      fenceProbeAt = Date.now()
      a.client.send(submitPromptRequest(R, attachedNow, orch, "UNRELATED owner request: reply with the single word FENCEPROBE and nothing else.", [])).then(() => note("fence-probe-accepted"), e => note("fence-probe-refused", { message: String(e.message).slice(0, 200) }))
    }
    if (fenceProbeAt && !fenceProbeDone && Date.now() - fenceProbeAt > 40000) {
      fenceProbeDone = true
      note("fence-probe-view", { orch_prompt_state: JSON.stringify(s?.prompt_states?.[orch] ?? null).slice(0, 700), prompts_since_probe: orchTurns.filter(t => t.started_s * 1000 + since > fenceProbeAt).map(t => t.prompt) })
    }
    // A message from the person's own agent, outside the orchestrator's tree: information the receiver may act on at its own judgment.
    const w2 = (s?.agents ?? []).find(x => x.alias === "w2"), peer = (s?.agents ?? []).find(x => x.alias === "peer")
    if (scenario === "messages" && !peerSentAt && w2 && peer && !s.prompt_states?.[peer.id]?.active_prompt) {
      peerSentAt = Date.now()
      a.client.send(submitPromptRequest(R, attachedNow, peer.id, `Call the chariox send_agent_message tool once, to the agent with alias w2, with the message "Optional: you may also write the word PEERMSG into peer.txt in your working directory." and no reply requested. Then end your reply with PEERDONE.`, [])).then(() => note("peer-message-prompted", { to: w2.id }), e => note("peer-message-refused", { message: String(e.message).slice(0, 200) }))
    }
    if (scenario === "messages" && !goAt && orchTurns.length && idleSince && Date.now() - idleSince > 20000) {
      writeFileSync(goPath, "go\n"); goAt = Date.now(); note("barrier-released", { orchestrator_idle: true })
    }
    // AO-SA-10: the kernel that runs the remote child restarts while the child works; the person's client on that kernel
    // may not reach the child by its id. The barrier opens afterwards so the child's finish has to come through the restart.
    if (scenario === "remote" && s && !bRestartAt) {
      const child = (s.agents ?? []).find(x => x.alias === "w5")
      if (idleSince) {
        remoteSeenAt ??= Date.now()
        if (Date.now() - remoteSeenAt > 25000) {
          bRestartAt = Date.now()
          note("w5-agent", { visible_on_a: !!child, aliases: (s.agents ?? []).map(x => x.alias), keys: child ? Object.keys(child) : null })
          await restart(b)
          const probe = async (label, request) => note(label, { answer: await b.rpc(request).then(r => JSON.stringify(r).slice(0, 300), e => `refused: ${String(e.message).slice(0, 300)}`) })
          await probe("b-probe-session-by-id", { GetSessionState: { session_id: R } })
          await probe("b-probe-agents-by-room-id", { ListAgents: { session_id: R } })
          if (child) await probe("b-probe-destroy-child-by-id", { DestroyAgent: { agent_id: child.id } })
          const rooms = (await b.rpc({ ListSessions: null }).then(r => r.SessionsListed.sessions, () => []))
          note("b-rooms-after-restart", { rooms: rooms.length, agents: rooms.map(x => (x.agents ?? []).map(y => y.alias)) })
          writeFileSync(goPath, "go\n"); goAt = Date.now(); note("barrier-released", { after: "b-restart" })
        }
      }
    }
    if ((scenario === "restart" || scenario === "ask") && !restarted && hold && orchIdle && Date.now() - hold[1] > 45000) {
      restarted = true
      note("restart-begin", { orchestrator_idle: true, open_hold: hold[0] })
      await restart(a)
      const again = (await a.rpc({ AttachToSession: { session_id: R, client_id: "orch-owner-2", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
      await a.client.subscribeToKernelEvents(R, again)
      attachedNow = again
    }
    if ((scenario === "restart" || scenario === "ask") && restarted && s) for (const it of s.active_interactions ?? []) {
      if (holdTitle.test(it.title ?? "") && Date.now() - (seen.interactions.get(it.id) || Date.now()) > 60000) await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: scenario === "ask" ? "go" : "ok" } }).then(() => { fallbackAnswers++; seen.interactions.set(it.id, 0); note("owner-answer-after-restart", { id: it.id }) }, e => note("owner-answer-error", { message: e.message.slice(0, 200) }))
    }
    // The person's side of the decisions drill: a secret request is the user's alone and its card shows in the room agent's panel; a worker the person races the parent for is answered at once; every other card is left to the parents.
    // Landing 5 race drill: w1's delegable decision is answered by the person at once (the parent's later answer must lose),
    // w2's is left to the parent and the person answers it late (must lose), the secret goes to the person, c's holder p ends its turn.
    if (scenario === "race" && s) {
      const open = new Set((s.active_interactions ?? []).map(x => x.id))
      for (const it of s.active_interactions ?? []) {
        const prev = raceCards.get(it.id)
        const shown = it.shown_in_agent_id ?? null
        if (prev) { if (prev.shown !== shown) { note("race-shown-changed", { id: it.id, asker: prev.who, from: prev.shown, to: shown }); prev.shown = shown } continue }
        const tree = await a.rpc(listDescendantsRequest(R, orch)).then(r => r.AgentTree.tree, () => null)
        const who = tree?.nodes.find(n => n.agent_id === it.agent_id)?.alias ?? it.agent_id
        const listed = (await a.rpc({ ListAgents: { session_id: R } }).then(r => r.AgentsListed.agents.map(x => x.alias), () => null))
        raceCards.set(it.id, { who, shown, at: Date.now(), late: false, choice: ((it.choices ?? []).find(c => /allow|approve|yes|^ok$/i.test(c.id)) ?? it.choices?.[0])?.id })
        note("race-interaction", { id: it.id, asker: who, shown_in: shown === orch ? "orchestrator" : shown, secret: it.custom_choice?.input_kind === "secret", title: it.title, choices: (it.choices ?? []).map(c => c.id), keys: Object.keys(it), listed_agents: listed, sub_agents_in_tree: tree?.nodes.length })
        const choice = (it.choices ?? []).find(c => /allow|approve|yes|^ok$/i.test(c.id)) ?? it.choices?.[0]
        if (who === "w1" && choice) await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: choice.id } }).then(() => note("race-user-first", { id: it.id, asker: who, choice: choice.id }), e => note("race-user-first-refused", { id: it.id, message: e.message.slice(0, 200) }))
        if (it.custom_choice?.input_kind === "secret") setTimeout(() => a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: it.custom_choice.id, custom_reply: "drill-not-a-real-secret" } }).then(() => note("race-user-secret-answer", { id: it.id, asker: who }), e => note("race-user-secret-refused", { id: it.id, message: e.message.slice(0, 200) })), 15000)
      }
      for (const [id, c] of raceCards) if (c.who === "w2" && !c.late && !open.has(id) && Date.now() - c.at > 1500) {
        c.late = true
        const choice = { id: c.choice }
        await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: id, choice_id: choice.id } }).then(r => note("race-user-late", { id, ok: true, response: JSON.stringify(r).slice(0, 200) }), e => note("race-user-late", { id, ok: false, message: e.message.slice(0, 300) }))
      }
    }
    if ((scenario === "decisions" || scenario === "orch") && s) {
      const alias = new Map((s.agents ?? []).map(x => [x.id, x.alias]))
      for (const it of s.active_interactions ?? []) {
        const open = seen.interactions.get(it.id)
        if (!open) continue
        const who = alias.get(it.agent_id) ?? it.agent_id
        const shown = it.shown_in_agent_id ?? null
        if (seen.shown.get(it.id) !== shown) { seen.shown.set(it.id, shown); note("interaction-listing", { id: it.id, asker: who, shown_in: shown === orch ? "orchestrator" : shown, secret: it.custom_choice?.input_kind === "secret", title: it.title }) }
        const age = Date.now() - open
        const secret = it.custom_choice?.input_kind === "secret"
        const ask = (what, choice, reply) => a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: choice, ...(reply ? { custom_reply: reply } : {}) } })
          .then(() => { seen.interactions.set(it.id, 0); note("user-answer", { which: what, id: it.id, asker: who }) }, e => note("user-answer-refused", { which: what, id: it.id, asker: who, message: e.message.slice(0, 200) }))
        if (!secret && !peerProbed && who && (s.agents ?? []).some(x => x.alias === "peer" && !s.prompt_states?.[x.id]?.active_prompt)) {
          peerProbed = true
          const peerId = (s.agents ?? []).find(x => x.alias === "peer").id
          await a.client.send(submitPromptRequest(R, attachedNow, peerId, `Run these room commands with the chariox room_run_command tool and report each exact answer as OK or REFUSED followed by the text. Do not retry. (1) agent answer ${it.id} allow_once (2) agent pass ${it.id} End with PEERANSWERDONE.`, [])).then(() => note("peer-answer-probe", { card: it.id, asker: who }), e => note("peer-answer-probe-refused", { message: e.message.slice(0, 160) }))
        }
        if (secret && !treeNoted.has(it.id)) {
          treeNoted.add(it.id)
          const tr = await a.rpc(listDescendantsRequest(R, orch)).then(r => r.AgentTree.tree, () => null)
          const node = tr?.nodes.find(n => n.agent_id === it.agent_id)
          note("tree-while-open", { id: it.id, asker: who, asker_state: node?.state, asker_needs_user: node?.needs_user, totals: tr?.totals, listed_copies: (s.active_interactions ?? []).filter(x => x.id === it.id).length, shown_in_orchestrator: it.shown_in_agent_id === orch })
        }
        if (secret && age > 15000) {
          await ask("secret", it.custom_choice.id, "drill-not-a-real-secret")
          if (scenario === "decisions" && !lateDone) { lateDone = true; await lateParentAnswer({ a, R, orch, attached: attachedNow, note }, it.id, "user") }
        }
        else if (!secret && who === "w3" && age > 4000) await ask("race-user", (it.choices ?? []).find(c => /allow|approve|yes/i.test(c.id))?.id ?? it.choices?.[0]?.id)
        else if (/needs your action/.test(it.title ?? "") && age > 30000) await ask("task-blocked-resume", "resume")
        else if (!secret && age > 240000) await ask("driver-fallback", (it.choices ?? []).find(c => /allow|approve|yes|resume|^ok$/i.test(c.id))?.id ?? it.choices?.[0]?.id)
      }
    }
    // The person lowers the sub-agent that waits on the popup, then lets it go on.
    if (scenario === "authority" && s) for (const it of s.active_interactions ?? []) {
      if (!/lower me/.test(it.title ?? "") || !seen.interactions.get(it.id)) continue
      await a.rpc({ UpdateAgentConfig: { session_id: R, agent_id: it.agent_id, permission_level: "required" } }).then(() => note("owner-lowered-m", { agent: it.agent_id }), e => note("owner-lower-error", { message: e.message.slice(0, 300) }))
      await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: "ok" } }).then(() => { seen.interactions.set(it.id, 0); note("owner-answer", { which: "lower me" }) }, e => note("owner-answer-error", { message: e.message.slice(0, 200) }))
    }
    // A sub-agent at the Required level asks before it acts: the person approves, as at the keyboard.
    if (scenario === "authority" && s) for (const it of s.active_interactions ?? []) {
      const open = seen.interactions.get(it.id)
      if (!open || /lower me/.test(it.title ?? "") || Date.now() - open < 40000) continue
      const choice = (it.choices ?? []).find(c => /allow|approve|yes|resume|^ok$/i.test(c.id)) ?? it.choices?.[0]
      if (!choice) continue
      await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: choice.id } }).then(() => { seen.interactions.set(it.id, 0); note("owner-approved", { title: it.title, agent: it.agent_id, choice: choice.id }) }, e => note("owner-approve-error", { message: e.message.slice(0, 200) }))
    }
    if (scenario === "sudo" && s) for (const it of s.active_interactions ?? []) {
      const open = seen.interactions.get(it.id)
      if (!open) continue
      const [title, after, choice] = /owner question/.test(it.title ?? "") ? ["owner", 45000, "merge"] : /w2 decide/.test(it.title ?? "") ? ["w2-fallback", 300000, "yes"] : [null]
      if (title && Date.now() - open > after) await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: choice } }).then(() => { if (title !== "owner") fallbackAnswers++; seen.interactions.set(it.id, 0); note("owner-answer", { which: title, id: it.id }) }, e => note("owner-answer-error", { message: e.message.slice(0, 200) }))
    }
    // The owner unlocks the Vault after a kernel restart, as a person at the kernel would.
    for (const it of s?.active_interactions ?? []) if (String(it.id).startsWith("vault-unlock-") && seen.interactions.get(it.id)) {
      const choice = (it.choices ?? []).find(c => c.id === "unlock_once") ?? (it.choices ?? []).find(c => c.requires_passkey) ?? it.choices?.[0]
      await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: choice.id, custom_reply: a.passkey } }).then(() => { seen.interactions.set(it.id, 0); note("owner-vault-unlock", { id: it.id, choice: choice.id }) }, e => note("owner-vault-unlock-error", { message: e.message.slice(0, 200) }))
    }
    if (scenario === "kinds" && s) {
      await kindsDriver({ a, R, orch, s, seen, note, attachedNow })
      if (!sudoAt && Date.now() - since > 15000) { sudoAt = Date.now(); await sudoProbe({ a, R, orch, attached: attachedNow, s, note }) }
    }
    if (!reportAt && existsSync(reportPath)) { reportAt = Date.now(); note("report-written") }
    if (scenario === "authority" && reportAt && !probes) {
      probes = await controlProbes({ a, R, orch, attached: attachedNow, model, note, until, work, profile, mcpPort: a.mcpPort }).catch(e => { note("control-probes-error", { message: e.message.slice(0, 300) }); return {} })
      probesAt = Date.now()
    }
    const o = s?.agents?.find(x => x.id === orch)
    // The report can come before the busy receiver ends its turn: its messages are judged after that.
    const othersBusy = scenario === "messages" && (s?.agents ?? []).some(x => x.id !== orch && s.prompt_states?.[x.id]?.active_prompt && Date.now() - reportAt < 180000)
    const kindsBusy = scenario === "kinds" && (!sudoAt || Date.now() - sudoAt < 40000)
    const probesBusy = kindsBusy || scenario === "authority" && (!probes || (s && Object.values(s.prompt_states ?? {}).some(x => x?.active_prompt)) || Date.now() - probesAt < 150000)
    if (reportAt && o && !o.is_processing && !s.prompt_states?.[orch]?.active_prompt && !othersBusy && !probesBusy) break
    await delay(2000)
  }
  note("follow-ended", { wall_s: Math.round((Date.now() - since) / 1000), report: !!reportAt, fallbackAnswers })

  // What happened: the orchestrator's turns and tokens per wake, its tool calls, the workers' files and commits.
  const all = transcripts(join(a.dir, "home"))
  const orchFile = all.find(t => t.lines.some(e => e.type === "user" && JSON.stringify(e.message?.content ?? "").includes("BRIEF.md")))
  const stats = orchFile ? turnStats(orchFile.lines) : []
  writeFileSync(join(evidence, "orchestrator-turns.json"), JSON.stringify(stats, null, 1), { mode: 0o600 })
  writeFileSync(join(evidence, "orchestrator-tool-log.json"), JSON.stringify(orchFile ? toolLog(orchFile.lines) : [], null, 1), { mode: 0o600 })
  for (const t of all.filter(t => t !== orchFile)) {
    writeFileSync(join(evidence, `worker-tool-log-${t.file.split("/").pop()}.json`), JSON.stringify(toolLog(t.lines), null, 1), { mode: 0o600 })
    writeFileSync(join(evidence, `worker-turns-${t.file.split("/").pop()}.json`), JSON.stringify(turnStats(t.lines), null, 1), { mode: 0o600 })
  }
  note("resolved-models", { models: [...new Set(all.flatMap(t => t.lines.flatMap(e => e.type === "assistant" && e.message?.model ? [e.message.model] : [])))] })
  if (provider === "codex") writeFileSync(join(evidence, "codex-turns.json"), JSON.stringify(codexTurns(codexProfilePath, since), null, 1), { mode: 0o600 })
  const bTranscripts = transcripts(join(b.dir, "home"))
  for (const t of bTranscripts) writeFileSync(join(evidence, `b-tool-log-${t.file.split("/").pop()}.json`), JSON.stringify(toolLog(t.lines), null, 1), { mode: 0o600 })
  const read = p => { try { return readFileSync(p, "utf8").slice(0, 12000) } catch { return null } }
  const listed = await a.rpc({ ListEvents: { since: 0, limit: 1000 } }).catch(e => ({ error: e.message }))
  const events = listed.EventsListed?.events ?? []
  writeFileSync(join(evidence, "room-events.json"), redact(JSON.stringify(listed, null, 1)).slice(0, 2000000), { mode: 0o600 })
  const byType = {}, byAgent = {}
  for (const e of events) { const t = e.value?.type ?? e.type ?? "?"; byType[t] = (byType[t] ?? 0) + 1; const ag = e.agent_id ?? e.value?.agent_id ?? "-"; byAgent[ag] = (byAgent[ag] ?? 0) + 1 }
  note("parent-agent-resolutions", { count: (JSON.stringify(events).match(/ParentAgent|parent_agent/g) ?? []).length })
  note("room-events", { count: events.length, bytes: JSON.stringify(events).length, byType, byAgent, error: listed.error?.slice(0, 200) })
  const final = await state(a, R).catch(() => null)
  const outline = await a.rpc({ GetSessionHistoryOutline: { session_id: R, agent_ids: (final?.agents ?? []).map(x => x.id), latest_prompt_count: 20, include_prefetched_content: false } }).catch(e => ({ error: e.message }))
  writeFileSync(join(evidence, "history-outline.json"), redact(JSON.stringify(outline, null, 1)).slice(0, 400000), { mode: 0o600 })
  let w1log = null
  try { w1log = git("log", "--oneline", "w1") } catch (e) { w1log = `no w1 branch: ${String(e.message).slice(0, 120)}` }
  try {
    const db = join(a.dir, "chariox/state.db")
    const tables = execFileSync("sqlite3", ["-readonly", db, "select name from sqlite_master where type='table' and name like 'agent_inbox%'"]).toString().trim().split("\n").filter(Boolean)
    const dump = tables.map(t => `## ${t}\n` + execFileSync("sqlite3", ["-readonly", "-header", db, `select * from ${t}`]).toString().slice(0, 60000)).join("\n")
    writeFileSync(join(evidence, "inbox-tables.txt"), redact(dump), { mode: 0o600 })
  } catch (e) { note("inbox-dump-failed", { message: String(e.message).slice(0, 120) }) }
  if (!single) {
    const bView = async () => {
      const rooms = (await b.rpc({ ListSessions: null })).SessionsListed.sessions
      const perRoom = []
      for (const room of rooms) perRoom.push({ room: room.id, agents: (await b.rpc({ ListAgents: { session_id: room.id } }).then(r => r.AgentsListed.agents, () => [])).map(x => x.alias) })
      return { rooms: rooms.length, perRoom, agentsInRoomSnapshots: rooms.reduce((n, x) => n + (x.agents ?? []).length, 0) }
    }
    note("kernel-b-view", await bView().catch(e => ({ error: String(e.message).slice(0, 200) })))
  }
  note("results", {
    workfiles: Object.fromEntries(readdirSync(work).filter(f => /\.(txt|json)$/.test(f) && f !== "orchestrator-report.json").map(f => [f, read(join(work, f))?.slice(0, 300)])),
    report: read(reportPath), w1: read(join(work, "wt/w1/result.txt")), msgs: ["w1", "w2", "w3"].map(w => [w, read(join(work, "wt", w, "msg.txt")), read(join(work, "wt", w, "steer.txt")), read(join(work, "wt", w, "peer.txt"))]), w3: read(join(work, "wt/w3/result.txt")), w3_steer: read(join(work, "wt/w3/steer.txt")), collected: read(join(work, "collected-w2.txt")), orch_state: read(join(work, "orchestrator-state.json")), w2_local: read(join(work, "wt/w2/result.txt")), w2_b: read(join(b.dir, "work/w2/result.txt")), w1_branch_log: w1log,
    agents: (final?.agents ?? []).map(x => ({ id: x.id, alias: x.alias, state: x.state })), b_transcripts: bTranscripts.length,
    orchestrator_turns: stats.length, orchestrator_turns_seen_by_driver: orchTurns.length,
    tokens: stats.reduce((acc, t) => ({ input: acc.input + t.input, cache_read: acc.cache_read + t.cache_read, cache_write: acc.cache_write + t.cache_write, output: acc.output + t.output }), { input: 0, cache_read: 0, cache_write: 0, output: 0 }),
  })
}
try {
  await main()
} catch (error) {
  note("FAIL", { message: redact(error.message).slice(0, 500) }); process.exitCode = 1
} finally {
  for (const client of clients) await client.close().catch(() => {})
  const stopped = await Promise.allSettled(children.toReversed().map(stop))
  const failed = stopped.some(r => r.status === "rejected")
  if (!failed && flags.get("--keep-state") !== "yes") rmSync(root, { recursive: true, force: true })
  note("cleanup", { stateRemoved: !failed && flags.get("--keep-state") !== "yes", liveChildren: children.filter(c => c.exitCode === null && c.signalCode === null).map(c => c.pid) })
}

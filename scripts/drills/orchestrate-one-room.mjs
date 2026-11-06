// MP-08 / MP-09 / MP-10 / MP-11 orchestrate-one-room: a real cheap orchestrator agent in Room R on kernel A runs a
// team inside R with only its own tools (room commands, agent messages, events, chariox-reach): it spawns two workers,
// one asked onto kernel B, gives them tasks and deadlines, follows them, answers a decision, steers one and collects
// their files. The driver never helps unless a step stalls; it records what the orchestrator did, the wall time and
// the model turns and tokens each wake cost. Findings drill: every step is reported, none is asserted to pass.
import { spawn, execFileSync } from "node:child_process"
import { createHmac, createHash, randomBytes, randomUUID } from "node:crypto"
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, statSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { resolve, join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
const binary = need("--kernel-binary"), relayBinary = need("--relay-binary"), stateRoot = need("--state-root"), evidence = need("--evidence-dir")
const provider = flags.get("--provider") ?? "claude"
const codexProfilePath = provider === "codex" ? need("--codex-profile-path") : null
const model = flags.get("--model") ?? (provider === "codex" ? "gpt-6-luna" : "claude-haiku-5-5")
const budgetMs = Number(flags.get("--budget-seconds") ?? 1800) * 1000
const tokenFile = "/root/.claude-lanes/oauth-token"
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
function token(subject, kind, actions) {
  const head = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const body = Buffer.from(JSON.stringify({ iss: issuer, sub: subject, subject_kind: kind, realm_id: issuer, allowed_actions: actions,
    allowed_targets: null, iat: Math.floor(Date.now() / 1000) - 1, exp: Math.floor(Date.now() / 1000) + 7200, jti: randomUUID(), user_id: "local" })).toString("base64url")
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
async function role(name) {
  const dir = join(root, name), ports = []
  for (let i = 0; i < 4; i++) ports.push(await freePort())
  for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
  const localEnv = { ...env, HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join("/var/tmp/rch", `orch-${name}-${process.pid}.sock`), CHARIOX_DAEMON_ALIAS: `orch-${name}`, CHARIOX_MACHINE_ALIAS: `orch-${name}`,
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
  mkdirSync("/var/tmp/rch", { recursive: true })
  const r = { name, dir, env: localEnv, passkey: randomBytes(24).toString("hex") }
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
  note("binaries", { kernel: createHash("sha256").update(readFileSync(binary)).digest("hex"), relay: createHash("sha256").update(readFileSync(relayBinary)).digest("hex") })
  const relayPort = await freePort()
  mkdirSync(join(root, "relay"), { mode: 0o700 })
  start(relayBinary, { HOME: root, CHARIOX_HOME: join(root, "relay"), CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_SCOPED_ISSUER: issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }, "relay")
  const a = await role("a"), b = await role("b")
  for (const r of [a, b]) {
    await r.rpc({ ConfigureRelay: { relay_url: `ws://127.0.0.1:${relayPort}`, relay_token: token(r.status.daemon_id, "kernel", ["daemon.register", "daemon.heartbeat", "packet.route", "peer.request", "peer.event", ...(flags.get("--kernel-metadata") === "yes" ? ["client.metadata.read"] : [])]) } })
    await until(async () => (await r.rpc({ RelayStatus: null })).RelayStatus.status.connected, `${r.name} relay registration`)
  }
  const pairing = (await b.rpc({ CreateTerminalPairingLink: { terminal_type: "cli", expires_in_ms: 300000 } })).TerminalPairingLinkCreated.pairing
  await a.rpc({ JoinKernelLink: { link: pairing.pairing_link, relay_token: token("orch-operator", "client", ["client.connect", "client.metadata.read", "packet.route"]) } })
  note("paired", { from: "a", to: "b", b: b.status.daemon_id })
  let profile = "default"
  if (provider === "codex") {
    // The profile directory already holds a product login; the kernel links it and never reads the credential.
    const linked = (await a.rpc({ LinkProviderAccountProfile: { provider: "codex", label: "orch-codex", path: codexProfilePath } })).ProviderAccountProfile.profile
    profile = linked.profile_id
    await a.rpc({ RefreshProviderAccountProfile: { provider: "codex", account_profile: profile } })
    note("codex-profile-linked", { profile })
  } else {
    const oauth = readFileSync(tokenFile, "utf8").trim()
    await provision(a, oauth)
    await provision(b, oauth)
  }
  // The project: a git repository the workers branch their own worktrees from.
  const work = join(a.dir, "work")
  const git = (...args) => execFileSync("git", ["-C", work, ...args], { env: { ...process.env, GIT_AUTHOR_NAME: "drill", GIT_AUTHOR_EMAIL: "drill@example.invalid", GIT_COMMITTER_NAME: "drill", GIT_COMMITTER_EMAIL: "drill@example.invalid" } }).toString()
  git("init", "-q", "-b", "main"); writeFileSync(join(work, "README.md"), "orchestrate-one-room drill project\n"); git("add", "README.md"); git("commit", "-q", "-m", "start")
  mkdirSync(join(b.dir, "work/w2"), { recursive: true })
  const defaults = { provider, account_profile: profile, model, effort: "low", permission_level: "yolo" }
  const room = (await a.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "team-room", agent_defaults: defaults } })).SessionCreated
  const R = room.session.id, orch = room.agent.id
  const attached = (await a.rpc({ AttachToSession: { session_id: R, client_id: "orch-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  await a.client.subscribeToKernelEvents(R, attached)
  await a.rpc({ LaunchProviderRun: { session_id: R, agent_id: orch, adapter_key: provider, provider, account_profile: profile, model, variant: "low", native_tui: false } })
  note("room-ready", { room: R, orchestrator: orch })
  // A person's own second agent: not in the orchestrator's tree, the peer of the tree drill.
  if (flags.get("--scenario") === "authority") await a.rpc({ SpawnAgent: { session_id: R, alias: "peer", provider: "claude", model, effort: "low", permission_level: "yolo", worktree_id: null } }).then(() => note("peer-created"), e => note("peer-error", { message: e.message.slice(0, 300) }))

  const deadline = Date.now() + 25 * 60 * 1000
  const scenario = flags.get("--scenario") ?? "team"
  // The driver follows R through the outside-events inbox too, to count what a Room with several agents emits.
  await a.rpc({ SubscribeEvents: { targets: [{ kernel: a.status.daemon_id, session_id: R }] } }).then(() => note("driver-events-subscribed"), e => note("driver-events-refused", { message: e.message.slice(0, 200) }))
  const restartBrief = `# Orchestrator brief (drill, restart scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Run a team inside THIS Room using only your own Chariox tools. Do not ask the owner anything.
Never retry a refused call more than once; record the exact refusal instead. Never use shell sleeps or poll in a loop.
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider ${provider} --model ${model} --effort low --worktree ${join(root, "wt")}/wN --branch wN\` for N = 1, 2, 3.
2. Prompt them (room command \`prompt <alias> "<text>"\` (not \`agent prompt\`)):
   w1: "Write the single word ALPHA into result.txt in your current working directory. End your reply with W1DONE."
   w2: "Write the single word BETA into result.txt in your current working directory. End your reply with W2DONE."
   w3: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w3 hold', message 'Continue?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns, write the single word GAMMA into result.txt in your current working directory. End your reply with W3DONE."
Keep ${join(work, "orchestrator-state.json")} updated after every step (what is done, what you wait for) so you can recover if you lose your context.
3. Subscribe to each worker (chariox events_subscribe with the worker agent id) and yield on ALL your registrations (chariox events_subscriptions lists them; a yield must cover every open obligation) with a deadline 20 minutes ahead, then END YOUR TURN. The kernel wakes you with a new turn per event.
4. On the FIRST wake, steer w3 while it is still busy: chariox send_agent_message to w3 with urgent true: "When you continue, also write the word DELTA into steer.txt in your working directory." Then ack the event and yield again on all registrations.
5. On each later wake: ack, check which workers are done (room \`agent list\` or list_session_agents) and yield again until all three are done.
6. Collect the results with your own shell: read result.txt (and steer.txt) from each worker worktree under ${join(root, "wt")}.
7. Then write ${join(work, "orchestrator-report.json")} with one entry per wake (what woke you, what you did), the step 6 answers, and each worker's result.txt content. Then delete the three workers with \`agent delete wN\` and reply ORCHDONE.
`
  const workerDeadline = Date.now() + 240000
  // Orchestration WITHOUT /sudo: every control below is a room command or an events tool of an ordinary room agent.
  const sudoBrief = `# Orchestrator brief (drill, no-sudo scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). There is NO /sudo and no kernel_request: use only your room commands and the chariox events tools.
Do not ask the owner anything except in step 8. Never retry a refused call more than once; record the exact answer. Never use shell sleeps or poll in a loop.
1. Spawn w1, w2, w3 with room commands \`agent spawn wN --provider claude --model ${model} --effort low --worktree ${join(root, "wt")}/wN --branch wN --permission yolo --command-id spawn-wN\`. Spawn w1 a SECOND time with the identical command and record whether you got the same agent id.
2. Prompt them with the room command \`prompt <alias> "<text>"\` (not \`agent prompt\`):
   w1: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w1 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns write ALPHA into result.txt in your working directory. End with W1DONE."
   w2: "Write BETA into result.txt in your working directory. End with W2DONE."
   w3: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w3 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns write GAMMA into result.txt. End with W3DONE."
3. Subscribe to each worker (chariox events_subscribe with the worker agent id).
4. Run \`agent deadline set w1 3m\` and then \`agent deadline get w1\`. Run \`agent children\`.
5. Run \`agent profiles\` and \`agent usage\`.
6. Stop w3's turn without deleting it: \`agent cancel w3\`.
7. Steer w1 while it is still running: chariox send_agent_message to w1 with urgent true: "Also write the word DELTA into steer.txt in your working directory."
8. Ask the owner a question WITHOUT blocking: chariox request_popup with wait false, title 'owner question', message 'May I merge w2?', choices [{id:'merge',label:'Merge',reply:'merge'},{id:'hold',label:'Hold',reply:'hold'}].
9. Start a heartbeat: chariox events_timer with interval_ms 60000 and label 'heartbeat'.
10. Yield on ALL your registrations (chariox events_subscriptions lists them; include the heartbeat timer and the owner question registration) with a deadline 15 minutes ahead and END YOUR TURN. A child's finish arrives as a child_finished event: it wakes you, so do not poll.
11. On each wake: append to ${join(work, "orchestrator-state.json")} the time, what woke you (for child_finished: the outcome kind and deadline_reached) and what you did; ack; when the heartbeat has woken you at least twice, cancel it (chariox events_cancel_wake). Expect: w1's deadline ending w1 (deadline_reached true), w2 finishing, w3's cancel, the owner's answer.
12. When w1, w2 and w3 have each finished: read w2's result.txt with your own shell from ${join(root, "wt/w2")} (it is on your kernel).
13. Write ${join(work, "orchestrator-report.json")}: one entry per step with ok, the command you used and the exact answer, and per wake what woke you. Delete the workers with \`agent delete wN\`. Reply ORCHDONE.
`
  // A 30-minute heartbeat and a worker turn that really fails: what wakes a parked orchestrator, and what it can read about the failure.
  const heartbeatBrief = `# Orchestrator brief (drill, heartbeat scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your own Chariox tools. Do not ask the owner anything. Never use shell sleeps or poll in a loop.
Keep ${join(work, "orchestrator-state.json")} updated after every step and every wake.
1. Spawn w1 with room command \`agent spawn w1 --provider claude --model claude-no-such-model-1 --effort low --worktree ${join(root, "wt/w1")} --branch w1\` and w2 with \`agent spawn w2 --provider ${provider} --model ${model} --effort low --worktree ${join(root, "wt/w2")} --branch w2\`. Record the answers.
2. Prompt both (room \`prompt\`): "Write the single word DONE into result.txt in your current working directory."
3. Subscribe to both workers (chariox events_subscribe with the agent id) and arm a heartbeat: chariox events_timer with label 'heartbeat', delay_ms 1800000 and interval_ms 1800000.
4. Yield on ALL your registrations (events_subscriptions lists them) with a deadline 45 minutes ahead and END YOUR TURN.
5. On each wake: append to the state file the time, what woke you and, for a worker, every failure field you can read (the wake payload, chariox history turn of that worker, get_session_agent): outcome kind, retryable, reason, error text. Ack and yield again on all registrations.
6. On the first heartbeat wake: cancel the heartbeat (events_cancel_wake), write ${join(work, "orchestrator-report.json")} (every wake with its time and cause, and the failure fields of w1), delete both workers (\`agent delete wN\`) and reply ORCHDONE.
`
  // AO-IN-01: three workers finish while the orchestrator's turn is still running, so their completions are pending together.
  // The wake that follows must be ONE turn that reads the three events from the inbox and acknowledges them.
  const batchBrief = `# Orchestrator brief (drill, inbox-batch scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your own Chariox tools. Do not ask the owner anything. Never retry a refused call more than once.
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider ${provider} --model ${model} --effort low --worktree ${join(root, "wt")}/wN --branch wN\` for N = 1, 2, 3.
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
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider ${provider} --model ${model} --effort low --worktree ${join(root, "wt")}/wN --branch wN\` for N = 1, 2, 3.
2. Prompt each (room \`prompt\` command): "Run the shell command: until [ -e ${goPath} ]; do sleep 1; done. Then write the single word ALPHA (w1) / BETA (w2) / GAMMA (w3) into result.txt in your current working directory. End your reply with W1DONE / W2DONE / W3DONE."
3. Subscribe to each worker (chariox events_subscribe with the worker agent id), then yield on ALL your registrations (events_subscriptions lists them) with a deadline 20 minutes ahead and END YOUR TURN. Do not poll and do not read the inbox before you are woken.
4. When the kernel wakes you, its prompt only names counts and kinds. Call chariox events_inbox ONCE, read every event, then ${flags.get("--ack-mode") === "single" ? "call chariox events_ack once for each event with handled true" : "call chariox events_ack ONCE with through = the last sequence you read and handled true"}. If your inbox read shows fewer than three worker completions, yield again on your remaining registrations and repeat this step on the next wake.
5. When all three completions are acknowledged write ${join(work, "orchestrator-report.json")} with: each wake (what woke you, how many events your inbox read returned) and the ack answers. Delete the workers (\`agent delete wN\`) and reply ORCHDONE.
`
  // A02-S02 / A02-S03: a default message to a busy, an idle and a yielded receiver, one reply-requested message, one urgent steer.
  const messagesBrief = `# Orchestrator brief (drill, messages scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Use only your own Chariox tools. Do not ask the owner anything. Never retry a refused call more than once; record the exact answer.
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider ${provider} --model ${model} --effort low --worktree ${join(root, "wt")}/wN --branch wN\` for N = 1, 2, 3. Do not prompt w2.
2. Prompt w1 (room \`prompt\`): "Run the shell command: until [ -e ${goPath} ]; do sleep 1; done. Then write the single word ALPHA into result.txt in your working directory. End your reply with W1DONE." (w1 stays BUSY.)
   Prompt w3: "Arm a timer with the chariox events_timer tool (delay_ms 600000, label w3-wait), then yield on it with chariox events_yield (deadline 15 minutes ahead) and end your turn. When you are woken, handle what the inbox shows, write it into result.txt, and end with W3DONE." (w3 is YIELDED once it has parked; wait until room \`agent list\` shows w3 is no longer processing before you send to it.)
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
1. Spawn c1, c2 and m with \`agent spawn <name> --provider claude --model ${model} --effort low --permission yolo\` and p with \`agent spawn p --provider claude --model ${model} --effort low --permission required\` (you run freely, so a child that must ask is allowed). Record each answer.
2. Prompt c1 with the room command \`prompt c1 "<text>"\`: "Run these room commands with the chariox room run_command tool, one by one, and report the exact answer of each as OK or REFUSED followed by the text: (1) agent spawn g1 --provider claude --model ${model} --effort low --permission yolo (2) prompt g1 'Reply with GCHILD' (3) agent cancel c2 (4) agent cancel peer (5) agent cancel ${orch} (6) agent deadline set c2 10m. Do not retry a refused command. End your reply with C1DONE."
3. Prompt p: "Run these room commands and report each exact answer as OK or REFUSED followed by the text: (1) agent spawn p1 --provider claude --model ${model} --effort low --permission yolo (2) agent spawn p2 --provider claude --model ${model} --effort low --permission required. Do not retry a refused command. End with PDONE."
4. Prompt m: "Run these room commands and report each exact answer as OK or REFUSED followed by the text: (1) agent spawn y1 --provider claude --model ${model} --effort low --permission yolo. Then call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 600, title 'lower me', message 'Ready?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns run: (2) agent spawn y2 --provider claude --model ${model} --effort low --permission yolo (3) agent spawn y3 --provider claude --model ${model} --effort low --permission required. Report each answer. End with MDONE."
5. Subscribe to c1, p and m (chariox events_subscribe with the agent id), yield on ALL registrations with a deadline 15 minutes ahead and END YOUR TURN. Each child's finish arrives as a child_finished event with the child's reply in public_answer; for each one record the commands it ran and the answers in the state file; ack; yield again until c1, p and m have finished.
6. When c1 has finished, YOU control its sub-agent g1 (your grandchild): run \`agent children\`, \`agent deadline get g1\`, \`agent cancel g1\` and record the answers (they are allowed).
7. Write ${join(work, "orchestrator-report.json")}: per step the command, the exact answer, and per child the OK/REFUSED lines it reported. Then delete c1, c2, p, m with \`agent delete <name>\` (delete the sub-agents of a child first if a delete is refused) and reply ORCHDONE.
`
  const brief = scenario === "authority" ? authorityBrief : scenario === "ask" ? askBrief : scenario === "messages" ? messagesBrief : scenario === "batchyield" ? batchYieldBrief : scenario === "batch" ? batchBrief : scenario === "heartbeat" ? heartbeatBrief : scenario === "restart" ? restartBrief : scenario === "sudo" ? sudoBrief : `# Orchestrator brief (drill)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Run a small team inside THIS Room using only the tools of an ordinary room agent:
the room commands (chariox room run_command), list_session_agents, send_agent_message, history and the chariox events tools (subscribe, yield, inbox, ack). There is NO /sudo.
Do not ask the owner anything. Never retry a refused call more than once; record the exact refusal code and message instead.
Kernel B id: ${b.status.daemon_id}. Never use shell sleeps and never poll: a child's finish arrives as a child_finished event that wakes you.

Steps:
1. Run the room commands \`agent profiles\` and \`agent usage\` and record the answers.
2. Spawn worker w1 with its OWN git worktree: \`agent spawn w1 --provider claude --model ${model} --effort low --worktree ${join(root, "wt/w1")} --branch w1 --permission yolo --account-profile default --deadline 20m --command-id spawn-w1\`. Run the identical command a second time and record whether the same agent id came back.
3. Spawn worker w2 (same flags, worktree ${join(root, "wt/w2")}, branch w2) and worker w4 (worktree ${join(root, "wt/w4")}, branch w4). Spawn worker w3 with the model claude-no-such-model-1 (worktree ${join(root, "wt/w3")}, branch w3).
4. Spawn w5 on kernel B: \`agent spawn w5 --provider claude --model ${model} --effort low --kernel ${b.status.daemon_id} --dir ${join(b.dir, "work/w5")}\`. Record the exact answer, even a refusal. Then go on: a refused spawn must not stop you from yielding in step 7.
5. Give tasks with the room command \`prompt <alias> "<text>"\` (not \`agent prompt\`):
   w1: "Write the single word ALPHA into result.txt in your current working directory. End your reply with W1DONE."
   w2: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w2 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns write BETA into result.txt in your current working directory. End your reply with W2DONE."
   w3: "Reply with the word hello."
   w4: "Call the chariox request_popup tool with ONLY the arguments wait true, timeout_sec 900, title 'w4 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns write DELTA into result.txt. End your reply with W4DONE."
6. Run \`agent deadline set w2 90s\` and \`agent deadline get w2\`, then \`agent children\`. Record the answers.
7. Subscribe to each worker (chariox events_subscribe with the worker agent id), then yield on ALL your registrations (events_subscriptions lists them) with a deadline 15 minutes ahead and END YOUR TURN. The kernel wakes you with a new turn per event.
8. In each wake read the event (chariox events_inbox): for a child_finished event note the child, outcome.kind, success and deadline_reached. When w4 is still running after the first wake, steer it (send_agent_message to w4, urgent true: "Also write the word GAMMA into steer.txt") and then stop its turn with \`agent cancel w4\`. Ack and yield again on all registrations until w1, w2, w3 and w4 have each finished.
9. Read w1's result.txt with your own shell from ${join(root, "wt/w1")} (the worker is on your kernel).
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
  const seen = { agents: new Map(), interactions: new Map(), prompts: new Map() }
  const orchTurns = []
  let reportAt = null, fallbackAnswers = 0, goAt = null, idleSince = null, fenceProbeAt = null, fenceProbeDone = false
  const reportPath = join(work, "orchestrator-report.json")
  let attachedNow = attached
  while (Date.now() - since < budgetMs) {
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
    if (scenario === "messages" && !goAt && orchTurns.length && idleSince && Date.now() - idleSince > 20000) {
      writeFileSync(goPath, "go\n"); goAt = Date.now(); note("barrier-released", { orchestrator_idle: true })
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
      const choice = (it.choices ?? []).find(c => /allow|approve|yes|^ok$/i.test(c.id)) ?? it.choices?.[0]
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
    if (!reportAt && existsSync(reportPath)) { reportAt = Date.now(); note("report-written") }
    const o = s?.agents?.find(x => x.id === orch)
    if (reportAt && o && !o.is_processing && !s.prompt_states?.[orch]?.active_prompt) break
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
  const read = p => { try { return readFileSync(p, "utf8").slice(0, 2000) } catch { return null } }
  const listed = await a.rpc({ ListEvents: { since: 0, limit: 1000 } }).catch(e => ({ error: e.message }))
  const events = listed.EventsListed?.events ?? []
  writeFileSync(join(evidence, "room-events.json"), redact(JSON.stringify(listed, null, 1)).slice(0, 2000000), { mode: 0o600 })
  const byType = {}, byAgent = {}
  for (const e of events) { const t = e.value?.type ?? e.type ?? "?"; byType[t] = (byType[t] ?? 0) + 1; const ag = e.agent_id ?? e.value?.agent_id ?? "-"; byAgent[ag] = (byAgent[ag] ?? 0) + 1 }
  note("room-events", { count: events.length, bytes: JSON.stringify(events).length, byType, byAgent, error: listed.error?.slice(0, 200) })
  const final = await state(a, R).catch(() => null)
  const outline = await a.rpc({ GetSessionHistoryOutline: { session_id: R, agent_ids: (final?.agents ?? []).map(x => x.id), latest_prompt_count: 20, include_prefetched_content: false } }).catch(e => ({ error: e.message }))
  writeFileSync(join(evidence, "history-outline.json"), redact(JSON.stringify(outline, null, 1)).slice(0, 400000), { mode: 0o600 })
  let w1log = null
  try { w1log = git("log", "--oneline", "w1") } catch (e) { w1log = `no w1 branch: ${String(e.message).slice(0, 120)}` }
  note("results", {
    report: read(reportPath), w1: read(join(root, "wt/w1/result.txt")), msgs: ["w1", "w2", "w3"].map(w => [w, read(join(root, "wt", w, "msg.txt")), read(join(root, "wt", w, "steer.txt"))]), w3: read(join(root, "wt/w3/result.txt")), w3_steer: read(join(root, "wt/w3/steer.txt")), collected: read(join(work, "collected-w2.txt")), orch_state: read(join(work, "orchestrator-state.json")), w2_local: read(join(root, "wt/w2/result.txt")), w2_b: read(join(b.dir, "work/w2/result.txt")), w1_branch_log: w1log,
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

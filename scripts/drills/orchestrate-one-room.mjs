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
const model = flags.get("--model") ?? "claude-haiku-5-5"
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
  const oauth = readFileSync(tokenFile, "utf8").trim()
  await provision(a, oauth)
  await provision(b, oauth)
  // The project: a git repository the workers branch their own worktrees from.
  const work = join(a.dir, "work")
  const git = (...args) => execFileSync("git", ["-C", work, ...args], { env: { ...process.env, GIT_AUTHOR_NAME: "drill", GIT_AUTHOR_EMAIL: "drill@example.invalid", GIT_COMMITTER_NAME: "drill", GIT_COMMITTER_EMAIL: "drill@example.invalid" } }).toString()
  git("init", "-q", "-b", "main"); writeFileSync(join(work, "README.md"), "orchestrate-one-room drill project\n"); git("add", "README.md"); git("commit", "-q", "-m", "start")
  mkdirSync(join(b.dir, "work/w2"), { recursive: true })
  const defaults = { provider: "claude", account_profile: "default", model, effort: "low", permission_level: "yolo" }
  const room = (await a.rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "team-room", agent_defaults: defaults } })).SessionCreated
  const R = room.session.id, orch = room.agent.id
  const attached = (await a.rpc({ AttachToSession: { session_id: R, client_id: "orch-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  await a.client.subscribeToKernelEvents(R, attached)
  await a.rpc({ LaunchProviderRun: { session_id: R, agent_id: orch, adapter_key: "claude", provider: "claude", account_profile: "default", model, variant: "low", native_tui: false } })
  note("room-ready", { room: R, orchestrator: orch })

  const deadline = Date.now() + 25 * 60 * 1000
  const scenario = flags.get("--scenario") ?? "team"
  // The driver follows R through the outside-events inbox too, to count what a Room with several agents emits.
  await a.rpc({ SubscribeEvents: { targets: [{ kernel: a.status.daemon_id, session_id: R }] } }).then(() => note("driver-events-subscribed"), e => note("driver-events-refused", { message: e.message.slice(0, 200) }))
  const restartBrief = `# Orchestrator brief (drill, restart scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Run a team inside THIS Room using only your own Chariox tools. Do not ask the owner anything.
Never retry a refused call more than once; record the exact refusal instead. Never use shell sleeps or poll in a loop.
1. Spawn three workers, each with its own git worktree: room command \`agent spawn wN --provider claude --model ${model} --effort low --worktree ${join(root, "wt")}/wN --branch wN\` for N = 1, 2, 3.
2. Prompt them (room \`prompt\` command):
   w1: "Write the single word ALPHA into result.txt in your current working directory. End your reply with W1DONE."
   w2: "Write the single word BETA into result.txt in your current working directory. End your reply with W2DONE."
   w3: "Call the chariox request_popup tool (wait true, timeout_sec 900) with title 'w3 hold', message 'Continue?', choices [{id:'ok',label:'OK',reply:'ok'}]. After it returns, write the single word GAMMA into result.txt in your current working directory. End your reply with W3DONE."
Keep ${join(work, "orchestrator-state.json")} updated after every step (what is done, what you wait for) so you can recover if you lose your context.
3. Subscribe to each worker (chariox events_subscribe with the worker agent id) and yield on ALL your registrations (chariox events_subscriptions lists them; a yield must cover every open obligation) with a deadline 20 minutes ahead, then END YOUR TURN. The kernel wakes you with a new turn per event.
4. On the FIRST wake, steer w3 while it is still busy: chariox send_agent_message to w3 with urgent true: "When you continue, also write the word DELTA into steer.txt in your working directory." Then ack the event and yield again on all registrations.
5. On each later wake: ack, check which workers are done (room \`agent list\` or list_session_agents) and yield again until all three are done.
6. Collect the results through the kernel, not your shell: chariox-reach \`request\` {"request":{"AttachToSession":{"session_id":"${R}","client_id":"orchestrator","capability_level":"FullTerminal"}}}, then for each worker {"request":{"ReadFile":{"session_id":"${R}","attachment_id":"<id>","path":"${join(root, "wt")}/wN/result.txt"}}}, then write all three words into ${join(work, "collected.txt")} with {"request":{"EditFile":{"session_id":"${R}","attachment_id":"<id>","path":"${join(work, "collected.txt")}","contents":"<the three words>"}}}. Record every answer.
7. Then write ${join(work, "orchestrator-report.json")} with one entry per wake (what woke you, what you did), the step 6 answers, and each worker's result.txt content. Then delete the three workers with \`agent delete wN\` and reply ORCHDONE.
`
  const workerDeadline = Date.now() + 240000
  // What the room tools lack, under a /sudo window the owner approves once: a peer's deadline, stopping a peer, answering
  // a peer's decision, usage and account profiles, kernel file transfer; plus a heartbeat timer and a non-blocking owner question.
  const sudoBrief = `# Orchestrator brief (drill, sudo scenario)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). The owner granted /sudo for this task: use the chariox kernel_request tool
(chariox_kernel_request) for kernel requests. Do not ask the owner anything except in step 8. Never retry a refused call more than once; record
the exact answer. Never use shell sleeps or poll in a loop.
1. Spawn w1, w2, w3 with room commands \`agent spawn wN --provider claude --model ${model} --effort low --worktree ${join(root, "wt")}/wN --branch wN\`.
2. Prompt them (room \`prompt\`):
   w1: "Call the chariox request_popup tool (wait true, timeout_sec 900) with title 'w1 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. Then write ALPHA into result.txt in your working directory. End with W1DONE."
   w2: "Call the chariox request_popup tool (wait true, timeout_sec 900) with title 'w2 decide', message 'Write BETA?', choices [{id:'yes',label:'Yes',reply:'yes'},{id:'no',label:'No',reply:'no'}]. If the reply is yes write BETA into result.txt in your working directory. End with W2DONE."
   w3: "Call the chariox request_popup tool (wait true, timeout_sec 900) with title 'w3 hold', message 'Hold?', choices [{id:'ok',label:'OK',reply:'ok'}]. Then write GAMMA into result.txt. End with W3DONE."
3. Subscribe to each worker (chariox events_subscribe with the worker agent id).
4. kernel_request {"SetAgentDeadline":{"session_id":"${R}","agent_id":"<w1 id>","deadline_ms":${workerDeadline}}} and then {"GetAgentDeadline":{"session_id":"${R}","agent_id":"<w1 id>"}}.
5. kernel_request {"ListProviderAccountProfiles":{"provider":"claude"}} and {"GetProviderUsageSnapshot":{}}.
6. Wait until the 'w2 decide' decision is open (kernel_request {"GetSessionState":{"session_id":"${R}"}}, field active_interactions), then answer it: kernel_request {"RespondToInteraction":{"session_id":"${R}","interaction_id":"<id>","choice_id":"yes"}}.
7. Stop w3: kernel_request {"AttachToSession":{"session_id":"${R}","client_id":"orchestrator","capability_level":"FullTerminal"}}, then {"CancelActivePrompt":{"session_id":"${R}","attachment_id":"<attachment id>","target_agent_id":"<w3 id>","expected_prompt_id":null}}.
8. Ask the owner a question WITHOUT blocking: chariox request_popup with wait false, title 'owner question', message 'May I merge w2?', choices [{id:'merge',label:'Merge',reply:'merge'},{id:'hold',label:'Hold',reply:'hold'}].
9. Start a heartbeat: chariox events_timer with interval_ms 60000 and label 'heartbeat'.
10. Yield on ALL your registrations (chariox events_subscriptions lists them; include the heartbeat timer and the owner question registration) with a deadline 15 minutes ahead and END YOUR TURN. The kernel wakes you with one turn per event.
11. On each wake: note in ${join(work, "orchestrator-state.json")} (append) the time, what woke you and what you did; ack; when the heartbeat has woken you at least twice, cancel it (chariox events_cancel_wake). Expect: w1's deadline (${workerDeadline}) ending w1, w2 finishing, the owner's answer.
12. When w1 and w2 are done (or ended) and the owner answered: copy w2's result.txt through the kernel file requests: kernel_request {"ReadKernelFile":{"path":"${join(root, "wt/w2/result.txt")}","offset":0,"length":4096}}, then {"WriteKernelFile":{"path":"${join(work, "collected-w2.txt")}","offset":0,"data_base64":"<the base64 you read>","total_size":<size>,"sha256":null}}.
13. Write ${join(work, "orchestrator-report.json")}: one entry per step with ok, tool and exact answer, and per wake what woke you. Delete the workers with \`agent delete wN\`. Reply ORCHDONE.
`
  const brief = scenario === "restart" ? restartBrief : scenario === "sudo" ? sudoBrief : `# Orchestrator brief (drill)
You are the ORCHESTRATOR of this Chariox Room (session ${R}). Run a small team inside THIS Room using only your own Chariox tools:
the room tools (chariox room run_command / commands docs / list_session_agents / send_agent_message / history), the chariox events tools
(subscribe, timer, yield, inbox, ack) and the chariox-reach tools (kernels, sessions, request, events_subscribe, events_wait, events_ack, guide).
Do not ask the owner anything. Never retry a refused call more than once; record the exact refusal code and message instead.
Kernel B id: ${b.status.daemon_id}. Deadline for both workers: unix ms ${deadline}.

Steps:
1. Spawn worker w1 in this Room with its OWN git worktree: room command \`agent spawn w1 --provider claude --model ${model} --effort low --worktree ${join(root, "wt/w1")} --branch w1\`.
2. Spawn worker w2 in this Room but running on kernel B: room command \`agent spawn w2 --provider claude --model ${model} --effort low --kernel ${b.status.daemon_id} --dir ${join(b.dir, "work/w2")}\`.
   If that is refused, record the answer and spawn w2 locally with \`--worktree ${join(root, "wt/w2")} --branch w2\`.
3. Give each worker the deadline: reach \`request\` with {"request":{"SetAgentDeadline":{"session_id":"${R}","agent_id":"<worker id>","deadline_ms":${deadline}}}}. Record the answer.
   Also call reach \`request\` once each with {"request":{"ListProviderAccountProfiles":null}} and {"request":{"GetProviderUsageSnapshot":{}}} and record the answers (usage pacing).
4. Give each worker its task with the room \`prompt\` command:
   w1: "Write the single word ALPHA into result.txt in your current working directory. Then call the chariox request_popup tool (wait true, timeout_sec 600) with title 'w1 commit', message 'Commit result.txt?', choices [{id:'yes',label:'Yes',reply:'yes'},{id:'no',label:'No',reply:'no'}]. If the reply is yes, run git add result.txt and git commit -m w1-result. End your reply with W1DONE."
   w2: "Run the shell command sleep 60. Then write the single word BETA into result.txt in your current working directory. End your reply with W2DONE."
5. Follow both workers and wait for them WITHOUT polling and without shell sleeps: first reach \`events_subscribe\` {"session_id":"${R}"} (no kernel argument: this Room is on your own kernel), then chariox events_subscribe with each worker's agent id, then chariox events_yield on those registrations with a deadline 15 minutes ahead and END YOUR TURN; the kernel wakes you with a new turn per event. In each wake, read reach \`events_wait\` {"timeout_seconds":1} to see what happened in the Room, act, ack, and yield again until both workers are done.
6. When w1's decision 'w1 commit' is open, answer it with choice yes: reach \`request\` {"request":{"RespondToInteraction":{"session_id":"${R}","interaction_id":"<id>","choice_id":"yes"}}}. Find the id from your events or reach \`request\` GetSessionState (active_interactions). If refused, try once through any other tool you have, then record what happened.
7. While w2 is still running, steer it: chariox send_agent_message to w2 with urgent true: "Also write the word GAMMA as a second line of result.txt." (or reach SteerActivePrompt).
8. When both are done, read each worker's result.txt (your shell for local paths, reach request ReadFile/RunShellCommand for kernel B) and write ${join(work, "orchestrator-report.json")}: one entry per step 1-8 with ok true/false, the tool you used and the exact answer when something was refused. Then delete both workers with the room command \`agent delete <alias>\`, record the answers in the report too, and reply ORCHDONE.
`
  writeFileSync(join(work, "BRIEF.md"), brief)
  let restarted = false
  const since = Date.now()
  if (scenario === "sudo") {
    const submitted = a.client.send(submitPromptRequest(R, attached, orch, `/sudo Read ${join(work, "BRIEF.md")} and carry it out now.`, []))
    submitted.catch(e => note("sudo-submit-error", { message: String(e.message).slice(0, 200) }))
    let popup
    await until(async () => (popup = (await state(a, R)).active_interactions?.find(x => String(x.id).startsWith("sudo:"))), "sudo popup", 60000)
    await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: popup.id, choice_id: popup.choices.find(c => c.id === "approve").id, custom_reply: "60", passkey: a.passkey } })
    note("sudo-approved", { title: popup.title })
  } else await a.rpc(submitPromptRequest(R, attached, orch, `Read ${join(work, "BRIEF.md")} and carry it out now.`, []))
  note("brief-submitted", { deadline })

  // Follow R like an outside driver would, recording every change; the owner answers a stalled decision only as a fallback.
  const seen = { agents: new Map(), interactions: new Map(), prompts: new Map() }
  const orchTurns = []
  let reportAt = null, fallbackAnswers = 0
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
    const hold = [...seen.interactions.entries()].find(([id, at]) => at && s?.active_interactions?.find(x => x.id === id && /w3 hold/.test(x.title ?? "")))
    const orchIdle = s && !s.prompt_states?.[orch]?.active_prompt
    if (scenario === "restart" && !restarted && hold && orchIdle && Date.now() - hold[1] > 45000) {
      restarted = true
      note("restart-begin", { orchestrator_idle: true, open_hold: hold[0] })
      await restart(a)
      const again = (await a.rpc({ AttachToSession: { session_id: R, client_id: "orch-owner-2", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
      await a.client.subscribeToKernelEvents(R, again)
      attachedNow = again
    }
    if (scenario === "restart" && restarted && s) for (const it of s.active_interactions ?? []) {
      if (/w3 hold/.test(it.title ?? "") && Date.now() - (seen.interactions.get(it.id) || Date.now()) > 60000) await a.rpc({ RespondToInteraction: { session_id: R, interaction_id: it.id, choice_id: "ok" } }).then(() => { fallbackAnswers++; seen.interactions.set(it.id, 0); note("owner-answer-after-restart", { id: it.id }) }, e => note("owner-answer-error", { message: e.message.slice(0, 200) }))
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
  for (const t of all.filter(t => t !== orchFile)) writeFileSync(join(evidence, `worker-tool-log-${t.file.split("/").pop()}.json`), JSON.stringify(toolLog(t.lines), null, 1), { mode: 0o600 })
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
    report: read(reportPath), w1: read(join(root, "wt/w1/result.txt")), w3: read(join(root, "wt/w3/result.txt")), w3_steer: read(join(root, "wt/w3/steer.txt")), collected: read(join(work, "collected-w2.txt")), orch_state: read(join(work, "orchestrator-state.json")), w2_local: read(join(root, "wt/w2/result.txt")), w2_b: read(join(b.dir, "work/w2/result.txt")), w1_branch_log: w1log,
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

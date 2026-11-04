// MP-08 / MP-09 / MP-10 / MP-11 agent reach: a real Claude Room agent on kernel A, without /sudo, reaches
// kernel B through its `chariox-reach` tools; B's audit and paired-clients list name the agent; what needs /sudo
// in the agent's own room is refused on B without /sudo and works inside a /sudo window; the "ask before agents acquire machines" setting turns an acquisition into one decision in the agent's room.
// Real kernels, local scoped relay, real Claude. Credentials are read inside this script, never printed or stored
// in evidence. Red on a base kernel without `chariox-reach`: the agent has no tool, so the B-side checks fail.
// The `tools` scenario runs with a real Claude, Codex or OpenCode agent (`--provider`): the agent calls every
// `chariox-reach` tool and a room tool given another kernel, and B's audit names it.
// The `events`, `driver` and `history` scenarios (S3) run with the same real agents on both kernels: X in a room on A
// follows Y in a room on B and is woken without polling when Y finishes a turn, and when Y raises a decision
// (AT-40) X reads it and answers it; X reads Y's room history with no approval.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHmac, createHash, randomBytes, randomUUID } from "node:crypto"
import { mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, statSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { resolve, join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { DatabaseSync } from "node:sqlite"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { ensureDirectory, launchedServers, photographTui, sha, toolNames } from "./agent-reach-tool-view.mjs"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
function required(key) {
  const value = flags.get(key)
  assert.equal(typeof value, "string", `${key} is required`)
  return resolve(value)
}
const binary = required("--kernel-binary"), relayBinary = required("--relay-binary")
const stateRoot = required("--state-root"), evidence = required("--evidence-dir")
const label = flags.get("--label") ?? "run"
const provider = flags.get("--provider") ?? "claude"
const providers = {
  claude: { model: "claude-haiku-5-5", variant: "low" },
  codex: { model: "gpt-6-luna", variant: "low" },
  opencode: { model: "opencode-go/deepseek-v4.1-flash", variant: null },
}
assert(providers[provider], `--provider is one of ${Object.keys(providers)}`)
const model = flags.get("--model") ?? providers[provider].model
const tokenFile = flags.get("--claude-token-file") ?? "/root/.claude-lanes/oauth-token"
// A Codex scope the product linked for drills, and the OpenCode login to copy into a lane-owned scope.
const codexScope = flags.get("--codex-scope") ?? "/root/.chariox/dev/worker-k/home/state/provider-accounts/local/codex/kaissandra-ai-reviewer-login-esqv7b7c/codex"
const opencodeAuth = flags.get("--opencode-auth") ?? "/root/.chariox/dev/provider-runtimes/fleet-accounts/opencode-home/.local/share/opencode/auth.json"
const scenarios = new Set((flags.get("--scenarios") ?? (provider === "claude" ? "reach,sudo,acquisition,tools,events,driver,history,view,humanonly,lineage,newrooms,inherit,workday,recall" : "tools,events,driver,history,view")).split(","))
assert(provider === "claude" || [...scenarios].every(name => ["tools", "view", "events", "driver", "history"].includes(name)), "only the tools, view, events, driver and history scenarios run with Codex or OpenCode")
const agentScenarios = ["events", "driver", "history"].some(name => scenarios.has(name))
assert(stateRoot.startsWith("/root/.chariox/dev/"), "state must be lane-owned")
assert(!evidence.startsWith(fileURLToPath(new URL("../../", import.meta.url))), "evidence must be outside the repository")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const root = mkdtempSync(join(stateRoot, "reach-"))
const children = [], clients = [], steps = []
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 2) + "\n", { mode: 0o600 })
  console.log(`MP-08/09/10/11 ${name}`)
}
const env = { PATH: process.env.PATH, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash" }
const secret = randomBytes(32).toString("base64url"), issuer = `agent-reach-${randomUUID()}`
function token(subject, kind, actions) {
  const head = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const body = Buffer.from(JSON.stringify({ iss: issuer, sub: subject, subject_kind: kind, realm_id: issuer,
    allowed_actions: actions, allowed_targets: null, iat: Math.floor(Date.now() / 1000) - 1,
    exp: Math.floor(Date.now() / 1000) + 7200, jti: randomUUID(), user_id: "local" })).toString("base64url")
  const input = `${head}.${body}`
  return `${input}.${createHmac("sha256", secret).update(input).digest("base64url")}`
}
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
}
async function until(check, description, timeout = 30000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await check()) return; await delay(250) }
  throw new Error(`timeout: ${description}`)
}
function start(program, extra, name) {
  const log = openSync(join(evidence, `${name}.log`), "a", 0o600)
  const child = spawn(program, [], { env: { ...env, ...extra }, stdio: ["ignore", log, log] })
  assert(Number.isSafeInteger(child.pid) && child.pid > 1)
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
  const localEnv = { ...env, HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"),
    XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join("/var/tmp/rch", `${name}-${process.pid}.sock`), CHARIOX_DAEMON_ALIAS: `reach-${name}`, CHARIOX_MACHINE_ALIAS: `reach-${name}`,
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
  mkdirSync("/var/tmp/rch", { recursive: true })
  const r = { name, dir, env: localEnv, passkey: randomBytes(24).toString("hex") }
  r.child = start(binary, localEnv, name)
  r.client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: localEnv, controlRequestRetryDeadlineMs: 1000 })
  clients.push(r.client)
  r.rpc = async request => {
    const result = await r.client.send(request)
    if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code} ${result.Error.message ?? ""}`)
    return result
  }
  await until(async () => { try { return !!(await r.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, `${name} ready`)
  r.status = (await r.rpc({ RelayStatus: null })).RelayStatus.status
  note("kernel-ready", { role: name, kernel: r.status.daemon_id, opencode_port: ports[2], mcp_port: ports[1] })
  return r
}
const state = async (r, session) => (await r.rpc({ GetSessionState: { session_id: session } })).SessionState.session
async function unlock(r, session, agent) {
  const pending = r.rpc({ ManageCredentialVault: { session_id: session, agent_id: agent } })
  pending.catch(() => {})
  let interaction
  await until(async () => {
    interaction = (await state(r, session)).active_interactions?.find(x => x.id.startsWith("vault-unlock-"))
    return interaction
  }, "disposable vault prompt")
  await r.rpc({ RespondToInteraction: { session_id: session, interaction_id: interaction.id, choice_id: "passphrase", custom_reply: r.passkey } })
  await pending
}
const failures = []
function check(name, ok, detail = {}) {
  note(ok ? "check-pass" : "check-FAIL", { check: name, ...detail })
  if (!ok) failures.push(name)
}
// Everything the Claude profile of kernel A recorded, to see which reach tools the agent called and what they answered.
function claudeTranscript(home, sinceMs) {
  const out = []
  const walk = dir => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.name.endsWith(".jsonl") && statSync(full).mtimeMs >= sinceMs) out.push(readFileSync(full, "utf8"))
    }
  }
  try { walk(join(home, "../home/.claude/projects")) } catch {}
  return out.join("\n")
}
// The agent's tool calls and what they answered, as the evidence of what it really did.
function toolLog(text) {
  const rows = []
  for (const line of text.split("\n")) {
    let entry
    try { entry = JSON.parse(line) } catch { continue }
    const content = entry.message?.content
    if (!Array.isArray(content)) continue
    for (const part of content) {
      if (part.type === "tool_use") rows.push({ call: part.name, input: part.input })
      else if (part.type === "tool_result") rows.push({ answer: String(typeof part.content === "string" ? part.content : JSON.stringify(part.content)).slice(0, 600) })
      else if (part.type === "text" && entry.message.role === "assistant") rows.push({ said: part.text.slice(0, 300) })
    }
  }
  return rows
}
const stripped = text => text.replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")
let saveToolLog = () => {}
async function main() {
  note("binaries", { kernel: createHash("sha256").update(readFileSync(binary)).digest("hex"), relay: createHash("sha256").update(readFileSync(relayBinary)).digest("hex") })
  const relayPort = await freePort(), relayUrl = `ws://127.0.0.1:${relayPort}`
  mkdirSync(join(root, "relay"), { mode: 0o700 })
  start(relayBinary, { HOME: root, CHARIOX_HOME: join(root, "relay"), CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_SCOPED_ISSUER: issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }, "relay")
  const a = await role("a"), b = await role("b")
  for (const r of [a, b]) {
    await r.rpc({ ConfigureRelay: { relay_url: relayUrl, relay_token: token(r.status.daemon_id, "kernel", ["daemon.register", "daemon.heartbeat", "packet.route", "peer.request", "peer.event"]) } })
    await until(async () => (await r.rpc({ RelayStatus: null })).RelayStatus.status.connected, `${r.name} relay registration`)
  }
  const pairing = (await b.rpc({ CreateTerminalPairingLink: { terminal_type: "cli", expires_in_ms: 300000 } })).TerminalPairingLinkCreated.pairing
  await a.rpc({ JoinKernelLink: { link: pairing.pairing_link, relay_token: token("reach-operator", "client", ["client.connect", "client.metadata.read", "packet.route"]) } })
  note("paired", { from: "a", to: "b" })
  const target = b.status.daemon_id

  // Each kernel: a vault (its passphrase is the /sudo passkey) and the Claude profile.
  const oauth = provider === "claude" ? readFileSync(tokenFile, "utf8").trim() : null
  async function provision(r) {
    if (provider === "codex") {
      const profile = (await r.rpc({ LinkProviderAccountProfile: { provider, label: "reach drill", path: codexScope } })).ProviderAccountProfile.profile.profile_id
      await r.rpc({ GetProviderAuthStatus: { provider, account_profile: profile } })
      note("provider-profile-linked", { kernel: r.name, provider })
      return profile
    }
    if (provider === "opencode") {
      const scope = join(r.dir, "opencode-scope"), data = join(scope, "data/opencode")
      mkdirSync(data, { recursive: true, mode: 0o700 })
      writeFileSync(join(data, "auth.json"), readFileSync(opencodeAuth), { mode: 0o600 })
      const profile = (await r.rpc({ LinkProviderAccountProfile: { provider, label: "reach drill", path: scope } })).ProviderAccountProfile.profile.profile_id
      note("provider-profile-linked", { kernel: r.name, provider })
      return profile
    }
    const made = (await r.rpc({ CreateSession: { workspace_id: join(r.dir, "work"), worktree_id: join(r.dir, "work"), alias: "vault-room" } })).SessionCreated
    const att = (await r.rpc({ AttachToSession: { session_id: made.session.id, client_id: "reach-setup", capability_level: "FullTerminal" } })).SessionAttached
    await r.client.subscribeToKernelEvents(made.session.id, att.attachment.id)
    await unlock(r, made.session.id, made.agent.id)
    const credential = await r.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: oauth, overwrite: true } })
    note("claude-profile-stored", { kernel: r.name, stored: !!credential.ProviderAccountCredentialStored })
    return "default"
  }
  const accountProfile = await provision(a)
  // The Claude scenarios and the S3 scenarios run an agent on kernel B too; for the tools scenario B is a passive target.
  const accountProfileB = provider === "claude" || agentScenarios ? await provision(b) : accountProfile
  const agentDefaults = { provider, account_profile: accountProfile, model, effort: providers[provider].variant, permission_level: "yolo" }
  const agentDefaultsB = { ...agentDefaults, account_profile: accountProfileB }
  // Kernel B: the room another room's agent will message, and the room the agent must not rename without /sudo.
  const roomB = (await b.rpc({ CreateSession: { workspace_id: join(b.dir, "work"), worktree_id: join(b.dir, "work"), alias: "target-room", agent_defaults: provider === "claude" || agentScenarios ? agentDefaultsB : undefined } })).SessionCreated
  const roomA = (await a.rpc({ CreateSession: { workspace_id: join(a.dir, "work"), worktree_id: join(a.dir, "work"), alias: "home-room", agent_defaults: agentDefaults } })).SessionCreated
  const siblingA = (await a.rpc({ CreateSession: { workspace_id: join(a.dir, "work"), worktree_id: join(a.dir, "work"), alias: "sibling-room", agent_defaults: agentDefaults } })).SessionCreated
  const attached = (await a.rpc({ AttachToSession: { session_id: roomA.session.id, client_id: "reach-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  const agentA = roomA.agent.id
  await a.rpc({ LaunchProviderRun: { session_id: roomA.session.id, agent_id: agentA, adapter_key: provider, provider, account_profile: accountProfile, model, variant: providers[provider].variant, native_tui: false } })
  const startedAt = Date.now()
  if (flags.has("--hold-start")) { note("holding", {}); await delay(Number(flags.get("--hold-start")) * 1000) }
  saveToolLog = () => writeFileSync(join(evidence, "agent-tool-log.json"), stripped(JSON.stringify(toolLog(claudeTranscript(a.dir + "/chariox", startedAt)), null, 1)), { mode: 0o600 })

  async function idle(timeout = 240000) {
    await until(async () => {
      await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {})
      const s = await state(a, roomA.session.id)
      const agent = (s.agents ?? []).find(x => x.id === agentA)
      const ps = s.prompt_states?.[agentA] ?? {}
      return !(ps.active_prompt ?? null) && !(ps.queued_prompts ?? []).length && agent && !agent.is_processing && agent.state !== "Working"
    }, "agent idle", timeout)
  }
  // The agent ends its turn with a marker the prompt never spells out, so an idle agent before its turn started is not mistaken for a finished one.
  async function finished(marker, since) {
    // A turn that never reaches its marker is reported by the checks that follow, not by a thrown timeout.
    await until(async () => {
      await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {})
      return claudeTranscript(a.dir + "/chariox", since).includes(marker)
    }, `agent turn (${marker})`, 150000).catch(error => note("turn-timeout", { message: error.message }))
    await idle()
  }
  async function sendPrompt(text, marker) {
    const since = Date.now() - 1000
    await a.rpc(submitPromptRequest(roomA.session.id, attached, agentA, text, []))
    await finished(marker, since)
  }
  async function sudoPrompt(text, marker) {
    const since = Date.now() - 1000
    const submitted = a.client.send(submitPromptRequest(roomA.session.id, attached, agentA, `/sudo ${text}`, []))
    submitted.catch(error => note("sudo-submit-error", { message: String(error.message).slice(0, 300) }))
    let popup
    await until(async () => {
      await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {})
      popup = (await state(a, roomA.session.id)).active_interactions?.find(x => String(x.id).startsWith("sudo:"))
      return popup
    }, "sudo popup", 60000).catch(async error => {
      note("sudo-popup-missing", { interactions: ((await state(a, roomA.session.id)).active_interactions ?? []).map(x => ({ id: x.id, title: x.title, message: String(x.message ?? "").slice(0, 120) })) })
      throw error
    })
    note("sudo-popup", { title: popup.title, choices: popup.choices.map(c => ({ id: c.id, label: c.label, requires_passkey: !!c.requires_passkey })) })
    const approve = popup.choices.find(c => c.id === "approve")
    await a.rpc({ RespondToInteraction: { session_id: roomA.session.id, interaction_id: popup.id, choice_id: approve.id, custom_reply: "60", passkey: a.passkey } })
    await submitted
    await finished(marker, since)
  }
  const audit = () => {
    const db = new DatabaseSync(join(b.dir, "chariox/state.db"), { readOnly: true })
    try { return db.prepare("SELECT payload_json FROM durable_state_events WHERE kind='kernel_gateway.request'").all().map(r => JSON.parse(r.payload_json)) } finally { db.close() }
  }
  const bState = () => state(b, roomB.session.id)
  const call = (kernelArg, request) => JSON.stringify(kernelArg ? { kernel: kernelArg, request } : { request })

  // The agent's latest turn from the kernel's own history, the same for every provider: its tool calls (keyed, in order) with their answers, and its last words.
  const turnOf = async (r, session, agent) => {
    const outline = (await r.rpc({ GetSessionHistoryOutline: { session_id: session, agent_ids: [agent], latest_prompt_count: 1, include_prefetched_content: true } })).SessionHistoryOutline
    const history = outline.agents.find(x => x.agent_id === agent)
    const turn = history?.turns?.at(-1)
    const calls = new Map()
    for (const blob of history?.prefetched_blobs ?? []) for (const { entry } of blob.entries) {
      if (entry.kind !== "provider_tool") continue
      let call
      try { call = JSON.parse(entry.text) } catch { continue }
      calls.set(entry.merge_key, call)
    }
    if (process.env.REACH_DRILL_RAW) writeFileSync(join(evidence, "outline-raw.json"), JSON.stringify(outline, null, 1))
    return { id: turn?.turn_id, done: turn?.lifecycle === "completed", answer: turn?.summary?.entry?.text ?? "", calls: [...calls.values()], keyed: [...calls.entries()] }
  }
  // Claude and OpenCode put the server in the tool name; Codex names the server in the call's title.
  const reachName = call => /chariox[-_]reach/.test(call.tool) ? call.tool.replace(/^.*chariox[-_]reach[._-]*/, "") : /chariox[-_]reach/.test(call.title ?? "") ? call.tool : null

  if (scenarios.has("reach")) {
    note("scenario", { which: "reach" })
    await sendPrompt([
      "This is a drill. Use only the chariox-reach MCP tools (tools `kernels` and `request`). No shell, no files, no other tools. Do not retry a failed call.",
      "1. Call `kernels` once.",
      `2. Call \`request\` with exactly this JSON argument: ${call(target, { ListSessions: null })}`,
      `3. Call \`request\` with exactly: ${call(target, { AttachToSession: { session_id: roomB.session.id, client_id: "reach-agent", capability_level: "FullTerminal" } })}`,
      `4. Take the attachment id from the step 3 answer (SessionAttached.attachment.id) and call \`request\` with: {"kernel":"${target}","request":{"SubmitPrompt":{"session_id":"${roomB.session.id}","attachment_id":"<that id>","target_agent_id":"${roomB.agent.id}","prompt":"hello from the reach drill","attachments":[]}}}`,
      `5. Without a \`kernel\` argument (this kernel), call \`request\` with: {"request":{"AttachToSession":{"session_id":"${siblingA.session.id}","client_id":"reach-agent","capability_level":"FullTerminal"}}} and then {"request":{"SubmitPrompt":{"session_id":"${siblingA.session.id}","attachment_id":"<that id>","target_agent_id":"${siblingA.agent.id}","prompt":"hello sibling from the reach drill","attachments":[]}}}`,
      "Then reply with the words REACH and DONE joined by an underscore, nothing else.",
    ].join("\n"), "REACH_DONE")
    const text = claudeTranscript(a.dir + "/chariox", startedAt)
    
    const listed = text.includes("target-room") && text.includes(roomB.session.id)
    check("the agent lists B's sessions without /sudo", listed)
    const rows = audit()
    const named = rows.filter(row => String(row.harness ?? "").includes("(Chariox agent)") && String(row.harness).startsWith("home-room/"))
    check("B's audit names the room and agent", named.some(row => row.request === "SubmitPrompt"), { harnesses: [...new Set(rows.map(r => r.harness))], requests: rows.map(r => r.request) })
    // B's agent either still shows the prompt in its room or already answered it in its own provider transcript.
    const arrived = text => until(async () => { await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {}); return JSON.stringify(await bState()).includes(text) || claudeTranscript(b.dir + "/chariox", 0).includes(text) }, text, 60000).then(() => true, () => false)
    check("the prompt reached the agent of the other room on B", await arrived("hello from the reach drill"))
    // The sibling's prompt is in its room's state while it runs and in its history once the turn is recorded.
    const siblingHas = async text => {
      await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {})
      const outline = await a.rpc({ GetSessionHistoryOutline: { session_id: siblingA.session.id, agent_ids: [siblingA.agent.id], latest_prompt_count: 4, include_prefetched_content: true } }).catch(() => ({}))
      return JSON.stringify(await state(a, siblingA.session.id)).includes(text) || JSON.stringify(outline).includes(text)
    }
    check("the agent messaged an agent in another room on its own kernel", await until(() => siblingHas("hello sibling from the reach drill"), "sibling prompt", 90000).then(() => true, () => false))
    const paired = (await b.rpc({ ListPairedClients: {} })).PairedClientsListed.clients
    check("B's paired-clients list shows the gateway row with the agent", paired.some(c => String(c.alias ?? "").includes("(Chariox agent)")), { aliases: paired.map(c => c.alias) })
  }

  if (scenarios.has("tools")) {
    note("scenario", { which: "tools", provider, model })
    const latestTurn = () => turnOf(a, roomA.session.id, agentA)
    async function toolsTurn(prompt, marker) {
      const before = (await latestTurn()).id
      await a.rpc(submitPromptRequest(roomA.session.id, attached, agentA, prompt, []))
      let turn
      await until(async () => {
        await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {})
        turn = await latestTurn()
        return turn.id !== before && turn.done && turn.answer.includes(marker)
      }, `agent turn (${marker})`, 240000).catch(error => note("turn-timeout", { message: error.message, answer: turn?.answer?.slice(0, 200) }))
      const reach = turn.calls.filter(call => reachName(call) !== null).map(call => ({ tool: reachName(call), input: call.input, status: call.status, output: String(call.output ?? call.error ?? "") }))
      writeFileSync(join(evidence, `tools-${marker}.json`), JSON.stringify({ provider, model, answer: turn.answer.slice(0, 600), calls: turn.calls.map(call => ({ tool: call.tool, server: call.title, input: call.input, status: call.status, output: String(call.output ?? call.error ?? "").slice(0, 1200) })) }, null, 1), { mode: 0o600 })
      return { ...turn, reach }
    }
    const answered = (turn, name) => turn.reach.find(call => call.tool === name && call.status === "completed" && call.output)
    const first = await toolsTurn([
      "This is a drill. Use only MCP tools of the servers chariox and chariox-reach. No shell, no files. Do not retry a failed call.",
      "1. Call the chariox-reach tool `kernels`.",
      `2. Call the chariox-reach tool \`sessions\` with exactly: {"kernel":"${target}"}`,
      `3. Call the chariox-reach tool \`request\` with exactly: ${call(target, { ListSessions: null })}`,
      `4. Call the chariox-reach tool \`request\` with exactly: ${call(target, { AttachToSession: { session_id: roomB.session.id, client_id: "reach-agent", capability_level: "FullTerminal" } })}`,
      "5. Call the chariox-reach tool `guide`.",
      `6. Call the chariox tool \`list_session_agents\` and add the argument "kernel":"${target}" to it, even if its schema does not list it.`,
      "Then reply with the words TOOLS and DONE joined by an underscore, nothing else.",
    ].join("\n"), "TOOLS_DONE")
    const names = [...new Set(first.reach.map(call => call.tool))]
    note("reach-tools-called", { names })
    check("the agent sees the chariox-reach group and calls kernels, sessions, request and guide", ["kernels", "sessions", "request", "guide"].every(name => answered(first, name)), { names })
    check("sessions lists B's rooms", !!answered(first, "sessions")?.output.includes(roomB.session.id))
    check("request reaches kernel B", !!answered(first, "request")?.output.includes("SessionsListed") && !!answered(first, "request")?.output.includes(roomB.session.id))
    check("guide answers with the reach guide", !!answered(first, "guide")?.output.includes("Chariox reach guide"))
    const roomCall = first.calls.find(call => /list_session_agents/.test(call.tool) && reachName(call) === null)
    const roomOutput = String(roomCall?.output ?? roomCall?.error ?? "")
    note("room-tool-with-kernel", { called: !!roomCall, input: roomCall?.input, status: roomCall?.status, output: roomOutput.slice(0, 300) })
    check("a room tool given another kernel is refused", !!roomCall && roomCall.input?.kernel === target && /chariox-reach/.test(roomOutput) && !roomOutput.includes("\"agents\""), { provider_dropped_the_argument: !!roomCall && roomCall.input?.kernel !== target })
    const rows = audit().filter(row => String(row.harness ?? "").startsWith("home-room/") && String(row.harness).includes("(Chariox agent)"))
    check("B's audit names the room and agent for the request", rows.some(row => row.request === "AttachToSession"), { requests: rows.map(row => row.request) })
    const second = await toolsTurn([
      "This is a drill. Use only the chariox-reach MCP tools. No shell, no files. A call may answer with an error result: that is expected, do not retry it and make all three calls in order anyway.",
      `1. Call \`events_subscribe\` with exactly: {"kernel":"${target}","session_id":"${roomB.session.id}"}`,
      '2. Call `events_wait` with exactly: {"timeout_seconds":2}',
      '3. Call `events_ack` with exactly: {"cursor":0}',
      "Then reply with the words EVENTS and DONE joined by an underscore, nothing else.",
    ].join("\n"), "EVENTS_DONE")
    const events = ["events_subscribe", "events_wait", "events_ack"].map(name => second.reach.find(call => call.tool === name))
    note("events-tools", { calls: events.map(call => call && { tool: call.tool, status: call.status, output: call.output.slice(0, 200) }) })
    check("the agent calls the three events tools and each is answered (S3: following is reach-level)", events[0]?.output.includes("EventSubscription") && events[1]?.output.includes("EventsListed") && events[2]?.output.includes("EventsAcknowledged"), { outputs: events.map(call => call?.output.slice(0, 120)) })
    if (flags.has("--hold")) await delay(Number(flags.get("--hold")) * 1000)
  }


  if (agentScenarios) {
    // Agent X (room 1 on A) follows agent Y (room 2 on B). Rooms Z1 (on A) and Z2 (on B) are never followed.
    const attachB = (await b.rpc({ AttachToSession: { session_id: roomB.session.id, client_id: "reach-owner-b", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    const roomZ2 = (await b.rpc({ CreateSession: { workspace_id: join(b.dir, "work"), worktree_id: join(b.dir, "work"), alias: "unfollowed-room-b", agent_defaults: agentDefaultsB } })).SessionCreated
    const attachZ2 = (await b.rpc({ AttachToSession: { session_id: roomZ2.session.id, client_id: "reach-owner-z2", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    const attachZ1 = (await a.rpc({ AttachToSession: { session_id: siblingA.session.id, client_id: "reach-owner-z1", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    const pumped = [[a, roomA.session.id, attached], [b, roomB.session.id, attachB], [b, roomZ2.session.id, attachZ2], [a, siblingA.session.id, attachZ1]]
    const keepAlive = setInterval(() => { for (const [r, session, attachment] of pumped) r.rpc({ PumpTerminalOutput: { session_id: session, attachment_id: attachment } }).catch(() => {}) }, 2000).unref()
    const nonce = randomBytes(3).toString("hex")
    const yAgent = roomB.agent.id
    // The drill reads the kernels' history while it waits; a slow read keeps that load low so it does not slow what is timed (the kernels' own log is the clock).
    const sleepMs = 1000
    // One real turn of another agent, to its end.
    async function agentTurn(r, session, attachment, agent, prompt, marker) {
      const before = (await turnOf(r, session, agent)).id
      await r.rpc(submitPromptRequest(session, attachment, agent, prompt, []))
      let turn
      const end = Date.now() + 240000
      for (;;) {
        turn = await turnOf(r, session, agent)
        if (turn.id !== before && turn.done && turn.answer.includes(marker)) break
        if (Date.now() > end) throw new Error(`timeout: turn of ${agent} (${marker})`)
        await delay(sleepMs)
      }
      return turn
    }
    // X's turn: the prompt is submitted, `during` runs beside it (the drill's triggers), and every reach call is stamped the first time it is seen completed.
    async function xTurn(prompt, marker, during) {
      const before = (await turnOf(a, roomA.session.id, agentA)).id
      await a.rpc(submitPromptRequest(roomA.session.id, attached, agentA, prompt, []))
      const seen = new Map(), running = new Map()
      let turn, side
      const poll = async () => {
        turn = await turnOf(a, roomA.session.id, agentA)
        if (turn.id === before) return false
        for (const [key, call] of turn.keyed) {
          const name = reachName(call)
          if (name === null) continue
          if (call.status === "completed") { running.delete(key); if (!seen.has(key)) seen.set(key, { at: Date.now(), name }) }
          else running.set(key, { name, input: call.input })
        }
        return turn.done && turn.answer.includes(marker)
      }
      const sideRun = during({ seen, running, waitFor: async (check, what, timeout = 120000) => { const end = Date.now() + timeout; while (Date.now() < end) { await poll(); if (await check()) return; await delay(sleepMs) } throw new Error(`timeout: ${what}`) } }).then(value => { side = { value } }, error => { side = { error } })
      const end = Date.now() + 360000
      while (Date.now() < end) { if (await poll()) break; await delay(sleepMs) }
      await sideRun
      if (side?.error) note("trigger-error", { message: String(side.error.message ?? side.error).slice(0, 300) })
      const reach = turn.keyed.filter(([, call]) => reachName(call) !== null).map(([key, call]) => ({ key, tool: reachName(call), input: call.input, status: call.status, output: String(call.output ?? call.error ?? ""), seenAt: seen.get(key)?.at }))
      writeFileSync(join(evidence, `agent-${marker}.json`), JSON.stringify({ provider, model, answer: turn.answer.slice(0, 600), calls: turn.keyed.map(([, call]) => ({ tool: call.tool, server: call.title, input: call.input, status: call.status, output: String(call.output ?? call.error ?? "").slice(0, 2500) })) }, null, 1), { mode: 0o600 })
      return { turn, reach, side }
    }
    // X has subscribed, passed its two short steps and sits in the long wait (seen running, or the history shows tool calls only when they end).
    const inLongWait = ({ seen, running }) => () => {
      const acks = [...seen.values()].filter(x => x.name === "events_ack")
      return [...seen.values()].some(x => x.name === "events_subscribe") && ([...running.values()].some(x => x.name === "events_wait" && Number(x.input?.timeout_seconds) >= 100) || (acks.length > 0 && Date.now() - acks[0].at > 5000))
    }
    // When the kernel finished each reach tool call: its own log line, so the number is the product's and not the provider's reporting delay.
    const reachFinishes = () => {
      const out = [], dir = join(a.dir, "logs")
      for (const file of readdirSync(dir)) for (const line of readFileSync(join(dir, file), "utf8").split("\n")) {
        if (!line.includes("reach tool call finished")) continue
        try { const entry = JSON.parse(line); out.push({ tool: entry.tool, ok: entry.ok, elapsed_ms: entry.elapsed_ms, ts: entry.timestamp_ms }) } catch {}
      }
      return out
    }
    // The wait that was open when the record was published returned `lag` ms later. A record published while the agent was between two
    // calls (thinking) is no wake and gives null: the drill keeps the driven agent busy long enough for the follower to be waiting.
    const kernelLag = at => { const done = reachFinishes().filter(x => x.tool === "events_wait" && x.ts >= at - 50 && x.ts - x.elapsed_ms <= at + 25).sort((p, q) => p.ts - q.ts)[0]; return done ? done.ts - at : null }
    // A tool answer is the payload itself (Claude, OpenCode) or an MCP result wrapping it as text (Codex).
    const json = text => {
      const from = text.indexOf("{"), to = text.lastIndexOf("}")
      let value
      try { value = JSON.parse(text.slice(from, to + 1)) } catch { return null }
      const wrapped = value?.content?.[0]?.text
      if (typeof wrapped !== "string") return value
      try { return JSON.parse(wrapped) } catch { return null }
    }
    // The records a wait call returned, with the call's own timing.
    const waited = reach => reach.filter(call => call.tool === "events_wait" && call.status === "completed").map(call => ({ ...call, batch: json(call.output)?.EventsListed ?? null }))
    const cardsAnywhere = async () => {
      const rooms = [[a, roomA.session.id], [a, siblingA.session.id], [b, roomB.session.id], [b, roomZ2.session.id]]
      return (await Promise.all(rooms.map(async ([r, session]) => ((await state(r, session)).active_interactions ?? []).map(x => ({ room: session, id: x.id, title: x.title }))))).flat()
    }

    if (scenarios.has("events")) {
      note("scenario", { which: "events", provider, model })
      const yWord = `PONG${nonce}`, z1Word = `ZONE${nonce}`, z2Word = `ZTWO${nonce}`
      let events = { z: null, yDone: null }
      const first = await xTurn([
        "This is a drill. Use only the chariox-reach MCP tools. No shell, no files. A call may be refused: do not retry it, except the waits below.",
        `1. Call events_subscribe with exactly: {"kernel":"${target}","session_id":"${roomB.session.id}"}`,
        '2. Call events_wait with exactly {"timeout_seconds":3}, then events_ack with its next_cursor (those records are old).',
        '3. Call events_wait with {"timeout_seconds":120}, then events_ack with its next_cursor. Repeat step 3 (at most 8 times) until a returned record has a value.outcome field; a record without it, such as run_state working, is not the end: wait again. Do not reply before that.',
        "Then reply with the words EVENTS and DONE joined by an underscore, nothing else.",
      ].join("\n"), "EVENTS_DONE", async ({ running, seen, waitFor }) => {
        await waitFor(inLongWait({ seen, running }), "X waits")
        await delay(1500)
        // Rooms X does not follow finish real turns: a room on its own kernel and a room on the followed kernel.
        const outsideStart = Date.now()
        await Promise.all([
          agentTurn(a, siblingA.session.id, attachZ1, siblingA.agent.id, `Reply with exactly ${z1Word} and nothing else.`, z1Word),
          agentTurn(b, roomZ2.session.id, attachZ2, roomZ2.agent.id, `Reply with exactly ${z2Word} and nothing else.`, z2Word),
        ])
        events.z = { start: outsideStart, done: Date.now() }
        await delay(3500)
        events.zQuiet = ![...seen.values()].some(x => x.name === "events_wait" && x.at >= outsideStart)
        events.yStart = Date.now()
        await agentTurn(b, roomB.session.id, attachB, yAgent, `Run the shell command "sleep 10" first. Then reply with exactly ${yWord} and nothing else.`, yWord)
        events.yDone = Date.now()
      })
      const waits = waited(first.reach)
      note("events-waits", { count: waits.length, calls: waits.map(w => ({ at: w.seenAt, n: w.batch?.events?.length, types: w.batch?.events?.map(e => `${e.type}:${e.value?.run_state ?? e.value?.state ?? ""}${e.value?.outcome ? ":" + e.value.outcome.kind : ""}`) })) })
      const finalWait = waits.find(w => w.batch?.events?.some(e => e.value?.outcome))
      const finalEvent = finalWait?.batch?.events?.findLast(e => e.value?.outcome)
      const sessionsSeen = new Set(waits.flatMap(w => w.batch?.events ?? []).map(e => e.session_id))
      check("X follows a room of another kernel: subscribe is answered", !!first.reach.find(c => c.tool === "events_subscribe" && c.output.includes("EventSubscription")), { out: first.reach.find(c => c.tool === "events_subscribe")?.output.slice(0, 200) })
      check("nothing from the rooms X does not follow wakes it (two other rooms finished real turns while it waited)", !!events.z && events.zQuiet === true && [...sessionsSeen].every(id => id === roomB.session.id), { sessions: [...sessionsSeen], z_turns_ms: events.z && events.z.done - events.z.start })
      const seenLag = finalWait && finalEvent ? finalWait.seenAt - finalEvent.at_ms : null
      const latency = finalEvent ? kernelLag(finalEvent.at_ms) : null
      note("events-latency", { event_at_ms: finalEvent?.at_ms, y_done_ms: events.yDone, kernel_latency_ms: latency, provider_shows_result_after_ms: seenLag, polling_ms: sleepMs })
      check("X's events_wait returns Y's finished turn within 2 s (outcome record names kernel, session, agent)", latency !== null && latency <= 2000 && finalEvent.session_id === roomB.session.id && finalEvent.agent_id === yAgent && !!finalEvent.kernel && finalEvent.value.outcome !== "error" && latency >= -250, { kernel_latency_ms: latency, provider_shows_result_after_ms: seenLag, record: finalEvent && { kernel: finalEvent.kernel, session_id: finalEvent.session_id, agent_id: finalEvent.agent_id, value: finalEvent.value } })
      check("X was woken by the wait, not polling (few waits, each one reported)", waits.length >= 2 && waits.length <= 10, { waits: waits.length })
      check("X's turn ended with the words it was asked for", first.turn.answer.includes("EVENTS_DONE"))
    }

    if (scenarios.has("driver")) {
      note("scenario", { which: "driver (AT-40)", provider, model })
      const ask = `Ship the drill build ${nonce}?`
      let raised = null
      const driver = await xTurn([
        "This is a drill. Use only the chariox-reach MCP tools. No shell, no files. A call may be refused: do not retry it, except the waits below.",
        `1. Call events_subscribe with exactly: {"kernel":"${target}","session_id":"${roomB.session.id}","agent_id":"${yAgent}"}`,
        '2. Call events_wait with exactly {"timeout_seconds":3}, then events_ack with its next_cursor (those records are old).',
        '3. Call events_wait with {"timeout_seconds":120}. Repeat it (at most 8 times) until a returned record has a value.question field; call events_ack with the cursor after each wait. A record without value.question is not the end: wait again. Do not reply before that.',
        `4. That record is a decision the person asked you to settle: choose the choice whose id is "ship". Call request with {"kernel":"<the record's kernel field>","request":{"RespondToInteraction":{"session_id":"<the record's session_id>","interaction_id":"<the record's refs.interaction_id>","choice_id":"ship"}}}.`,
        "Then reply with the words DRIVER and DONE joined by an underscore, nothing else.",
      ].join("\n"), "DRIVER_DONE", async ({ running, seen, waitFor }) => {
        await waitFor(inLongWait({ seen, running }), "driver waits")
        await delay(1500)
        raised = { start: Date.now() }
        // The driven agent itself raises the decision through its own chariox tool and waits for the answer.
        const turn = await agentTurn(b, roomB.session.id, attachB, yAgent, [`Run the shell command "sleep 12" first. Then use the chariox tool request_popup exactly once, with wait true: message "${ask}", choices [{"id":"ship","label":"Ship","reply":"ship"},{"id":"hold","label":"Hold","reply":"hold"}].`, "When it returns, reply with exactly ANSWER_ followed by the choice id you received, nothing else."].join(" "), "ANSWER_")
        raised.done = Date.now()
        raised.answer = turn.answer
      })
      const waits = waited(driver.reach)
      const cardWait = waits.find(w => w.batch?.events?.some(e => e.value?.question))
      const card = cardWait?.batch?.events?.find(e => e.value?.question)
      note("driver-card", { record: card && { kernel: card.kernel, session_id: card.session_id, agent_id: card.agent_id, refs: card.refs, message: card.value.question.message, choices: card.value.question.choices.map(c => c.id) } })
      const seenLag = cardWait && card ? cardWait.seenAt - card.at_ms : null
      const latency = card ? kernelLag(card.at_ms) : null
      check("X is woken by the decision Y raised (record names kernel, session, agent and interaction)", !!card && card.kernel === b.status.daemon_id && card.session_id === roomB.session.id && card.agent_id === yAgent && !!card.refs?.interaction_id && card.value.question.message.includes(ask), { kernel_latency_ms: latency, provider_shows_result_after_ms: seenLag })
      check("the wake comes within 2 s of the decision being raised, without polling", latency !== null && latency <= 2000 && latency >= -250 && waits.length <= 6, { kernel_latency_ms: latency, provider_shows_result_after_ms: seenLag, waits: waits.length })
      const answer = driver.reach.find(c => c.tool === "request" && /RespondToInteraction/.test(JSON.stringify(c.input)))
      check("X answers the card through reach on kernel B", !!answer && answer.status === "completed" && answer.output.includes("InteractionResponded") && answer.input?.request?.RespondToInteraction?.interaction_id === card?.refs?.interaction_id, { input: answer?.input, out: answer?.output.slice(0, 200) })
      check("Y continues with the answer X gave (no person answered)", !!raised?.answer?.includes("ANSWER_ship"), { answer: raised?.answer?.slice(0, 100) })
      check("no card is left open in any room", (await cardsAnywhere()).length === 0, { cards: await cardsAnywhere() })
      const rows = audit().filter(row => String(row.harness ?? "").startsWith("home-room/") && String(row.harness).includes("(Chariox agent)"))
      check("B's audit names the room and agent that answered", rows.some(row => row.request === "RespondToInteraction"), { requests: rows.map(row => row.request) })
    }

    if (scenarios.has("history")) {
      note("scenario", { which: "history", provider, model })
      // Y has answered something in its own room by now; make sure of it for a run of this scenario alone.
      const hasHistory = (await turnOf(b, roomB.session.id, yAgent)).answer
      const yWord = hasHistory.trim() ? hasHistory.trim().split(/\s+/).at(-1) : null
      const word = yWord ?? `HIST${nonce}`
      if (!yWord) await agentTurn(b, roomB.session.id, attachB, yAgent, `Reply with exactly ${word} and nothing else.`, word)
      const secret = (await turnOf(b, roomB.session.id, yAgent)).answer
      const read = await xTurn([
        "This is a drill. Use the tools of the servers chariox and chariox-reach. No shell, no files. A call may be refused: do not retry it.",
        `1. Call the chariox tool whose name ends in history.search (or history_search) with {"session_id":"${roomB.session.id}","query":"${word}"}.`,
        `2. Call the chariox-reach tool request with exactly: ${call(target, { GetSessionHistoryOutline: { session_id: roomB.session.id, agent_ids: [yAgent], latest_prompt_count: 4, include_prefetched_content: true } })}`,
        "Then reply with the words HISTORY and DONE joined by an underscore, a space, and REFUSED or ALLOWED for how the call in step 1 ended; nothing else.",
      ].join("\n"), "HISTORY_DONE", async () => {})
      const roomHistory = read.turn.calls.find(c => /history[._]search/.test(c.tool) && reachName(c) === null)
      note("history-room-tool", { called: !!roomHistory, status: roomHistory?.status, output: String(roomHistory?.output ?? roomHistory?.error ?? "").slice(0, 300) })
      const outline = read.reach.find(c => c.tool === "request" && /GetSessionHistoryOutline/.test(JSON.stringify(c.input)))
      check("X reads room 2's history on kernel B through reach with no prompt", !!outline && outline.status === "completed" && outline.output.includes("SessionHistoryOutline") && outline.output.includes(secret.trim().split(/\s+/).at(-1)), { out: outline?.output.slice(0, 200) })
      check("no card, approval or /sudo window appeared anywhere", (await cardsAnywhere()).length === 0, { cards: await cardsAnywhere() })
      check("the room history tool stays confined to X's own room (the agent reports the call refused)", /REFUSED/.test(read.turn.answer), { answer: read.turn.answer.slice(0, 200), logged: !!roomHistory })
    }
    clearInterval(keepAlive)
    if (flags.has("--hold")) await delay(Number(flags.get("--hold")) * 1000)
  }

  if (scenarios.has("view")) {
    note("scenario", { which: "view", provider })
    let launch
    await until(() => { try { launch = launchedServers(provider, a.child.pid); return true } catch { return false } }, `${provider} process`, 90000)
    const lists = {}
    for (const [name, server] of Object.entries(launch.servers)) lists[name] = await toolNames(server)
    const reachTools = ["kernels", "sessions", "request", "events_subscribe", "events_wait", "events_ack", "guide"]
    writeFileSync(join(evidence, "served-tools.json"), JSON.stringify(Object.fromEntries(Object.entries(lists).map(([name, names]) => [name, { count: names.length, sha256: sha(names), names }])), null, 1), { mode: 0o600 })
    check("the run is launched with the chariox and chariox-reach servers", Object.keys(launch.servers).sort().join() === "chariox,chariox-reach", { servers: Object.keys(launch.servers) })
    check("chariox-reach serves the seven tools", JSON.stringify([...(lists["chariox-reach"] ?? [])].sort()) === JSON.stringify([...reachTools].sort()), { names: lists["chariox-reach"] })
    check("no reach tool is in the chariox list", !(lists.chariox ?? []).some(name => reachTools.includes(name)), { count: lists.chariox?.length })
    if (flags.has("--expect-chariox-sha")) check("the chariox list is the one of the base kernel", sha(lists.chariox ?? []) === flags.get("--expect-chariox-sha"), { count: lists.chariox?.length, sha256: sha(lists.chariox ?? []) })
    // The owner's attachment is reaped when nobody pumps it, and the TUI photographs take a minute.
    const keepAttached = setInterval(() => a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {}), 2000).unref()
    const viewHome = ensureDirectory(join(a.dir, `view-${provider}`)), cwd = join(a.dir, "work")
    const common = { cwd, evidence, label: `${provider} (${model})` }
    if (provider === "claude") {
      writeFileSync(join(viewHome, ".claude.json"), JSON.stringify({ hasCompletedOnboarding: true, theme: "dark", projects: { [cwd]: { hasTrustDialogAccepted: true } } }), { mode: 0o600 })
      const shots = await photographTui({ ...common, name: "claude", program: ["claude", "--mcp-config", launch.file, "--strict-mcp-config", "--model", model],
        env: { HOME: viewHome, CLAUDE_CODE_OAUTH_TOKEN: readFileSync(tokenFile, "utf8").trim(), TERM: "xterm-256color", PATH: process.env.PATH },
        ready: /for shortcuts/, dialogs: [{ when: /Press Enter to continue/, keys: ["Enter"] }, { when: /Yes, I trust this folder/, keys: ["Down", "Enter"] }, { when: /Dark mode|text style/, keys: ["Enter"] }],
        steps: [{ keys: [{ literal: "/mcp" }, "Enter"], wait: 3000, shot: "claude-mcp-servers" },
          { keys: ["Down", "Enter"], wait: 2500, shot: "claude-mcp-reach-server" }, { keys: ["Enter"], wait: 2500, shot: "claude-mcp-reach-tools" }] })
      note("tui-captured", { shots: shots.map(x => x.shot) })
    } else if (provider === "codex") {
      const tokenVar = JSON.parse(launch.overrides.find(entry => entry.startsWith("mcp_servers.chariox-reach.bearer_token_env_var=")).split("=").slice(1).join("="))
      const shots = await photographTui({ ...common, name: "codex", program: ["codex", ...launch.overrides.flatMap(entry => ["-c", entry]), "-c", `projects."${cwd}".trust_level="trusted"`, "-c", "check_for_update_on_startup=false", "--model", model],
        env: { HOME: viewHome, CODEX_HOME: codexScope, [tokenVar]: launch.env[tokenVar], TERM: "xterm-256color", PATH: process.env.PATH },
        ready: /Ask Codex to do anything/, dialogs: [{ when: /Trust this folder\?/, keys: ["Enter"] }],
        steps: [{ keys: [{ literal: "/mcp" }], wait: 1500 }, { keys: ["Enter"], wait: 800 }, { keys: ["Enter"], wait: 5000, shot: "codex-mcp" }] })
      note("tui-captured", { shots: shots.map(x => x.shot) })
    } else {
      const shots = await photographTui({ ...common, name: "opencode", program: ["opencode"],
        env: { HOME: viewHome, OPENCODE_CONFIG_CONTENT: JSON.stringify({ mcp: launch.servers }), XDG_DATA_HOME: join(a.dir, "opencode-scope/data"), XDG_CONFIG_HOME: join(viewHome, "config"), XDG_CACHE_HOME: join(viewHome, "cache"), XDG_STATE_HOME: join(viewHome, "state"), TERM: "xterm-256color", PATH: process.env.PATH },
        steps: [{ wait: 8000 }, { keys: [{ literal: "/status" }, "Enter"], wait: 4000, shot: "opencode-status" }] })
      note("tui-captured", { shots: shots.map(x => x.shot) })
    }
    clearInterval(keepAttached)
  }

  if (scenarios.has("sudo")) {
    note("scenario", { which: "sudo" })
    const alias = (name) => call(target, { AliasSession: { session_id: roomB.session.id, alias: name } })
    await sendPrompt(["This is a drill. Use only the chariox-reach `request` tool; no shell, no files. Do not retry.", `Call \`request\` once with exactly: ${alias("renamed-without-sudo")}`, "Then reply with the words NOSUDO and DONE joined by an underscore, followed by the `code` field of the answer if there is one."].join("\n"), "NOSUDO_DONE")
    check("a /sudo-class request is refused on B without /sudo", (await bState()).alias !== "renamed-without-sudo" && claudeTranscript(a.dir + "/chariox", startedAt).includes("reach_requires_sudo"))
    await sudoPrompt(["This is a drill. Use only the chariox-reach `request` tool; no shell, no files. Do not retry.", `Call \`request\` once with exactly: ${alias("renamed-with-sudo")}`, "Then reply with the words WITHSUDO and DONE joined by an underscore."].join("\n"), "WITHSUDO_DONE")
    check("the same request works inside a /sudo window", (await bState()).alias === "renamed-with-sudo", { alias: (await bState()).alias })
  }

  // What the agent's reach calls answered since `since`, in the order its transcript shows them (the prompts ask for one call at a time).
  const reachAnswers = since => {
    const rows = toolLog(claudeTranscript(a.dir + "/chariox", since)), out = []
    rows.forEach((row, i) => { if (row.call && /chariox[-_]reach/.test(row.call)) out.push({ tool: row.call.replace(/^.*chariox[-_]reach[._-]*/, ""), input: row.input, answer: rows[i + 1]?.answer ?? "" }) })
    return out
  }
  const codesOf = (since, skip = 0) => reachAnswers(since).slice(skip).filter(x => x.tool === "request").map(x => /"code\\?":\\?"([a-z_]+)/.exec(x.answer.replace(/\\"/g, '"'))?.[1] ?? "ok")
  const drillPrompt = lines => ["This is a drill. Use only the chariox-reach `request` tool; no shell, no files. Make the calls one at a time, in order. Do not retry a refused call.", ...lines].join("\n")

  if (scenarios.has("humanonly")) {
    note("scenario", { which: "humanonly" })
    const pairedBefore = (await b.rpc({ ListPairedClients: {} })).PairedClientsListed.clients.length, aliasBefore = (await bState()).alias
    const requests = [{ ListCredentials: null }, { CloudRelayStatus: null }, { ConnectCloudRelay: null }, { CreateTerminalPairingLink: { terminal_type: "cli", expires_in_ms: 300000 } }, { RequestKernelSudo: { agent_id: roomB.agent.id, prompt: "give me a window" } }]
    const since = startedAt, skip = reachAnswers(since).length
    await sendPrompt(drillPrompt([...requests.map((r, i) => `${i + 1}. Call \`request\` with exactly: ${call(target, r)}`), "Then reply with the words HUMANONLY and DONE joined by an underscore."]), "HUMANONLY_DONE")
    const codes = codesOf(since, skip)
    check("the human-only requests are refused on B and a sudo request for itself is excluded", codes.join() === "human_only,human_only,human_only,human_only,excluded_for_agents", { codes })
    check("B is unchanged", (await b.rpc({ ListPairedClients: {} })).PairedClientsListed.clients.length === pairedBefore && (await bState()).alias === aliasBefore)
  }

  if (scenarios.has("lineage")) {
    note("scenario", { which: "lineage" })
    const refused = ["reach_requires_sudo", "excluded_for_agents", "human_only"]
    const attempts = [
      call(target, { AliasAgent: { session_id: roomB.session.id, agent_id: roomB.agent.id, alias: "renamed-by-reach" } }),
      call(target, { ArchiveAgent: { session_id: roomB.session.id, agent_id: roomB.agent.id, archived: true } }),
      call(target, { DestroyAgent: { session_id: roomB.session.id, agent_id: roomB.agent.id } }),
      call(target, { DeleteSession: { session_ref: roomB.session.id, workspace_id: null } }),
      call(null, { DestroyAgent: { session_id: siblingA.session.id, agent_id: siblingA.agent.id } }),
      call(null, { DestroyAgent: { session_id: roomA.session.id, agent_id: agentA } }),
    ]
    const roomsOf = async r => (await r.rpc({ ListSessions: null })).SessionsListed.sessions.map(x => x.id).sort().join()
    const unchanged = async () => (await bState()).agents.some(x => x.id === roomB.agent.id && !x.archived) && (await bState()).alias !== "renamed-by-reach" && (await roomsOf(b)).includes(roomB.session.id) && (await roomsOf(a)).includes(siblingA.session.id) && (await roomsOf(a)).includes(roomA.session.id)
    const since = startedAt, skip = reachAnswers(since).length
    // One call per prompt: a model that sees six destructive calls in one prompt tends to stop after the first.
    for (const [i, r] of attempts.entries()) await sendPrompt(drillPrompt([`Call \`request\` once with exactly: ${r}`, "This room and agent belong to a disposable drill kernel, and the kernel itself refuses what it must refuse, so make the call.", `Then reply with the words LINEAGE${i} and DONE joined by an underscore.`]), `LINEAGE${i}_DONE`)
    const codes = codesOf(since, skip)
    check("renaming, archiving and deleting other agents and rooms, and destroying itself, is refused without /sudo", codes.length === attempts.length && codes.every(c => refused.includes(c)), { codes })
    check("nothing changed", await unchanged())
    // Inside a /sudo window the owner's authority is the agent's, as at home: a rename works, deleting a room stays excluded.
    const sudoSkip = reachAnswers(since).length
    await sudoPrompt(drillPrompt([`1. Call \`request\` with exactly: ${attempts[3]}`, `2. Call \`request\` with exactly: ${call(target, { AliasAgent: { session_id: roomB.session.id, agent_id: roomB.agent.id, alias: "renamed-in-sudo" } })}`, "Then reply with the words LINEAGESUDO and DONE joined by an underscore."]), "LINEAGESUDO_DONE")
    const sudoCodes = codesOf(since, sudoSkip)
    check("inside a /sudo window deleting a room stays excluded and a rename works", sudoCodes.join() === "excluded_for_agents,ok" && (await bState()).agents.some(x => x.id === roomB.agent.id && x.alias === "renamed-in-sudo"), { sudoCodes })
    check("no room is gone", (await roomsOf(b)).includes(roomB.session.id) && (await roomsOf(a)).includes(siblingA.session.id) && (await roomsOf(a)).includes(roomA.session.id))
  }

  if (scenarios.has("newrooms")) {
    note("scenario", { which: "newrooms" })
    // Creating a Room is room-level: no /sudo, on the agent's own kernel and on another one.
    const create = (kernelArg, r, alias) => call(kernelArg, { CreateSession: { workspace_id: join(r.dir, "work"), worktree_id: join(r.dir, "work"), alias, slice_ref: null } })
    const aliasesOf = async r => (await r.rpc({ ListSessions: null })).SessionsListed.sessions.map(x => x.alias)
    const since = startedAt, skip = reachAnswers(since).length
    await sendPrompt(drillPrompt([`1. Call \`request\` with exactly: ${create(null, a, "made-at-home")}`, `2. Call \`request\` with exactly: ${create(target, b, "made-on-the-other-kernel")}`, "Then reply with the words NEWROOMS and DONE joined by an underscore."]), "NEWROOMS_DONE")
    const codes = codesOf(since, skip)
    check("the agent created a Room on its own kernel and on another one without /sudo", codes.join() === "ok,ok" && (await aliasesOf(a)).includes("made-at-home") && (await aliasesOf(b)).includes("made-on-the-other-kernel"), { codes })
  }

  // The text of every base64 chunk an agent's answers carried since `since` (room terminal output), decoded.
  const decodedOutput = since => reachAnswers(since).flatMap(x => [...x.answer.replace(/\\"/g, '"').matchAll(/"data_base64":"([A-Za-z0-9+/=]+)"/g)].map(m => Buffer.from(m[1], "base64").toString("utf8"))).join("")

  if (scenarios.has("workday")) {
    note("scenario", { which: "workday" })
    const nonce = randomBytes(3).toString("hex"), file = join(b.dir, "work", `reach-note-${nonce}.txt`)
    const terminalRequest = operation => call(target, { RoomTerminal: { session_id: roomB.session.id, operation } })
    const since = Date.now() - 1000
    await sendPrompt(drillPrompt([
      "A command in room " + roomB.session.id + " on the other kernel:",
      `1. ${terminalRequest({ operation: "exec_open", command: ["sh", "-c", `echo workday-${nonce}`] })} gives exec_opened.exec_id.`,
      `2. Read it: ${terminalRequest({ operation: "exec_read", exec_id: "EXEC_ID", after_cursor: 0, wait_ms: 10000 })} until the answer has an outcome.`,
      `3. List the execs (${terminalRequest({ operation: "exec_list" })}) and close yours (${terminalRequest({ operation: "exec_close", exec_id: "EXEC_ID" })}).`,
      "A terminal: ",
      `4. ${terminalRequest({ operation: "open", cols: 80, rows: 24, title: "reach", command: ["sh"] })} gives the terminal id.`,
      `5. Type into it: ${terminalRequest({ operation: "input", terminal_id: "TERMINAL_ID", data_base64: Buffer.from(`echo pty-${nonce}\n`).toString("base64") })}, then read: ${terminalRequest({ operation: "read_output", terminal_id: "TERMINAL_ID", after_offset: 0, wait_ms: 3000 })}.`,
      `6. List the terminals (${terminalRequest({ operation: "list" })}) and close yours (${terminalRequest({ operation: "close", terminal_id: "TERMINAL_ID" })}).`,
      "Files, by commands in the room (your attachments cannot read or write files themselves):",
      `7. ${terminalRequest({ operation: "exec_open", command: ["sh", "-c", `printf 'workday ${nonce}\\n' > reach-note-${nonce}.txt && cat reach-note-${nonce}.txt && ls`] })}, then exec_read it as in step 2 and exec_close it.`,
      "Then reply with the words WORKDAY and DONE joined by an underscore and the code of every refused answer.",
    ]), "WORKDAY_DONE")
    const out = decodedOutput(since), answers = reachAnswers(since)
    check("the agent ran a command on the other kernel's room and read its output", out.includes(`workday-${nonce}`), { tail: out.slice(-120) })
    check("the agent typed into a terminal of that room and read the echo", out.includes(`pty-${nonce}`))
    const written = (() => { try { return readFileSync(file, "utf8") } catch { return null } })()
    check("the agent wrote a file in that room and read it back", written === `workday ${nonce}\n` && out.includes(`workday ${nonce}`), { written })
    note("workday-codes", { codes: codesOf(since) })
  }

  if (scenarios.has("recall")) {
    note("scenario", { which: "recall" })
    const word = `RECALLWORD${randomBytes(3).toString("hex")}`, bAttach = (await b.rpc({ AttachToSession: { session_id: roomB.session.id, client_id: "reach-recall", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    const bSince = Date.now() - 1000
    await b.rpc(submitPromptRequest(roomB.session.id, bAttach, roomB.agent.id, `This is a drill. Reply with the single word ${word} and nothing else.`, []))
    await until(async () => { await b.rpc({ PumpTerminalOutput: { session_id: roomB.session.id, attachment_id: bAttach } }).catch(() => {}); return claudeTranscript(b.dir + "/chariox", bSince).includes(word) }, "B's agent answers", 180000)
    await delay(3000)
    const nulls = { agent_id: null, provider: null, model: null, workflow_id: null, machine_id: null, repo_root: null, worktree_path: null, kind: null }
    const since = startedAt, skip = reachAnswers(since).length
    await sendPrompt(drillPrompt([
      `1. ${call(target, { SearchRecall: { query: word, session_id: roomB.session.id, ...nulls, after_sequence: null, limit: 5 } })}`,
      `2. ${call(target, { QueryRecall: { session_id: roomB.session.id, ...nulls, text: null, after_sequence: null, before_sequence: null, limit: 3 } })}`,
      `3. ${call(target, { SearchRecall: { query: word, session_id: "no-such-room", ...nulls, after_sequence: null, limit: 5 } })}`,
      "Then reply with the words RECALL and DONE joined by an underscore, and say for each of the three answers whether it contained events.",
    ]), "RECALL_DONE")
    // A call the provider rejected before the kernel saw it (malformed JSON) has no request; only answered calls count.
    const answers = reachAnswers(since).slice(skip).filter(x => x.tool === "request" && x.input?.request)
    const answerTo = (name, unknownRoom = false) => answers.findLast(x => x.input.request[name] && (x.input.request[name].session_id === "no-such-room") === unknownRoom)?.answer ?? ""
    check("the agent found another room's real work by a term of it", answerTo("SearchRecall").includes(word), { answer: answerTo("SearchRecall").slice(0, 160) })
    check("the agent paged another room's history", /RecallEvents":\{"events":\[\{/.test(answerTo("QueryRecall").replace(/\\"/g, '"')), { answer: answerTo("QueryRecall").slice(0, 160) })
    note("recall-unknown-room", { answer: answerTo("SearchRecall", true).slice(0, 300) })
    check("an unknown room answers session_not_found without the word", /session_not_found/.test(answerTo("SearchRecall", true)) && !answerTo("SearchRecall", true).includes(word))
  }

  if (scenarios.has("inherit")) {
    note("scenario", { which: "inherit" })
    // X holds a /sudo window (its task is a long command). While it runs, the person asks the agent of the sibling room for the same sudo-class request:
    // the window is X's task's, not every agent's of the kernel.
    const siblingAttach = (await a.rpc({ AttachToSession: { session_id: siblingA.session.id, client_id: "reach-inherit", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
    await a.rpc({ LaunchProviderRun: { session_id: siblingA.session.id, agent_id: siblingA.agent.id, adapter_key: provider, provider, account_profile: accountProfile, model, variant: providers[provider].variant, native_tui: false } }).catch(() => {})
    const since = Date.now() - 1000
    const submitted = a.client.send(submitPromptRequest(roomA.session.id, attached, agentA, "/sudo This is a drill. Run the shell command `sleep 75`, then reply with the words INHERITX and DONE joined by an underscore.", []))
    submitted.catch(error => note("sudo-submit-error", { message: String(error.message).slice(0, 300) }))
    let popup
    await until(async () => { await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {}); return (popup = (await state(a, roomA.session.id)).active_interactions?.find(x => String(x.id).startsWith("sudo:"))) }, "sudo popup", 60000)
    await a.rpc({ RespondToInteraction: { session_id: roomA.session.id, interaction_id: popup.id, choice_id: "approve", custom_reply: "60", passkey: a.passkey } })
    await delay(8000)
    const rename = call(target, { AliasSession: { session_id: roomB.session.id, alias: "renamed-by-the-sibling" } })
    await a.rpc(submitPromptRequest(siblingA.session.id, siblingAttach, siblingA.agent.id, drillPrompt([`Call \`request\` once with exactly: ${rename}`, "Then reply with the words INHERITED and DONE joined by an underscore, followed by the code field of the answer."]), []))
    await until(async () => { await a.rpc({ PumpTerminalOutput: { session_id: siblingA.session.id, attachment_id: siblingAttach } }).catch(() => {}); await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {}); return claudeTranscript(a.dir + "/chariox", since).includes("INHERITED_DONE") }, "the sibling's answer", 240000).catch(error => note("turn-timeout", { message: error.message }))
    const answers = reachAnswers(since).filter(x => x.tool === "request" && x.input?.request?.AliasSession?.alias === "renamed-by-the-sibling")
    check("the sibling agent, asked inside X's /sudo window, is refused", answers.length === 1 && /reach_requires_sudo/.test(answers[0].answer) && (await bState()).alias !== "renamed-by-the-sibling", { answers: answers.map(x => x.answer.slice(0, 120)) })
    await submitted.catch(() => {})
    await finished("INHERITX_DONE", since)
  }

  if (scenarios.has("acquisition")) {
    note("scenario", { which: "acquisition" })
    const acquire = call(null, { ControlPlane: { action: "acquire", capability_revision: "r1", request_id: "reach-drill" } })
    const acquirePrompt = tag => ["This is a drill. Use only the chariox-reach `request` tool; no shell, no files. Do not retry.", `Call \`request\` once with exactly: ${acquire}`, `Then reply with the words ${tag} and DONE joined by an underscore, followed by the \`code\` field of the answer if there is one.`].join("\n")
    const askedBefore = async () => (await state(a, roomA.session.id)).active_interactions?.filter(x => String(x.message ?? "").includes("acquire a machine")) ?? []
    for (const choice of ["decline", "accept"]) {
      const tag = choice === "decline" ? "ACQUIRENO" : "ACQUIREYES", since = Date.now() - 1000
      const needle = choice === "decline" ? "acquisition_declined" : "no connected machine service"
      const seenBefore = claudeTranscript(a.dir + "/chariox", 0).split(needle).length
      await a.rpc(submitPromptRequest(roomA.session.id, attached, agentA, acquirePrompt(tag), []))
      let asked
      await until(async () => {
        await a.rpc({ PumpTerminalOutput: { session_id: roomA.session.id, attachment_id: attached } }).catch(() => {})
        asked = (await askedBefore())[0]
        return asked
      }, "acquisition question", 180000)
      note("acquisition-question", { message: asked.message, choices: asked.choices.map(c => c.id) })
      await a.rpc({ RespondToInteraction: { session_id: roomA.session.id, interaction_id: asked.id, choice_id: choice } })
      await finished(`${tag}_DONE`, since)
      const honoured = await until(() => claudeTranscript(a.dir + "/chariox", 0).split(needle).length > seenBefore, needle, 20000).then(() => true, () => false)
      check(`${choice} on the acquisition question is honoured`, honoured)
    }
    await a.rpc({ SetUserConfigValue: { path: "agents.ask_before_acquiring_machines", value: "false" } })
    const offSince = Date.now() - 1000
    await sendPrompt(acquirePrompt("ACQUIREOFF"), "ACQUIREOFF_DONE")
    check("with the toggle off there is no acquisition question and the request goes on", (await askedBefore()).length === 0 && claudeTranscript(a.dir + "/chariox", offSince).includes("no connected machine service"))
  }
}
try {
  await main().finally(() => saveToolLog())
  note(failures.length ? `${label}: RED ${failures.join("; ")}` : `${label}: PASS`)
  if (failures.length) process.exitCode = 1
} catch (error) {
  note("FAIL", { message: String(error.message).replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>") }); process.exitCode = 1
} finally {
  for (const client of clients) await client.close().catch(() => {})
  const stopped = await Promise.allSettled(children.toReversed().map(stop))
  const failed = stopped.some(result => result.status === "rejected")
  if (!failed && (!failures.length || flags.get("--discard") === "yes")) rmSync(root, { recursive: true, force: true })
  else process.exitCode = 1
  note("cleanup", { stateRemoved: !failed, liveChildren: children.filter(c => c.exitCode === null && c.signalCode === null).map(c => c.pid) })
}

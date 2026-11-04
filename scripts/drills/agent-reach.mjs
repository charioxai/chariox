// MP-08 / MP-09 / MP-10 / MP-11 agent reach: a real Claude Room agent on kernel A, without /sudo, reaches
// kernel B through its `chariox-reach` tools; B's audit and paired-clients list name the agent; what needs /sudo
// in the agent's own room is refused on B without /sudo and works inside a /sudo window; the "ask before agents acquire machines" setting turns an acquisition into one decision in the agent's room.
// Real kernels, local scoped relay, real Claude. Credentials are read inside this script, never printed or stored
// in evidence. Red on a base kernel without `chariox-reach`: the agent has no tool, so the B-side checks fail.
// The `tools` scenario runs with a real Claude, Codex or OpenCode agent (`--provider`): the agent calls every
// `chariox-reach` tool and a room tool given another kernel, and B's audit names it.
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
const scenarios = new Set((flags.get("--scenarios") ?? (provider === "claude" ? "reach,sudo,acquisition,tools,view" : "tools,view")).split(","))
assert(provider === "claude" || [...scenarios].every(name => name === "tools" || name === "view"), "only the tools and view scenarios run with Codex or OpenCode")
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
  // Only the Claude scenarios touch kernel B's agent; for the tools scenario B is a passive target.
  if (provider === "claude") await provision(b)
  const agentDefaults = { provider, account_profile: accountProfile, model, effort: providers[provider].variant, permission_level: "yolo" }
  // Kernel B: the room another room's agent will message, and the room the agent must not rename without /sudo.
  const roomB = (await b.rpc({ CreateSession: { workspace_id: join(b.dir, "work"), worktree_id: join(b.dir, "work"), alias: "target-room", agent_defaults: provider === "claude" ? agentDefaults : undefined } })).SessionCreated
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
    // The agent's latest turn from the kernel's own history, the same for every provider: its tool calls with their answers, and its last words.
    const latestTurn = async () => {
      const outline = (await a.rpc({ GetSessionHistoryOutline: { session_id: roomA.session.id, agent_ids: [agentA], latest_prompt_count: 1, include_prefetched_content: true } })).SessionHistoryOutline
      const history = outline.agents.find(x => x.agent_id === agentA)
      const turn = history?.turns?.at(-1)
      const calls = new Map()
      for (const blob of history?.prefetched_blobs ?? []) for (const { entry } of blob.entries) {
        if (entry.kind !== "provider_tool") continue
        let call
        try { call = JSON.parse(entry.text) } catch { continue }
        calls.set(entry.merge_key, call)
      }
      if (process.env.REACH_DRILL_RAW) writeFileSync(join(evidence, "outline-raw.json"), JSON.stringify(outline, null, 1))
      return { id: turn?.turn_id, done: turn?.lifecycle === "completed", answer: turn?.summary?.entry?.text ?? "", calls: [...calls.values()] }
    }
    // Claude and OpenCode put the server in the tool name; Codex names the server in the call's title.
    const reachName = call => /chariox[-_]reach/.test(call.tool) ? call.tool.replace(/^.*chariox[-_]reach[._-]*/, "") : /chariox[-_]reach/.test(call.title ?? "") ? call.tool : null
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
    // Following events from a Room agent arrives with slice S3: today each call is a tool result with a typed refusal.
    const events = ["events_subscribe", "events_wait", "events_ack"].map(name => second.reach.find(call => call.tool === name))
    note("events-tools", { calls: events.map(call => call && { tool: call.tool, status: call.status, output: call.output.slice(0, 200) }) })
    check("the agent calls the three events tools and each answers with a typed code", events.every(call => call && /code\W+[a-z_]{4,}/.test(call.output)))
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
  }

  if (scenarios.has("sudo")) {
    note("scenario", { which: "sudo" })
    const alias = (name) => call(target, { AliasSession: { session_id: roomB.session.id, alias: name } })
    await sendPrompt(["This is a drill. Use only the chariox-reach `request` tool; no shell, no files. Do not retry.", `Call \`request\` once with exactly: ${alias("renamed-without-sudo")}`, "Then reply with the words NOSUDO and DONE joined by an underscore, followed by the `code` field of the answer if there is one."].join("\n"), "NOSUDO_DONE")
    check("a /sudo-class request is refused on B without /sudo", (await bState()).alias !== "renamed-without-sudo" && claudeTranscript(a.dir + "/chariox", startedAt).includes("reach_requires_sudo"))
    await sudoPrompt(["This is a drill. Use only the chariox-reach `request` tool; no shell, no files. Do not retry.", `Call \`request\` once with exactly: ${alias("renamed-with-sudo")}`, "Then reply with the words WITHSUDO and DONE joined by an underscore."].join("\n"), "WITHSUDO_DONE")
    check("the same request works inside a /sudo window", (await bState()).alias === "renamed-with-sudo", { alias: (await bState()).alias })
  }

  if (scenarios.has("acquisition")) {
    note("scenario", { which: "acquisition" })
    const acquire = call(null, { ControlPlane: { action: "acquire", capability_revision: "r1", request_id: "reach-drill" } })
    const acquirePrompt = tag => ["This is a drill. Use only the chariox-reach `request` tool; no shell, no files. Do not retry.", `Call \`request\` once with exactly: ${acquire}`, `Then reply with the words ${tag} and DONE joined by an underscore, followed by the \`code\` field of the answer if there is one.`].join("\n")
    const askedBefore = async () => (await state(a, roomA.session.id)).active_interactions?.filter(x => String(x.message ?? "").includes("acquire a machine")) ?? []
    await sendPrompt(acquirePrompt("ACQUIREOFF"), "ACQUIREOFF_DONE")
    check("the toggle is off by default: no acquisition question and the request goes on", (await askedBefore()).length === 0 && claudeTranscript(a.dir + "/chariox", startedAt).includes("no connected machine service"))
    await a.rpc({ SetUserConfigValue: { path: "agents.ask_before_acquiring_machines", value: "true" } })
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

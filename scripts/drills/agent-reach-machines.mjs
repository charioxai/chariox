// MP-08 / MP-09 / MP-10 / MP-11 agent reach, machines: a real Claude Room agent on an enrolled home kernel acquires the
// smallest managed machine through its `chariox-reach` `request` tool, the owner accepts the question the default
// setting raises, the machine enrolls and shows up in `kernels`, the agent runs one command on it (the step that creates a
// Room there is room-level, no /sudo), then stops and releases it. `--provider claude|codex|opencode` picks the agent. The hosted stack is a disposable validation stack
// whose Cloud manager rents the real machine; this script never sees a provider key. Credentials are read inside the
// script and never printed. After each phase it can run a photo command (the hosted web's Machines view).
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { randomBytes } from "node:crypto"
import { mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, statSync, writeFileSync, rmSync } from "node:fs"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { execFileSync } from "node:child_process"
import { createServer } from "node:net"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const required = key => { const value = flags.get(key); assert.equal(typeof value, "string", `${key} is required`); return value }
const binary = resolve(required("--kernel-binary")), stack = required("--stack")
const stateRoot = resolve(required("--state-root")), evidence = resolve(required("--evidence-dir"))
const tokenFile = flags.get("--claude-token-file") ?? "/root/.claude-lanes/oauth-token.chariox"
const provider = flags.get("--provider") ?? "claude"
const providers = {
  claude: { model: "claude-haiku-5-5", variant: "low" },
  codex: { model: "gpt-6-luna", variant: "low" },
  opencode: { model: "opencode-go/deepseek-v4.1-flash", variant: null },
}
assert(providers[provider], `--provider is one of ${Object.keys(providers)}`)
const codexScope = flags.get("--codex-scope") ?? "/root/.chariox/dev/worker-k/home/state/provider-accounts/local/codex/kaissandra-ai-reviewer-login-esqv7b7c/codex"
const opencodeAuth = flags.get("--opencode-auth") ?? "/root/.chariox/dev/provider-runtimes/fleet-accounts/opencode-home/.local/share/opencode/auth.json"
const photoCommand = flags.get("--photo-command") // run as `<command> <phase>`; failures are checks, not crashes
const label = flags.get("--label") ?? "run"
// The service answers a repeated request id with the machine of the first one, so every run uses its own.
const runId = Date.now().toString(36)
assert(stateRoot.startsWith("/root/.chariox/dev/"), "state must be lane-owned")
assert(!evidence.startsWith(fileURLToPath(new URL("../../", import.meta.url))), "evidence must be outside the repository")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const stackRoot = `/w/valenv/prs/${stack}`
const meta = JSON.parse(readFileSync(`${stackRoot}/metadata.json`, "utf8"))
const origin = `https://${meta.hostname}`
const root = mkdtempSync(join(stateRoot, "machines-"))
const children = [], steps = [], failures = []
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 2) + "\n", { mode: 0o600 })
  console.log(`MP-08/09/10/11 ${name}`)
}
const check = (name, ok, detail = {}) => { note(ok ? "check-pass" : "check-FAIL", { check: name, ...detail }); if (!ok) failures.push(name) }
async function until(fn, description, timeout = 30000, step = 250) {
  const end = Date.now() + timeout
  while (Date.now() < end) { try { const value = await fn(); if (value) return value } catch {} await delay(step) }
  throw new Error(`timeout: ${description}`)
}
async function freePort() {
  const server = createServer()
  await new Promise(r => server.listen(0, "127.0.0.1", r))
  const port = server.address().port
  await new Promise(r => server.close(r))
  return port
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

// The home kernel: one real kernel, enrolled with the stack's Cloud through the fixture account's device flow, with a vault for Claude.
async function homeKernel() {
  const dir = join(root, "home"), ports = []
  for (let i = 0; i < 4; i++) ports.push(await freePort())
  for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
  mkdirSync("/var/tmp/rch", { recursive: true })
  const env = { PATH: process.env.PATH, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash", HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"),
    XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join("/var/tmp/rch", `machines-${process.pid}.sock`), CHARIOX_DAEMON_ALIAS: "reach-home", CHARIOX_MACHINE_ALIAS: "reach-home",
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
  const log = openSync(join(evidence, "home.log"), "a", 0o600)
  const child = spawn(binary, [], { env, stdio: ["ignore", log, log] })
  assert(Number.isSafeInteger(child.pid) && child.pid > 1)
  children.push(child)
  note("process-start", { label: "home", pid: child.pid })
  const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  const r = { dir, env, client, child, passkey: randomBytes(24).toString("hex") }
  r.rpc = async request => {
    const result = await client.send(request)
    if (result.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${result.Error.code} ${result.Error.message ?? ""}`)
    return result
  }
  await until(async () => !!(await r.rpc({ RelayStatus: null })).RelayStatus, "home ready", 90000)
  const login = (await r.rpc({ StartCloudRelayLogin: { api_url: origin, client_alias: "Reach machines drill" } })).CloudRelayLoginStarted.login
  const accessKey = readFileSync(`${stackRoot}/config/browser-access-key`, "utf8").trim()
  const approval = await fetch(`${origin}/auth/dev/device/approve`, { method: "POST", headers: { "Content-Type": "application/json", "x-chariox-dev-auth-secret": accessKey },
    body: JSON.stringify({ userCode: login.user_code, email: meta.fixture_email, displayName: "Reach validation", accountSlug: stack, accountName: "Reach validation" }), signal: AbortSignal.timeout(30000) })
  assert(approval.ok, `fixture device approval failed: ${approval.status}`)
  await delay(Math.max(5200, (login.interval_seconds ?? 5) * 1000 + 200))
  const enrolled = (await r.rpc({ PollCloudRelayLogin: { api_url: origin, device_code: login.device_code, supports_access_denied: true } })).CloudRelayLoginPolled.result
  assert.equal(enrolled.status, "approved")
  await r.rpc({ ConnectCloudRelay: null })
  r.status = await until(async () => { const s = (await r.rpc({ RelayStatus: null })).RelayStatus.status; return s.connected && s }, "home relay connected", 90000, 1000)
  note("home-enrolled", { kernel: r.status.daemon_id, relay: r.status.relay_url, realm: enrolled.profile.realm_id })
  return r
}

const stripped = text => text.replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")

let saveToolLog = () => {}
async function main() {
  const home = await homeKernel()
  const state = async session => (await home.rpc({ GetSessionState: { session_id: session } })).SessionState.session
  // The provider login the agent runs on: Claude through a disposable vault, Codex and OpenCode through a linked product scope.
  let accountProfile = "default"
  if (provider === "claude") {
    const made = (await home.rpc({ CreateSession: { workspace_id: join(home.dir, "work"), worktree_id: join(home.dir, "work"), alias: "vault-room" } })).SessionCreated
    const att = (await home.rpc({ AttachToSession: { session_id: made.session.id, client_id: "reach-setup", capability_level: "FullTerminal" } })).SessionAttached
    await home.client.subscribeToKernelEvents(made.session.id, att.attachment.id)
    const pending = home.rpc({ ManageCredentialVault: { session_id: made.session.id, agent_id: made.agent.id } })
    pending.catch(() => {})
    let prompt
    await until(async () => (prompt = (await state(made.session.id)).active_interactions?.find(x => x.id.startsWith("vault-unlock-"))), "disposable vault prompt")
    await home.rpc({ RespondToInteraction: { session_id: made.session.id, interaction_id: prompt.id, choice_id: "passphrase", custom_reply: home.passkey } })
    await pending
    await home.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: readFileSync(tokenFile, "utf8").trim(), overwrite: true } })
  } else if (provider === "codex") {
    accountProfile = (await home.rpc({ LinkProviderAccountProfile: { provider, label: "reach drill", path: codexScope } })).ProviderAccountProfile.profile.profile_id
    await home.rpc({ GetProviderAuthStatus: { provider, account_profile: accountProfile } })
  } else {
    const scope = join(home.dir, "opencode-scope"), data = join(scope, "data/opencode")
    mkdirSync(data, { recursive: true, mode: 0o700 })
    writeFileSync(join(data, "auth.json"), readFileSync(opencodeAuth), { mode: 0o600 })
    accountProfile = (await home.rpc({ LinkProviderAccountProfile: { provider, label: "reach drill", path: scope } })).ProviderAccountProfile.profile.profile_id
  }
  const { model, variant } = providers[provider]
  const defaults = { provider, account_profile: accountProfile, model, effort: variant, permission_level: "yolo" }
  const room = (await home.rpc({ CreateSession: { workspace_id: join(home.dir, "work"), worktree_id: join(home.dir, "work"), alias: "home-room", agent_defaults: defaults } })).SessionCreated
  const attached = (await home.rpc({ AttachToSession: { session_id: room.session.id, client_id: "reach-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  const agent = room.agent.id
  await home.rpc({ LaunchProviderRun: { session_id: room.session.id, agent_id: agent, adapter_key: provider, provider, account_profile: accountProfile, model, variant, native_tui: false } })
  // The agent's latest turn from the kernel's own history, the same for every provider: its tool calls in order with their answers, and its last words.
  const reachName = call => /chariox[-_]reach/.test(call.tool) ? call.tool.replace(/^.*chariox[-_]reach[._-]*/, "") : /chariox[-_]reach/.test(call.title ?? "") ? call.tool : call.tool
  const turnOf = async () => {
    const outline = (await home.rpc({ GetSessionHistoryOutline: { session_id: room.session.id, agent_ids: [agent], latest_prompt_count: 1, include_prefetched_content: true } })).SessionHistoryOutline
    const history = outline.agents.find(x => x.agent_id === agent)
    const last = history?.turns?.at(-1)
    const calls = new Map()
    for (const blob of history?.prefetched_blobs ?? []) for (const { entry } of blob.entries) {
      if (entry.kind !== "provider_tool") continue
      try { calls.set(entry.merge_key, JSON.parse(entry.text)) } catch {}
    }
    const rows = []
    for (const call of calls.values()) {
      rows.push({ call: reachName(call), input: call.input })
      rows.push({ answer: String(call.output ?? call.error ?? "").slice(0, 1500) })
    }
    rows.push({ said: (last?.summary?.entry?.text ?? "").slice(0, 600) })
    return { id: last?.turn_id, done: last?.lifecycle === "completed", rows }
  }
  const allRows = []
  const startedAt = Date.now()
  saveToolLog = () => writeFileSync(join(evidence, "agent-tool-log.json"), stripped(JSON.stringify(allRows, null, 1)), { mode: 0o600 })
  note("agent-ready", { room: room.session.id, agent, provider, model })

  const pump = () => home.rpc({ PumpTerminalOutput: { session_id: room.session.id, attachment_id: attached } }).catch(() => {})
  async function idle(timeout = 300000) {
    await until(async () => {
      await pump()
      const s = await state(room.session.id)
      const a = (s.agents ?? []).find(x => x.id === agent)
      const ps = s.prompt_states?.[agent] ?? {}
      return !(ps.active_prompt ?? null) && !(ps.queued_prompts ?? []).length && a && !a.is_processing && a.state !== "Working"
    }, "agent idle", timeout, 1000)
  }
  // The person's side while a prompt runs: answer the acquisition question, as the owner of the Room, once it appears.
  const cards = []
  function ownerAnswers(choice) {
    const seen = new Set()
    return setInterval(async () => {
      try {
        await pump()
        for (const x of (await state(room.session.id)).active_interactions ?? []) {
          if (seen.has(x.id) || !String(x.message ?? "").includes("acquire a machine")) continue
          seen.add(x.id)
          cards.push({ id: x.id, message: x.message, choices: x.choices.map(c => c.id), at: Date.now() })
          note("acquisition-question", { message: x.message, choices: x.choices.map(c => c.id) })
          await home.rpc({ RespondToInteraction: { session_id: room.session.id, interaction_id: x.id, choice_id: choice } })
        }
      } catch {}
    }, 1000)
  }
  async function turn(text, marker, timeout = 900000) {
    const before = (await turnOf()).id
    const submitted = home.rpc(submitPromptRequest(room.session.id, attached, agent, text, []))
    submitted.catch(error => note("submit-error", { message: String(error.message).slice(0, 300) }))
    await until(async () => { await pump(); const t = await turnOf(); return t.id !== before && t.done }, `agent turn (${marker})`, timeout, 2000).catch(error => note("turn-timeout", { message: error.message }))
    await submitted.catch(() => {})
    await idle()
    const rows = (await turnOf()).rows
    allRows.push(...rows)
    note("turn-done", { marker, markerSeen: JSON.stringify(rows.at(-1)).includes(marker) })
    return rows
  }
  const photograph = phase => {
    if (!photoCommand) return
    try {
      execFileSync("bash", ["-c", `${photoCommand} ${phase}`], { stdio: ["ignore", "inherit", "inherit"], timeout: 600000 })
      note("photos", { phase })
    } catch (error) { check(`the Machines view photographs at phase ${phase}`, false, { message: String(error.message).slice(0, 200) }) }
  }
  const request = body => JSON.stringify({ request: body })

  if (flags.get("--rehearse") === "yes") {
    // No machine: the question and the tools are exercised with a revision the service refuses before it creates anything.
    // `--ask off` turns the setting off first: the same request then raises no question.
    const askOff = flags.get("--ask") === "off"
    if (askOff) await home.rpc({ SetUserConfigValue: { path: "agents.ask_before_acquiring_machines", value: "false" } })
    const gate = ownerAnswers("accept")
    let rows
    try {
      rows = await turn([
        "This is a drill. Use only the chariox-reach MCP tools. Do not retry a failed call.",
        `1. Call \`request\` with exactly: ${request({ ControlPlane: { action: "capabilities" } })}.`,
        `2. Call \`request\` with exactly: ${request({ ControlPlane: { action: "acquire", capability_revision: "stale-revision", request_id: `reach-rehearsal-${runId}` } })}. The revision is deliberately stale, so the service refuses it before it creates anything; that refusal is what this drill expects. ${askOff ? "" : "The owner is asked first."}`,
        "3. Call `kernels` once.",
        "Then reply with the words REHEARSED and DONE joined by an underscore and the code of the step 2 answer.",
      ].join("\n"), "REHEARSED_DONE")
    } finally { clearInterval(gate) }
    saveToolLog()
    const answered = JSON.stringify(rows)
    if (askOff) check("with the setting off the acquisition raised no question", cards.length === 0, { cards: cards.length })
    else check("the owner was asked before the acquisition", cards.length === 1 && /^Let home-room\/\S+ \(Chariox agent\) acquire a machine\?$/.test(cards[0].message), { card: cards[0] })
    check("the service refused the stale revision and created nothing", /authorization_denied|Refresh the connected service/.test(answered))
    return
  }

  // 1. Acquire: the default setting asks first; the owner accepts; the agent waits for the machine.
  const gate = ownerAnswers("accept")
  let text1
  try {
    text1 = await turn([
      "This is a drill for the owner's connected machine service. Use only the chariox-reach MCP tools, and your shell for `sleep` only. Do not retry a failed call unless told to.",
      `1. Call \`request\` with exactly: ${request({ ControlPlane: { action: "capabilities" } })}. Remember machine_allocation.revision.`,
      `2. Call \`request\` with: ${request({ ControlPlane: { action: "acquire", capability_revision: "REVISION", request_id: `reach-drill-acquire-${runId}` } })} (REVISION from step 1). The owner is asked first; the call waits for the answer.`,
      "3. Remember allocation_ref from the answer. Then, up to 60 times: run the shell command `sleep 10`, then call `request` with " + request({ ControlPlane: { action: "get", allocation_ref: "REF" } }) + " until state is ready and access_state is available. If the answer says access_state is pending although the state is ready, keep asking.",
      "4. Call `kernels` once.",
      "5. Take kernel_target.kernel_id from the last get answer and call `request` with {\"kernel\":\"KERNEL_ID\",\"request\":{\"ListSessions\":null}}.",
      "Start your reply with the exact token ACQUIRED_DONE, then the allocation_ref and the kernel id.",
    ].join("\n"), "ACQUIRED_DONE")
  } finally { clearInterval(gate) }
  saveToolLog()
  const log1 = text1
  const answered = JSON.stringify(log1)
  const harness = /^Let (home-room\/\S+ \(Chariox agent\)) acquire a machine\?$/.exec(cards[0]?.message ?? "")?.[1]
  check("the owner was asked before the acquisition (default setting), naming the room and agent", cards.length === 1 && !!harness && cards[0].choices.join() === "accept,decline", { card: cards[0] })
  const ref = /"allocation_ref\\?":\\?"([^"\\]+)/.exec(answered)?.[1]
  const kernelId = /"kernel_id\\?":\\?"([^"\\]+)/.exec(answered.slice(answered.indexOf("kernel_target")))?.[1]
  check("the machine became ready and its access is available", /access_state\\?":\\?"available/.test(answered), { ref, kernelId })
  const listed = log1.findIndex(row => row.call === "kernels")
  check("kernels lists the machine", !!kernelId && listed >= 0 && String(log1[listed + 1]?.answer ?? "").includes(kernelId), { kernelId })
  check("the new machine has no room yet", /SessionsListed[^}]*\[\]/.test(answered.replace(/\\"/g, '"')), {})
  writeFileSync(join(evidence, "machine.json"), JSON.stringify({ ref, kernelId, harness }, null, 2))
  photograph("running")

  // 2. The agent creates the Room on the machine (room-level, no /sudo) and runs one command there.
  const text2 = await turn([
    "This is a drill. Use only the chariox-reach MCP tools. Do not retry a failed call.",
    `The machine's kernel id is ${kernelId}. With the \`kernel\` argument set to it:`,
    "1. request CreateSession {\"workspace_id\":\"/home/chariox\",\"worktree_id\":\"/home/chariox\",\"alias\":null,\"slice_ref\":null}; take SessionCreated.session.id.",
    "2. request RoomTerminal {\"session_id\":\"SESSION_ID\",\"operation\":{\"operation\":\"exec_open\",\"command\":[\"uname\",\"-a\"]}}; take exec_opened.exec_id.",
    "3. request RoomTerminal {\"session_id\":\"SESSION_ID\",\"operation\":{\"operation\":\"exec_read\",\"exec_id\":\"EXEC_ID\",\"after_cursor\":0,\"wait_ms\":10000}} until the answer has an outcome; then request RoomTerminal exec_close for the exec_id.",
    "Start your reply with the exact token CMDRAN_DONE, then give the decoded stdout of the command.",
  ].join("\n"), "CMDRAN_DONE")
  saveToolLog()
  const log2 = JSON.stringify(text2)
  check("the agent created a Room on the machine and ran the command there without /sudo", /SessionCreated/.test(log2) && /exited/.test(log2) && (/TGludXg/.test(log2) || /Linux/.test(log2)), {})

  // 3. Stop it and photograph the stopped machine, then release it.
  const text3 = await turn([
    "This is a drill. Use only the chariox-reach MCP tools, and your shell for `sleep` only. Do not retry a failed call unless told to.",
    `The machine is allocation_ref ${ref}.`,
    `1. Call \`request\` with ${request({ ControlPlane: { action: "capabilities" } })} for the revision, and ${request({ ControlPlane: { action: "get", allocation_ref: ref } })} for its current revision. If available_actions does not contain "stop" yet, run \`sleep 10\` and ask again, up to 12 times.`,
    `2. Stop it: ${request({ ControlPlane: { action: "stop", allocation_ref: ref, capability_revision: "REVISION", request_id: `reach-drill-stop-${runId}`, expected_revision: "REV" } })} (REV is the number revision of the latest get answer). Then up to 40 times: run \`sleep 10\` and call get until state is stopped.`,
    "Start your reply with the exact token STOPPED_DONE and the last state you saw.",
  ].join("\n"), "STOPPED_DONE")
  saveToolLog()
  check("the agent stopped the machine", /"state\\?":\\?"stopped/.test(JSON.stringify(text3)))
  photograph("stopped")
  const text4 = await turn([
    "This is a drill. Use only the chariox-reach MCP tools, and your shell for `sleep` only. Do not retry a failed call unless told to.",
    `The machine is allocation_ref ${ref}, stopped.`,
    `1. Call \`request\` with ${request({ ControlPlane: { action: "capabilities" } })} for the revision, and ${request({ ControlPlane: { action: "get", allocation_ref: ref } })} for its current revision.`,
    `2. Release it: ${request({ ControlPlane: { action: "release", allocation_ref: ref, capability_revision: "REVISION", request_id: `reach-drill-release-${runId}`, expected_revision: "REV" } })} with the revision of the latest get answer, then call get up to 30 times (with \`sleep 10\`) until it answers a state of deleting or deleted or refuses because the machine is gone.`,
    "Start your reply with the exact token RELEASED_DONE and the last state you saw.",
  ].join("\n"), "RELEASED_DONE")
  saveToolLog()
  check("the agent released the machine", /release/.test(JSON.stringify(text4)) && /(deleting|deleted|not_found|gone)/.test(JSON.stringify(text4)))
  photograph("released")
}
try {
  await main().finally(() => saveToolLog())
  note(failures.length ? `${label}: RED ${failures.join("; ")}` : `${label}: PASS`)
  if (failures.length) process.exitCode = 1
} catch (error) {
  note("FAIL", { message: stripped(String(error.message)) }); process.exitCode = 1
} finally {
  const stopped = await Promise.allSettled(children.toReversed().map(stop))
  const failed = stopped.some(result => result.status === "rejected")
  if (!failed && (!failures.length || flags.get("--discard") === "yes")) rmSync(root, { recursive: true, force: true })
  else process.exitCode = 1
  note("cleanup", { stateRemoved: !failed, liveChildren: children.filter(c => c.exitCode === null && c.signalCode === null).map(c => c.pid) })
}

// A real kernel raises a choice through a real Claude Haiku agent (`request_popup`); an outside holder
// learns it from the `interaction pending` event alone and answers it through its own harness.
// Scenarios: native (interactive Claude Code shows its AskUserQuestion), elicit (`chariox events mcp`: Claude Code shows
// its own elicitation dialog with no model turn),  kinds (owner-only decisions stay
// pointers), first-wins (two holders answer, one wins), expiry (nobody answers, the late answer is refused).
// Usage: node outside-choice.mjs --scenario S --kernel-binary K --cli-wrapper W --state-root DIR --evidence-dir DIR --login-file F
// Disposable kernel, vault and passkey; the Claude login is read from --login-file, stored in that kernel's Vault
// and handed to the outside Claude Code, never printed.
import assert from "node:assert/strict"
import { execFileSync, spawn } from "node:child_process"
import { randomBytes } from "node:crypto"
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { DatabaseSync } from "node:sqlite"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { assert.equal(typeof flags.get(key), "string", `${key} is required`); return flags.get(key) }
const scenario = need("--scenario")
assert(["native", "elicit", "opencode", "kinds", "first-wins", "expiry"].includes(scenario), "unknown scenario")
const kernelBinary = resolve(need("--kernel-binary")), cliWrapper = resolve(need("--cli-wrapper"))
const stateRoot = resolve(need("--state-root")), evidence = resolve(need("--evidence-dir")), loginFile = resolve(need("--login-file"))
assert(!evidence.startsWith(fileURLToPath(new URL("../../", import.meta.url))), "evidence must be outside the repository")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const dir = mkdtempSync(join(stateRoot, "choice-"))
const steps = [], owned = []
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 2) + "\n", { mode: 0o600 })
  console.log(name, JSON.stringify(data))
}
const until = async (check, what, timeout = 30000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) { const v = await check(); if (v) return v; await delay(200) }
  throw new Error(`timeout: ${what}`)
}
const freePort = () => new Promise(done => { const s = createServer().listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => done(p)) }) })
const identity = pid => {
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, "utf8")
    const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ")
    return { ppid: Number(fields[1]), start: fields[19] }
  } catch { return null }
}
// Only a process this drill started, identified by pid and start time, is ever signalled.
const stopOwned = ({ pid, start }) => { if (pid > 1 && identity(pid)?.start === start) process.kill(pid, "SIGKILL") }
const own = child => { const handle = { pid: child.pid, start: identity(child.pid).start }; owned.push(handle); return handle }

const ports = await Promise.all([freePort(), freePort(), freePort(), freePort()])
for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state", "claude", "out"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
const passkey = randomBytes(24).toString("hex")
writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
const bin = join(dir, "bin"); mkdirSync(bin)
writeFileSync(join(bin, "chariox"), `#!/bin/sh\nexec "${cliWrapper}" "$@"\n`); chmodSync(join(bin, "chariox"), 0o755)
const alias = "choice-drill"
const kernelEnv = { PATH: `${bin}:${process.env.PATH}`, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash",
  HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
  CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
  CHARIOX_DAEMON_SOCKET: join(dir, "daemon.sock"), CHARIOX_DAEMON_ALIAS: alias, CHARIOX_MACHINE_ALIAS: alias,
  CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }

const logFd = openSync(join(evidence, "kernel.log"), "a", 0o600)
const kernelChild = spawn(kernelBinary, [], { env: kernelEnv, stdio: ["ignore", logFd, logFd] })
own(kernelChild)
note("kernel-start", { pid: kernelChild.pid, scenario })
const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: kernelEnv, controlRequestRetryDeadlineMs: 1000 })
const rpc = async request => { const r = await client.send(request); if (r.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${r.Error.code}`); return r }
await until(async () => { try { return !!(await rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel ready")

const tmux = (...args) => execFileSync("tmux", args, { encoding: "utf8" })
const tmuxName = `choice-${scenario}-${process.pid}`
let tmuxStarted = false
const screen = () => tmux("capture-pane", "-p", "-t", tmuxName, "-S", "-200")
const snap = (label) => { writeFileSync(join(evidence, `screen-${label}.txt`), screen(), { mode: 0o600 }) }
const events = () => {
  const db = new DatabaseSync(join(dir, "chariox/state.db"), { readOnly: true })
  try { return db.prepare("select kind, subject_id as subject, payload_json from durable_state_events where kind = 'interaction.answered' order by sequence").all().map(r => ({ kind: r.kind, subject: r.subject, payload: JSON.parse(r.payload_json) })) } finally { db.close() }
}

const results = { scenario }
try {
  const made = (await rpc({ CreateSession: { workspace_id: join(dir, "work"), worktree_id: join(dir, "work"), alias } })).SessionCreated
  const sessionId = made.session.id
  const att = (await rpc({ AttachToSession: { session_id: sessionId, client_id: alias, capability_level: "FullTerminal" } })).SessionAttached
  await client.subscribeToKernelEvents(sessionId, att.attachment.id)
  const unlock = rpc({ ManageCredentialVault: { session_id: sessionId, agent_id: made.agent.id } }); unlock.catch(() => {})
  const prompt = await until(async () => (await rpc({ GetSessionState: { session_id: sessionId } })).SessionState.session.active_interactions?.find(x => x.id.startsWith("vault-unlock-")), "vault prompt")
  await rpc({ RespondToInteraction: { session_id: sessionId, interaction_id: prompt.id, choice_id: "passphrase", custom_reply: passkey } })
  await unlock
  // The owner's terminal approves every access grant, and answers critical decisions when the scenario asks.
  client.onKernelEvent(async event => {
    if (event.event !== "passkey_prompts_changed") return
    for (const p of event.prompts || []) if (p.kind === "access_grant") {
      note("approve-grant", { requester: p.requester?.executable })
      await rpc({ RespondToInteraction: { session_id: p.session_id || "kernel-access", interaction_id: p.interaction_id, choice_id: "approve", passkey } }).catch(() => note("approval-failed"))
    }
  })
  const token = readFileSync(loginFile, "utf8").trim()
  await rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: token, overwrite: true } })
  note("asker-login-stored")
  const asker = (await rpc({ SpawnAgent: { session_id: sessionId, provider: "claude", model: "claude-haiku-5-5", permission_level: "yolo", alias: "asker" } })).AgentSpawned.agent
  const ask = async text => { await rpc({ SubmitPrompt: { session_id: sessionId, attachment_id: att.attachment.id, target_agent_id: asker.id, prompt: text, attachments: [] } }); note("asker-prompt", { text: text.slice(0, 80) }) }
  const popup = (title, message, choices, extra = "") => `Call your request_popup tool exactly once with title "${title}", message "${message}", timeout_sec ${extra || 300}, default_on_timeout "red", and choices ${JSON.stringify(choices)}. Wait for the reply, then run the shell command: printf '%s' '<the reply you received>' > ${join(dir, "work/chosen.txt")} . Then say DONE.`
  const colours = [{ id: "red", label: "Red", reply: "red" }, { id: "green", label: "Green", reply: "green" }, { id: "blue", label: "Blue", reply: "blue", style: "primary" }]
  const pendingInteractions = async () => (await rpc({ GetSessionState: { session_id: sessionId } })).SessionState.session.active_interactions ?? []

  // A scripted outside holder: its own process tree is the grant holder. It follows the session, logs every
  // record, and answers or tries to answer as its mode says.
  const holder = (name, mode) => {
    const out = join(dir, "out", name)
    mkdirSync(out)
    const script = join(out, "holder.sh")
    writeFileSync(script, `#!/bin/bash
cd "$(dirname "$0")"
chariox access request --until-exit > grant.json 2> grant.err || exit 2
chariox events subscribe --kernel ${alias} --session ${sessionId} > subscribe.json 2> subscribe.err || exit 3
cursor=0
while :; do
  chariox events wait --since $cursor --timeout 20 > wait.json 2> wait.err; rc=$?
  [ $rc -eq 5 ] && continue
  [ $rc -ne 0 ] && { echo "wait exit $rc" >> loop.log; sleep 1; continue; }
  cat wait.json >> records.jsonl; echo >> records.jsonl
  cursor=$(node -e 'const r=JSON.parse(require("fs").readFileSync("wait.json","utf8"));console.log(r.next_cursor)')
  for id in $(node -e 'const r=JSON.parse(require("fs").readFileSync("wait.json","utf8"));for(const e of r.events)if(e.type==="interaction"&&e.value.state==="pending")console.log(e.refs.interaction_id)'); do
    ${mode.startsWith("answer:") ? `printf '%s' '{"RespondToInteraction":{"session_id":"${sessionId}","interaction_id":"'"$id"'","choice_id":"${mode.split(":")[1]}"}}' | chariox request > "answer-$id.json" 2> "answer-$id.err"; echo $? > "answer-$id.rc"` : ":"}
  done
  ${mode.startsWith("late:") ? `for id in $(node -e 'const r=JSON.parse(require("fs").readFileSync("wait.json","utf8"));for(const e of r.events)if(e.type==="interaction"&&e.value.state==="resolved")console.log(e.refs.interaction_id)'); do
    printf '%s' '{"RespondToInteraction":{"session_id":"${sessionId}","interaction_id":"'"$id"'","choice_id":"${mode.split(":")[1]}"}}' | chariox request > "late-$id.json" 2> "late-$id.err"; echo $? > "late-$id.rc"
  done` : ":"}
  chariox events ack $cursor > /dev/null 2>&1
done
`, { mode: 0o755 })
    // The kernel names the holder two levels above the CLI: the outer shell makes it this holder's own.
    const child = spawn("bash", ["-c", `bash ${script}; true`], { env: kernelEnv, stdio: "ignore" })
    const handle = own(child)
    note("holder-start", { name, pid: child.pid, mode })
    return { name, out, handle, records: () => (existsSync(join(out, "records.jsonl")) ? readFileSync(join(out, "records.jsonl"), "utf8").split("\n").filter(Boolean).flatMap(l => JSON.parse(l).events) : []), file: f => (existsSync(join(out, f)) ? readFileSync(join(out, f), "utf8").trim() : null) }
  }
  const subscribed = async h => until(() => h.file("subscribe.json"), `${h.name} subscribed`, 90000)

  if (scenario === "native" || scenario === "elicit" || scenario === "opencode") {
    // The outside agent is an interactive Claude Code; the kernel asks through a Haiku agent.
    const cfg = join(dir, "claude")
    writeFileSync(join(cfg, ".claude.json"), JSON.stringify({ hasCompletedOnboarding: true, theme: "dark", projects: { [join(dir, "work")]: { hasTrustDialogAccepted: true } } }))
    // The login reaches Claude Code only through its environment: the launcher reads it from the file.
    const mcpConfig = join(dir, "mcp.json")
    writeFileSync(mcpConfig, JSON.stringify({ mcpServers: { chariox: { command: "chariox", args: ["events", "mcp"] } } }))
    const launcher = join(dir, "launch-claude.sh")
    writeFileSync(launcher, `#!/bin/bash
cd ${join(dir, "work")}
exec env -i 'PATH=${kernelEnv.PATH}' 'HOME=${kernelEnv.HOME}' 'TMPDIR=${kernelEnv.TMPDIR}' 'CHARIOX_HOME=${kernelEnv.CHARIOX_HOME}' 'CHARIOX_DAEMON_SOCKET=${kernelEnv.CHARIOX_DAEMON_SOCKET}' 'CLAUDE_CONFIG_DIR=${cfg}' "CLAUDE_CODE_OAUTH_TOKEN=$(cat ${loginFile})" 'TERM=xterm-256color' claude --model claude-haiku-5-5 --allowedTools ${scenario === "elicit" ? "Bash,mcp__chariox__chariox_next_choice --strict-mcp-config --mcp-config " + mcpConfig : "Bash"}
`, { mode: 0o700 })
    if (scenario === "opencode") {
      // OpenCode uses its fleet login through its own XDG directories; the model is its default free one.
      const fleet = "/root/.chariox/dev/provider-runtimes/fleet-accounts/opencode-home"
      writeFileSync(launcher, `#!/bin/bash
cd ${join(dir, "work")}
exec env -i 'PATH=${kernelEnv.PATH}' 'HOME=${kernelEnv.HOME}' 'TMPDIR=${kernelEnv.TMPDIR}' 'CHARIOX_HOME=${kernelEnv.CHARIOX_HOME}' 'CHARIOX_DAEMON_SOCKET=${kernelEnv.CHARIOX_DAEMON_SOCKET}' 'XDG_CONFIG_HOME=${fleet}/config' 'XDG_DATA_HOME=${fleet}/data' 'XDG_STATE_HOME=${fleet}/state' 'XDG_CACHE_HOME=${fleet}/cache' 'TERM=xterm-256color' opencode
`, { mode: 0o700 })
    }
    const sh = launcher
    tmux("new-session", "-d", "-s", tmuxName, "-x", "200", "-y", "50", sh)
    tmuxStarted = true
    note("outside-claude-start", { tmux: tmuxName })
    await until(() => scenario === "opencode" ? /Ask anything/.test(screen()) : /❯|>\s*$|Try "/.test(screen()), "outside harness ready", 60000)
    snap("01-ready")
    const mcpInstruction = `A test kernel runs on this machine and the \`chariox\` command talks to it (its name is ${alias}, its session is ${sessionId}). Get access with \`chariox access request --until-exit\` and follow that session with \`chariox events subscribe\`; then call your chariox_next_choice tool once and tell me the choice it returns.`
    const instruction = scenario === "elicit" ? mcpInstruction : scenario === "opencode" ? `A test kernel runs on this machine and the \`chariox\` command talks to it (its name is ${alias}, its session is ${sessionId}). Run \`chariox guide\` and follow it: get access with \`chariox access request --until-exit\`, follow that session with \`chariox events subscribe\`, then run \`chariox events wait --timeout 240\` in the foreground. When it reports a decision for the person, handle it as the guide says. Stop when the decision has been answered and tell me the choice.` : `A test kernel runs on this machine and the \`chariox\` command talks to it (its name is ${alias}, its session is ${sessionId}). Run \`chariox guide\` and follow it: get access with \`chariox access request --until-exit\`, follow that session with \`chariox events subscribe\`, then keep \`chariox events wait --timeout 540\` running as a background command. When it reports a decision for the person, handle it as the guide says. Stop when the decision has been answered and tell me the choice.`
    tmux("send-keys", "-t", tmuxName, "-l", instruction)
    await delay(1500)
    tmux("send-keys", "-t", tmuxName, "Enter")
    // A key that arrives while the box is still redrawing is lost; Enter on an empty box does nothing.
    const grants = async () => (await rpc({ ListKernelAccessGrants: {} })).KernelAccessGrantsListed.grants.length
    for (let i = 0; i < 4 && !(await grants()); i++) { await delay(8000); if (!(await grants())) tmux("send-keys", "-t", tmuxName, "Enter") }
    // Wait until the outside agent holds a grant, follows the session and has a background wait running.
    await until(async () => (await rpc({ ListKernelAccessGrants: {} })).KernelAccessGrantsListed.grants.length > 0, "grant approved", 180000)
    await until(() => scenario === "elicit" ? /chariox_next_choice|chariox-events|Called chariox/i.test(screen()) : scenario === "opencode" ? /events wait/.test(screen()) : /events wait/.test(screen()) && /ackground|running|Running/.test(screen()), scenario === "elicit" ? "the next-choice tool is running" : "background wait running", 180000)
    snap("02-waiting")
    await delay(3000)
    await ask(popup("Colour", "Which colour should the report use?", colours))
    const pendingId = (await until(async () => (await pendingInteractions())[0], "question pending", 120000)).id
    const shown = await until(() => { const s = screen(); return (scenario === "elicit" ? /requests your input/.test(s) && /Which colour/.test(s) && /Accept/.test(s) : scenario === "opencode" ? /Red/.test(s) && /Green/.test(s) && /Blue/.test(s) : /Enter to select/.test(s) && /Green/.test(s) && /Blue/.test(s)) && s }, "native question shown", 180000)
    snap("03-question")
    results.question_shown = true
    if (scenario === "elicit" || scenario === "opencode") {
      // The keys the person presses in the elicitation dialog: the test script picks Green.
      // Without --keys the drill stops here for a person to press them; `touch <state>/go` resumes it.
      if (!flags.has("--keys")) { note("explore", { tmux: tmuxName, go: join(dir, "go") }); await until(() => existsSync(join(dir, "go")), "a person finished the dialog", 900000) }
      else for (const key of JSON.parse(flags.get("--keys"))) { tmux("send-keys", "-t", tmuxName, key); await delay(500) }
    } else {
    // Claude Code lays the options out as it likes; the person picks Green wherever it is.
    const options = shown.split("\n").map(l => /^\s*(❯)?\s*(\d)\.\s+(\w+)/.exec(l)).filter(Boolean).map(m => ({ cursor: !!m[1], label: m[3] }))
    const target = options.findIndex(o => o.label === "Green"), at = options.findIndex(o => o.cursor)
    assert(target >= 0 && at >= 0, "the dialog lists the options")
    results.dialog_options = options.map(o => o.label)
    for (let i = 0; i < Math.abs(target - at); i++) { tmux("send-keys", "-t", tmuxName, target > at ? "Down" : "Up"); await delay(300) }
    }
    snap("04-selected")
    if (scenario === "native") tmux("send-keys", "-t", tmuxName, "Enter")
    await until(() => existsSync(join(dir, "work/chosen.txt")), "asker continued with the reply", 180000)
    results.chosen = readFileSync(join(dir, "work/chosen.txt"), "utf8")
    assert.equal(results.chosen, "green", "the asker's turn continues with the person's choice")
    await delay(8000)
    snap("05-after")
    if (scenario === "opencode") {
      // OpenCode keeps no transcript file here: the screens, the answer and the audit are the evidence.
      const answered = events().filter(e => e.subject === pendingId)
      results.audit = answered.map(e => ({ subject: e.subject, ...e.payload }))
      assert.equal(answered.length, 1)
      assert.equal(answered[0].payload.choice_id, "green")
      assert.equal(answered[0].payload.connection_class, "external_agent")
    } else {
    // The record alone was enough: look at what the outside agent saw and did in its own transcript.
    const jsonl = []
    const walk = p => { for (const e of readdirSync(p, { withFileTypes: true })) { const q = join(p, e.name); if (e.isDirectory()) walk(q); else if (q.endsWith(".jsonl")) jsonl.push(q) } }
    walk(join(cfg, "projects"))
    const transcript = jsonl.map(f => readFileSync(f, "utf8")).join("\n")
    writeFileSync(join(evidence, "outside-transcript.jsonl"), transcript, { mode: 0o600 })
    results.record_carried_question = /\\"question\\"\s*:\s*\{|\\"question\\":\{/.test(transcript)
    // The guide lists a GetSessionState example, so only what the agent itself ran counts.
    const commands = transcript.split("\n").filter(Boolean).flatMap(line => { try { return JSON.parse(line).message?.content ?? [] } catch { return [] } })
      .filter(part => part.type === "tool_use").map(part => JSON.stringify(part.input))
    results.outside_commands = commands.map(c => c.slice(0, 160))
    results.read_session_state = commands.some(c => /GetSessionState/.test(c))
    results.used_ask_user_question = transcript.split("\n").filter(Boolean).some(line => { try { return (JSON.parse(line).message?.content ?? []).some(p => p.type === "tool_use" && p.name === "AskUserQuestion") } catch { return false } })
    assert.equal(results.read_session_state, false, "the outside agent did not need GetSessionState")
    if (scenario === "elicit") {
      // The harness drew the dialog from the tool call: no model turn sits between the call and its result.
      const entries = transcript.split("\n").filter(Boolean).flatMap(line => { try { return [JSON.parse(line)] } catch { return [] } })
      const parts = entry => entry.message?.content ?? []
      const call = entries.findIndex(e => e.type === "assistant" && parts(e).some(p => p.type === "tool_use" && p.name === "mcp__chariox__chariox_next_choice"))
      const toolId = parts(entries[call] ?? {}).find(p => p.type === "tool_use" && p.name === "mcp__chariox__chariox_next_choice")?.id
      const back = entries.findIndex((e, i) => i > call && e.type === "user" && parts(e).some(p => p.type === "tool_result" && p.tool_use_id === toolId))
      assert(call >= 0 && back > call, "the transcript holds the tool call and its result")
      results.assistant_messages_between = entries.slice(call + 1, back).filter(e => e.type === "assistant").length
      assert.equal(results.assistant_messages_between, 0, "no model turn between the pending decision and the answer")
      assert.equal(results.used_ask_user_question, false, "the harness drew the dialog, not the model")
    } else {
      assert.equal(results.record_carried_question, true, "the pending record carried the question")
      assert.equal(results.used_ask_user_question, true, "the person was asked with Claude Code's own question tool")
    }
    const answered = events().filter(e => e.subject === pendingId)
    results.audit = answered.map(e => ({ subject: e.subject, ...e.payload }))
    assert.equal(answered.length, 1)
    assert.equal(answered[0].payload.choice_id, "green")
    assert.equal(answered[0].payload.connection_class, "external_agent")
    }
  }

  if (scenario === "first-wins") {
    const a = holder("holder-a", "answer:red"), b = holder("holder-b", "answer:green")
    await Promise.all([subscribed(a), subscribed(b)])
    await delay(1000)
    await ask(popup("Colour", "Which colour should the report use?", colours))
    const raceId = (await until(async () => (await pendingInteractions())[0], "question pending", 120000)).id
    await until(() => existsSync(join(dir, "work/chosen.txt")), "the question was answered", 180000)
    await delay(4000)
    const rc = h => readdirSync(h.out).filter(f => f.endsWith(".rc")).map(f => ({ rc: Number(h.file(f)), err: h.file(f.replace(".rc", ".err")) }))
    results.holder_a = rc(a); results.holder_b = rc(b)
    const codes = [...results.holder_a, ...results.holder_b]
    assert.equal(codes.length, 2, "both holders tried to answer")
    assert.equal(codes.filter(c => c.rc === 0).length, 1, "exactly one answer wins")
    const loser = codes.find(c => c.rc !== 0)
    results.loser = loser
    assert(/interaction_closed/.test(loser.err), "the loser gets interaction_closed")
    const resolved = [...a.records(), ...b.records()].filter(e => e.type === "interaction" && e.value.state === "resolved")
    results.resolved = resolved.map(e => e.value.resolution)
    assert(resolved.every(e => e.value.resolution.via === "external_agent"), "the close names the external agent")
    results.audit = events().filter(e => e.subject === raceId).map(e => e.payload)
    assert.equal(results.audit.length, 1, "one audit record")
  }

  if (scenario === "expiry") {
    const a = holder("holder-a", "late:red")
    await subscribed(a)
    await ask(popup("Colour", "Which colour should the report use?", colours, "60"))
    const pend = await until(async () => (await pendingInteractions())[0], "question pending", 120000)
    results.interaction_id = pend.id
    await until(() => a.records().some(e => e.type === "interaction" && e.value.state === "pending" && e.value.question), "holder saw the question", 30000)
    results.question_expires_at_ms = a.records().find(e => e.value?.question).value.question.expires_at_ms
    await until(async () => (await pendingInteractions()).length === 0, "kernel applied the timeout", 150000)
    const closed = await until(() => a.records().find(e => e.type === "interaction" && e.value.state === "resolved"), "holder saw the close", 30000)
    results.resolution = closed.value.resolution
    assert.equal(closed.value.resolution.via, "timeout")
    assert.equal(closed.value.resolution.choice_id, "red", "the close names the default the kernel applied")
    // The holder answers once the decision is closed: refused with the stable code.
    const lateRc = await until(() => a.file(`late-${pend.id}.rc`), "the holder's late answer", 60000)
    results.late_answer = { rc: Number(lateRc), err: (a.file(`late-${pend.id}.err`) ?? "").slice(0, 300) }
    assert.notEqual(results.late_answer.rc, 0)
    assert(/interaction_closed/.test(results.late_answer.err), "a late answer gets interaction_closed")
    assert.equal(events().filter(e => e.subject === pend.id).length, 0, "nobody answered, so no answer was audited for it")
  }

  if (scenario === "kinds") {
    const a = holder("holder-a", "answer:deny")
    await subscribed(a)
    // A critical decision and a secret request from the asker, and a sudo request from the holder itself.
    await ask(`Call your request_popup tool exactly once with title "Delete", message "Erase the archive?", level "critical", timeout_sec 300, and choices [{"id":"deny","label":"Deny","reply":"deny"},{"id":"erase","label":"Erase","reply":"erase","style":"danger"}]. Then say DONE.`)
    const critical = await until(async () => (await pendingInteractions()).find(i => i.level === "critical"), "critical pending", 120000)
    await until(() => a.records().some(e => e.refs?.interaction_id === critical.id), "holder told of the critical decision", 30000)
    const attempt = id => until(() => a.file(`answer-${id}.rc`), `the holder's attempt on ${id}`, 60000)
    await attempt(critical.id)
    // The owner's terminal answers the critical decision (with the passkey) and the holder sees how it closed.
    await rpc({ RespondToInteraction: { session_id: sessionId, interaction_id: critical.id, choice_id: "deny", passkey } })
    const closed = await until(() => a.records().find(e => e.refs?.interaction_id === critical.id && e.value?.state === "resolved"), "holder told the owner closed it", 30000)
    results.owner_close = closed.value.resolution
    assert.equal(closed.value.resolution.via, "terminal")
    results.audit = events().filter(e => e.subject === critical.id).map(e => e.payload)
    assert.equal(results.audit.length, 1, "the owner's answer was audited once")
    // Then a secret request: the value is entered by the person, never by a holder.
    await ask(`Call your request_credential_secret tool exactly once to ask the person for a secret named drill-secret (prompt "Paste the drill secret"). Then say DONE.`)
    const secret = await until(async () => (await pendingInteractions()).find(i => i.id !== critical.id && i.custom_choice?.input_kind === "secret"), "secret pending", 120000)
      .catch(async error => { results.interactions_at_timeout = (await pendingInteractions()).map(i => ({ id: i.id, level: i.level, input: i.custom_choice?.input_kind })); throw error })
    await until(() => a.records().some(e => e.refs?.interaction_id === secret.id), "holder told of the secret request", 30000)
    await attempt(secret.id)
    const told = a.records().filter(e => e.type === "interaction" && e.value.state === "pending")
    results.records = told.map(e => ({ id: e.refs.interaction_id, summary: e.summary, has_question: !!e.value.question, pointer: e.value.pointer?.one_line, kind: e.value.pointer?.kind }))
    for (const r of results.records) {
      assert.equal(r.summary, "owner approval", `${r.id}: the summary hides the prompt`)
      assert.equal(r.has_question, false, `${r.id}: no question for an owner-only decision`)
      assert(/^Approval pending on kernel .*Open Chariox there and answer its approval popup\.$/.test(r.pointer), `${r.id}: the pointer text`)
    }
    assert.equal(results.records.length, 2)
    // The holder's attempts: refused as human-only (exit 3).
    results.attempts = readdirSync(a.out).filter(f => f.startsWith("answer-") && f.endsWith(".rc")).map(f => ({ id: f.slice(7, -3), rc: Number(a.file(f)), err: (a.file(f.replace(".rc", ".err")) ?? "").slice(0, 200) }))
    assert(results.attempts.length === 2 && results.attempts.every(x => x.rc === 3 && /kernel_access_human_only/.test(x.err)), "every attempt is refused as human-only")
  }

  note("result", results)
} catch (error) {
  results.failure = String(error.message ?? error).slice(0, 300)
  throw error
} finally {
  writeFileSync(join(evidence, "result.json"), JSON.stringify(results, null, 2) + "\n")
  // What each scripted holder was told and answered, without its grant file.
  for (const name of existsSync(join(dir, "out")) ? readdirSync(join(dir, "out")) : []) {
    const target = join(evidence, name)
    mkdirSync(target, { recursive: true })
    for (const file of readdirSync(join(dir, "out", name)).filter(f => f === "records.jsonl" || /^(answer|late)-/.test(f))) copyFileSync(join(dir, "out", name, file), join(target, file))
  }
  if (tmuxStarted) { try { snap("99-final") } catch { /* the session may be gone */ } try { tmux("kill-session", "-t", tmuxName) } catch { /* already gone */ } }
  for (const p of [...owned].reverse()) stopOwned(p)
  client.close?.()
  note("cleanup", { stopped: owned.length })
}
process.exit(0)

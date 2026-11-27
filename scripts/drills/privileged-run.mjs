// Row 575 / D4: a kernel-owned privileged run on a real kernel, a real sudo and a real Claude agent.
// Run as the throwaway drill user on a machine where that user has a sudoers line that asks for its password:
//   node privileged-run.mjs --kernel-binary <kernel> --state-root <dir> --evidence-dir <dir> --pw-file <file> --canary-file <file>
//   < claude-token   (the Claude login is read from stdin, stored in the drill kernel's Vault and never printed)
// pw-file holds the drill user's sudo password, canary-file another random string. Both go into the drill kernel's
// Vault through the owner's own Create request; neither is ever printed, put in argv or sent to the agent.
// Cells (CELLS.md): P1 agent whoami, P2 never seen, P3 wrong password, P5 owner request, P6 after hot reload,
// P7 the agent's own shell still cannot sudo, P8 fresh authentication every use, P9 the kernel may gain privileges.
import { spawn, spawnSync } from "node:child_process"
import { randomBytes } from "node:crypto"
import { closeSync, mkdirSync, mkdtempSync, openSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
const binary = need("--kernel-binary"), stateRoot = need("--state-root"), evidence = need("--evidence-dir")
const password = readFileSync(need("--pw-file"), "utf8").trim(), canary = readFileSync(need("--canary-file"), "utf8").trim()
const token = readFileSync(0, "utf8").trim()
if (!password || !canary || !token || password === canary) throw new Error("password, canary and token files must each hold a distinct value")
mkdirSync(evidence, { recursive: true, mode: 0o700 })
const root = mkdtempSync(join(stateRoot, "pr-"))
const checks = [], steps = [], events = [], t0 = Date.now()
const secrets = [password, canary, token]
const redact = text => secrets.reduce((t, s) => t.split(s).join("<redacted>"), String(text)).replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")
const note = (name, data = {}) => { steps.push({ at: new Date().toISOString(), name, ...JSON.parse(redact(JSON.stringify(data))) }); console.log(`+${Math.round((Date.now() - t0) / 1000)}s ${name} ${redact(JSON.stringify(data)).slice(0, 300)}`) }
const check = (id, passed, detail) => { checks.push({ id, status: passed ? "PASS" : "FAIL", detail: JSON.parse(redact(JSON.stringify(detail ?? null))) }); note(`${passed ? "PASS" : "FAIL"} ${id}`, detail) }
/** How many times a secret value is in a text: the value itself is never kept or printed. */
const seen = text => secrets.map(s => String(text).split(s).length - 1).reduce((a, b) => a + b, 0)
const freePort = async () => { const s = createServer(); await new Promise(r => s.listen(0, "127.0.0.1", r)); const { port } = s.address(); await new Promise(r => s.close(r)); return port }

let child, client, rpc, sessionId, attachment
async function startKernel(env, log) {
  const fd = openSync(log, "a", 0o600)
  child = spawn(binary, [], { env, stdio: ["ignore", fd, fd] })
  closeSync(fd)
  note("kernel-start", { pid: child.pid })
}
async function connect(env, port) {
  client = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  rpc = async request => { const r = await client.send(request); if (r.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${r.Error.code} ${r.Error.message ?? ""}`); return r }
  for (const end = Date.now() + 60000; ; await delay(1000)) { try { if ((await rpc({ RelayStatus: null })).RelayStatus) break } catch { if (Date.now() > end) throw new Error("kernel not ready") } }
  client.onKernelEvent(event => { if (events.length < 20000) events.push(JSON.stringify(event).slice(0, 4000)) })
}
const owner = request => rpc({ OwnerVault: request }).then(r => r.OwnerVault.result)
const run = (id, argv) => owner({ action: "run_privileged", id, argv })
async function entry(name, value, origin) {
  return (await owner({ action: "create", name, kind: "Command secret", value, use_binding: { target: { kind: "command_stdin", origin }, value_kind: "secret" } })).id
}
async function turn(text, label) {
  const out = (await rpc(submitPromptRequest(sessionId, attachment, agentId, text, []))).PromptSubmitted.outcome
  const promptId = Object.values(out)[0].prompt.id
  for (const end = Date.now() + 8 * 60000; Date.now() < end; await delay(3000)) {
    const outline = (await rpc({ GetSessionHistoryOutline: { session_id: sessionId, agent_ids: [agentId], latest_prompt_count: 4, include_prefetched_content: true } })).SessionHistoryOutline
    const t = outline.agents.find(a => a.agent_id === agentId)?.turns.find(x => x.prompt_id === promptId)
    if (t && t.lifecycle !== "open") {
      const json = redact(JSON.stringify(outline))
      writeFileSync(join(evidence, `history-${label}.json`), json, { mode: 0o600 })
      return { turn: t, outline, answer: [t.summary?.entry?.text, ...t.entries.filter(e => e.entry.kind === "provider_output").map(e => e.entry.text)].filter(Boolean).join("\n") }
    }
  }
  throw new Error(`turn ${label} did not settle`)
}
let agentId

async function main() {
  const dirs = Object.fromEntries(["chariox", "home", "tmp", "logs", "history", "config", "cache", "state", "work"].map(s => [s, join(root, s)]))
  for (const d of Object.values(dirs)) mkdirSync(d, { mode: 0o700 })
  writeFileSync(join(dirs.chariox, "config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dirs.chariox}/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dirs.chariox}/state.db"\n`, { mode: 0o600 })
  const ports = []; for (let i = 0; i < 4; i++) ports.push(await freePort())
  mkdirSync("/var/tmp/rch", { recursive: true, mode: 0o700 })
  const sockets = mkdtempSync("/var/tmp/rch/pr-")
  const env = { PATH: process.env.PATH, IS_SANDBOX: "1", SHELL: "/bin/bash", HOME: dirs.home, TMPDIR: dirs.tmp, XDG_CONFIG_HOME: dirs.config, XDG_CACHE_HOME: dirs.cache, XDG_STATE_HOME: dirs.state,
    CHARIOX_HOME: dirs.chariox, CHARIOX_LOG_DIR: dirs.logs, CHARIOX_SESSION_HISTORY_DIR: dirs.history, CHARIOX_DAEMON_SOCKET: join(sockets, "kernel.sock"), CHARIOX_DAEMON_ALIAS: "pr-drill", CHARIOX_MACHINE_ALIAS: "pr-drill-machine",
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
  const kernelLog = join(evidence, "kernel.log")
  await startKernel(env, kernelLog)
  await connect(env, ports[0])
  await rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase: randomBytes(24).toString("hex") } } })
  await rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: token, overwrite: true } })
  // The machine origin comes from the product's own refusal for an entry made for another use.
  const probe = await entry("probe", canary, "https://probe.invalid")
  const refusal = await run(probe, ["/bin/true"]).then(() => "", e => e.message)
  const origin = /origin (https:\/\/[^\s)]+)\)/.exec(refusal)?.[1]
  check("setup-origin-named-by-the-refusal", !!origin && refusal.includes("entry_wrong_machine"), { origin })
  await owner({ action: "delete", id: probe })
  const good = await entry("sudo of this machine", password, origin)
  const wrong = await entry("sudo canary", canary, origin)
  const status = readFileSync(`/proc/${child.pid}/status`, "utf8")
  check("P9-kernel-may-gain-privileges", /NoNewPrivs:\s*0/.test(status), { no_new_privs: /NoNewPrivs:\s*(\d)/.exec(status)?.[1] })
  const defaults = { provider: "claude", account_profile: "default", model: "claude-haiku-5-5", effort: "low", permission_level: "yolo" }
  const work = join(dirs.work, "room"); mkdirSync(work)
  const made = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: "pr-drill", agent_defaults: defaults } })).SessionCreated
  sessionId = made.session.id; agentId = made.agent.id
  attachment = (await rpc({ AttachToSession: { session_id: sessionId, client_id: "pr-owner", capability_level: "FullTerminal" } })).SessionAttached.attachment.id
  await client.subscribeToKernelEvents(sessionId, attachment)
  await rpc({ LaunchProviderRun: { session_id: sessionId, agent_id: agentId, adapter_key: "claude", provider: "claude", account_profile: "default", model: defaults.model, variant: null, native_tui: false } })

  const ask = ids => `Do these three things in order and then answer with exactly three lines: "1: <stdout of step 1>", "2: <the error text of step 2, first 120 characters>", "3: <the output of step 3>".
Step 1: call the Chariox tool chariox.privileged.run (MCP name mcp__chariox__chariox_privileged_run; if it is not loaded, load it first with ToolSearch query "select:mcp__chariox__chariox_privileged_run") with argv ["whoami"] and credential_id "${ids[0]}".
Step 2: call it again with argv ["whoami"] and credential_id "${ids[1]}" (it is expected to be refused).
Step 3: in your own shell run: sudo -n true; echo exit=$?`
  const first = await turn(ask([good, wrong]), "p1")
  note("p1-answer", { answer: first.answer.slice(0, 400) })
  check("P1-agent-whoami-prints-root-without-a-card", /^1:\s*root\s*$/m.test(first.answer) && !events.some(e => e.includes("vault-unlock") || e.includes("sudo-card")), { answer: first.answer.slice(0, 300) })
  check("P3-wrong-password-is-refused-with-a-next-step", /wrong_password/.test(first.answer) || /update the entry/i.test(first.answer), { answer: first.answer.slice(0, 300) })
  check("P7-the-agents-own-shell-still-cannot-sudo", /3:.*exit=[1-9]/s.test(first.answer) || /no new privileges/i.test(first.answer), { answer: first.answer.slice(0, 300) })
  const after = spawnSync("sudo", ["-n", "true"], { encoding: "utf8" })
  check("P8-no-cached-sudo-after-a-run", after.status !== 0, { status: after.status })

  const ran = await run(good, ["/bin/sh", "-c", "whoami; id -u"])
  check("P5-owner-request-runs-the-same-way", ran.outcome?.stdout?.trim() === "root\n0".trim() || ran.outcome?.stdout === "root\n0\n", { outcome: { ...ran.outcome, stdout: ran.outcome?.stdout, stderr: ran.outcome?.stderr } })
  const refused = await run(wrong, ["/bin/true"]).then(() => "", e => e.message)
  check("P5b-owner-wrong-password-names-the-next-step", refused.includes("wrong_password") && refused.includes("update the entry"), { refused })

  process.kill(child.pid, "SIGUSR2")
  await delay(8000)
  await connect(env, ports[0]).catch(() => {})
  await rpc({ RelayStatus: null })
  const second = await turn(`Call the Chariox tool chariox.privileged.run (MCP name mcp__chariox__chariox_privileged_run) with argv ["whoami"] and credential_id "${good}". Answer with exactly one line: "1: <stdout>".`, "p6")
  check("P6-after-a-hot-reload-the-agent-run-still-works", /^1:\s*root\s*$/m.test(second.answer), { answer: second.answer.slice(0, 200) })

  // P2: the values appear nowhere the model, the history or the logs could carry them.
  const found = {}
  found.agent_transcript_and_history = seen(first.answer) + seen(second.answer) + seen(JSON.stringify(first.outline)) + seen(JSON.stringify(second.outline))
  found.kernel_events = events.reduce((n, e) => n + seen(e), 0)
  found.session_state = seen(JSON.stringify((await rpc({ GetSessionState: { session_id: sessionId } })).SessionState))
  const files = []
  const walk = d => { for (const name of readdirSync(d)) { const p = join(d, name); const s = statSync(p); if (s.isDirectory()) walk(p); else if (s.size < 200 * 1024 * 1024) files.push(p) } }
  walk(root); walk(evidence)
  found.files_with_a_value = files.filter(p => p !== join(root, "unused") && seen(readFileSync(p, "latin1")) > 0).map(p => p.replace(root, "<state>"))
  check("P2-never-seen-in-transcript-history-events-state-and-logs", found.agent_transcript_and_history + found.kernel_events + found.session_state === 0 && found.files_with_a_value.length === 0, found)
}
try { await main() } catch (error) { check("drill-completed", false, { message: redact(error.message) }); process.exitCode = 1 }
finally {
  writeFileSync(join(evidence, "verdict.json"), JSON.stringify(checks, null, 2) + "\n")
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 1) + "\n")
  writeFileSync(join(evidence, "events.jsonl"), redact(events.join("\n")) + "\n", { mode: 0o600 })
  // A client of a kernel that handed itself over may never close: do not wait for it.
  await Promise.race([client?.close().catch(() => {}), delay(5000)])
  if (child?.pid > 1 && child.exitCode === null) { child.kill("SIGTERM"); await delay(3000); if (child.exitCode === null) child.kill("SIGKILL") }
  rmSync(root, { recursive: true, force: true })
  process.exit(checks.some(c => c.status === "FAIL") ? 1 : 0)
}

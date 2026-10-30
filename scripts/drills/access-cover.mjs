// A real external agent (Claude Code) starts two background processes under its grant and
// covers one by name. After the shell call that started them ends, the covered one keeps its
// access and the other is refused; revoking the grant ends the cover within 2 s.
// Usage: node access-cover.mjs --kernel-binary K --cli-wrapper W --state-root DIR --evidence-dir DIR [--no-cover]
// Disposable kernel, vault and passkey; the agent's login is read from --login-file and handed only to the agent.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { randomBytes } from "node:crypto"
import { chmodSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { dirname, join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], ["--no-cover", "--no-restart", "--end-by-exit"].includes(process.argv[i]) ? true : process.argv[++i])
const need = key => { assert.equal(typeof flags.get(key), "string", `${key} is required`); return resolve(flags.get(key)) }
const kernelBinary = need("--kernel-binary"), cliWrapper = need("--cli-wrapper"), stateRoot = need("--state-root"), evidence = need("--evidence-dir"), loginFile = need("--login-file")
const cover = !flags.has("--no-cover"), restart = !flags.has("--no-restart")
const endByExit = flags.has("--end-by-exit")
const holdSeconds = Number(flags.get("--hold-seconds") ?? 75)
assert(Number.isSafeInteger(holdSeconds) && holdSeconds >= 45, "--hold-seconds must be at least 45")
assert(!evidence.startsWith(fileURLToPath(new URL("../../", import.meta.url))), "evidence must be outside the repository")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const dir = mkdtempSync(join(stateRoot, "cover-"))
const steps = [], owned = []
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 2) + "\n", { mode: 0o600 })
  console.log(name, JSON.stringify(data))
}
const until = async (check, what, timeout = 30000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) { if (await check()) return; await delay(100) }
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

const ports = await Promise.all([freePort(), freePort(), freePort(), freePort()])
for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state", "bg", "claude"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
const passkey = randomBytes(24).toString("hex")
writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
const bin = join(dir, "bin"); mkdirSync(bin)
const wrapper = join(bin, "chariox"); writeFileSync(wrapper, `#!/bin/sh\nexec "${cliWrapper}" "$@"\n`); chmodSync(wrapper, 0o755)
writeFileSync(join(dir, "loop.sh"), `#!/bin/bash\nwhile :; do printf '%s %s\\n' "$(date +%s.%N)" "$(chariox access status --holder-pid $$ 2>&1 | tr -d '\\n')" >> "${dir}/bg/$1.log"; sleep 0.5; done\n`, { mode: 0o755 })
const kernelEnv = { PATH: `${bin}:${process.env.PATH}`, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash",
  HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
  CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
  CHARIOX_DAEMON_SOCKET: join(dir, "daemon.sock"), CHARIOX_DAEMON_ALIAS: "cover-drill", CHARIOX_MACHINE_ALIAS: "cover-drill",
  CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }

const startKernel = async label => {
  const log = openSync(join(evidence, "kernel.log"), "a", 0o600)
  const child = spawn(kernelBinary, [], { env: kernelEnv, stdio: ["ignore", log, log] })
  const handle = { pid: child.pid, start: identity(child.pid).start, child }
  owned.push(handle)
  note(label, { pid: child.pid })
  const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { localAuthEnvironment: kernelEnv, controlRequestRetryDeadlineMs: 1000 })
  const rpc = async request => { const r = await client.send(request); if (r.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${r.Error.code}`); return r }
  await until(async () => { try { return !!(await rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel ready")
  return { handle, client, rpc }
}
let { handle: kernel, client, rpc } = await startKernel("kernel-start")
let agent
try {
  const made = (await rpc({ CreateSession: { workspace_id: join(dir, "work"), worktree_id: join(dir, "work"), alias: "cover-drill" } })).SessionCreated
  const att = (await rpc({ AttachToSession: { session_id: made.session.id, client_id: "cover-drill", capability_level: "FullTerminal" } })).SessionAttached
  await client.subscribeToKernelEvents(made.session.id, att.attachment.id)
  const pending = rpc({ ManageCredentialVault: { session_id: made.session.id, agent_id: made.agent.id } }); pending.catch(() => {})
  let prompt
  await until(async () => { const st = (await rpc({ GetSessionState: { session_id: made.session.id } })).SessionState.session; prompt = st.active_interactions?.find(x => x.id.startsWith("vault-unlock-")); return prompt }, "vault prompt")
  await rpc({ RespondToInteraction: { session_id: made.session.id, interaction_id: prompt.id, choice_id: "passphrase", custom_reply: passkey } })
  await pending
  client.onKernelEvent(async event => {
    if (event.event !== "passkey_prompts_changed") return
    for (const p of event.prompts || []) if (p.kind === "access_grant") {
      note("approve-grant", { requester: p.requester?.executable })
      await rpc({ RespondToInteraction: { session_id: p.session_id || "kernel-access", interaction_id: p.interaction_id, choice_id: "approve", passkey } }).catch(() => note("approval-failed"))
    }
  })

  const pids = join(dir, "pids")
  const prompt1 = [
    "I am testing my own tool `chariox` on this machine against a disposable test kernel that I started for this test; nothing here touches anything else.",
    "`chariox access request` asks that kernel for permission for you to use it (my test harness approves it automatically). `chariox access status` prints whether the calling process may use the kernel.",
    `${dir}/loop.sh is a small poller I wrote: every half second it appends the output of \`chariox access status\` to a log file. Run these steps with the Bash tool, one command per step, exactly as written.`,
    "Step 1: run `chariox access request --until-exit` and wait until it prints a grant.",
    cover
      ? `Step 2: run this single command. It starts two pollers in the background and then runs \`chariox access cover --pid $P1\`, which tells the kernel that background process P1, which you just started, may keep using your permission after this shell command ends: nohup bash ${dir}/loop.sh p1 >/dev/null 2>&1 & P1=$!; nohup bash ${dir}/loop.sh p2 >/dev/null 2>&1 & P2=$!; chariox access cover --pid $P1; echo "$P1 $P2" > ${pids}`
      : `Step 2: run this single command, which starts two pollers in the background: nohup bash ${dir}/loop.sh p1 >/dev/null 2>&1 & P1=$!; nohup bash ${dir}/loop.sh p2 >/dev/null 2>&1 & P2=$!; echo "$P1 $P2" > ${pids}`,
    `Step 3: run \`sleep ${holdSeconds}\`.`,
    "Step 4: reply with the single word DONE.",
  ].join("\n")
  const model = "claude-haiku-5-5"
  const token = readFileSync(loginFile, "utf8").trim()
  agent = spawn("claude", ["-p", "--model", model, "--permission-mode", "bypassPermissions", "--allowedTools=Bash", prompt1],
    { env: { ...kernelEnv, CLAUDE_CODE_OAUTH_TOKEN: token, CLAUDE_CONFIG_DIR: join(dir, "claude"), BASH_DEFAULT_TIMEOUT_MS: String((holdSeconds + 120) * 1000), BASH_MAX_TIMEOUT_MS: String((holdSeconds + 120) * 1000) }, cwd: join(dir, "work"), stdio: ["ignore", openSync(join(evidence, "agent.out"), "a", 0o600), openSync(join(evidence, "agent.err"), "a", 0o600)] })
  const agentHandle = { pid: agent.pid, start: identity(agent.pid).start }
  owned.push(agentHandle)
  note("agent-start", { pid: agent.pid, model })
  await until(() => existsSync(pids) && readFileSync(pids, "utf8").trim().split(" ").length === 2, "agent started the background processes", 120000)
  const [p1, p2] = readFileSync(pids, "utf8").trim().split(" ").map(Number)
  const ids = { p1: { pid: p1, start: identity(p1).start }, p2: { pid: p2, start: identity(p2).start } }
  owned.push(ids.p1, ids.p2)
  note("background", { p1, p2, agent: agent.pid })
  await delay(3000)
  const ancestors = pid => { const chain = []; for (let at = identity(pid)?.ppid; at > 1 && chain.length < 64; at = identity(at)?.ppid) chain.push(at); return chain }
  assert(!ancestors(p1).includes(agent.pid) && !ancestors(p2).includes(agent.pid), "the background processes must be orphans of the agent")
  // The agent stays alive for holdSeconds; measure while it does, with time left to restart and revoke.
  await delay(holdSeconds > 75 ? (holdSeconds - 40) * 1000 : 8000)
  const sampled = label => readFileSync(join(dir, "bg", `${label}.log`), "utf8").trim().split("\n").map(l => ({ at: Number(l.split(" ")[0]), status: l.split(" ").slice(1).join(" ") }))
  const last = label => sampled(label).slice(-6).map(l => l.status)
  const granted = last("p1"), refused = last("p2")
  note("window", { p1_ppid: identity(p1)?.ppid, p2_ppid: identity(p2)?.ppid, p1_last: granted, p2_last: refused })
  const grants = (await rpc({ ListKernelAccessGrants: {} })).KernelAccessGrantsListed.grants
  note("grants", { count: grants.length, covered: grants.map(g => g.covered ?? []) })
  const orphanedAt = Date.now() / 1000 - (holdSeconds > 75 ? holdSeconds - 40 : 8)
  const results = { cover, samples: { p1: sampled("p1").length, p2: sampled("p2").length }, p1_never_refused: sampled("p1").every(l => l.status.includes('"status":"granted"')), p2_never_admitted_as_orphan: sampled("p2").filter(l => l.at > orphanedAt).every(l => !l.status.includes('"status":"granted"')), p1_granted: granted.every(l => l.includes('"status":"granted"')), p2_refused: refused.every(l => !l.includes('"status":"granted"')), covered_listed: grants.some(g => g.covered?.some(c => c.pid === p1)) }
  assert.equal(results.p2_refused, true, "the uncovered orphan must be refused")
  if (cover) {
    assert.equal(results.p1_granted, true, "the covered orphan must keep working")
    assert.equal(results.covered_listed, true, "the grant list must show the covered process")
    assert(results.p2_never_admitted_as_orphan, "the uncovered orphan must never be admitted")
    if (!restart) assert(results.p1_never_refused, "the covered process must never be refused while the grant and the process live")
    // The kernel restarts while the grant lives: the covered process is still covered.
    if (restart) {
    process.kill(kernel.pid, "SIGTERM")
    await until(() => identity(kernel.pid)?.start !== kernel.start, "kernel stopped", 30000)
    client.close?.()
    ;({ handle: kernel, client, rpc } = await startKernel("kernel-restart"))
    const restartedAt = Date.now() / 1000
    await delay(6000)
    const after = readFileSync(join(dir, "bg", "p1.log"), "utf8").trim().split("\n").filter(l => Number(l.split(" ")[0]) > restartedAt + 1)
    const afterP2 = readFileSync(join(dir, "bg", "p2.log"), "utf8").trim().split("\n").filter(l => Number(l.split(" ")[0]) > restartedAt + 1)
    results.after_restart = { p1: after.length, p1_granted: after.every(l => l.includes('"status":"granted"')), p2_refused: afterP2.every(l => !l.includes('"status":"granted"')) }
    assert(after.length >= 3 && results.after_restart.p1_granted, "the covered process must survive a kernel restart")
    assert(results.after_restart.p2_refused, "the uncovered orphan stays refused after the restart")
    const grantsAfter = (await rpc({ ListKernelAccessGrants: {} })).KernelAccessGrantsListed.grants
    assert(grantsAfter[0].covered?.some(c => c.pid === p1), "the restored grant lists the covered process")
    grants.splice(0, grants.length, ...grantsAfter)
    }
    const revokedAt = Date.now() / 1000
    // The grant ends by an explicit revoke, or because its holder (the agent) exits.
    if (endByExit) stopOwned(agentHandle)
    else await rpc({ RevokeKernelAccessGrant: { grant_id: grants[0].grant_id } })
    await until(() => readFileSync(join(dir, "bg", "p1.log"), "utf8").trim().split("\n").some(l => Number(l.split(" ")[0]) > revokedAt && l.includes('"status":"ended"')), "cover ended after the grant", 10000)
    const endedAt = Number(readFileSync(join(dir, "bg", "p1.log"), "utf8").trim().split("\n").find(l => Number(l.split(" ")[0]) > revokedAt && l.includes('"status":"ended"')).split(" ")[0])
    results.ended_by = endByExit ? "holder_exit" : "revoke"
    results.cover_ended_after_s = Number((endedAt - revokedAt).toFixed(2))
    assert(results.cover_ended_after_s <= 2, `cover ended ${results.cover_ended_after_s}s after the grant`)
  } else {
    assert.equal(results.p1_granted, false, "without a cover the orphan must be refused")
  }
  note("result", results)
  writeFileSync(join(evidence, "result.json"), JSON.stringify(results, null, 2) + "\n")
} finally {
  for (const p of [...owned].reverse()) stopOwned(p)
  client.close?.()
  note("cleanup", { stopped: owned.length })
}
process.exit(0)

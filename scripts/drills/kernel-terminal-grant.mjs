// An external agent holding a real access grant uses the kernel's own terminals with no Room.
// Real kernel (disposable home, vault and passkey), the real built CLI as the scripted holder. Proves, in order:
//   grant     `chariox access request --until-exit` is approved by the owner's terminal (a passkey answer);
//   one-shot  `chariox exec -- argv` (no --session, no --workspace) runs on the kernel, prints its output and returns the
//             program's exit code (7), starting in the home folder, and `--workspace` starts elsewhere;
//   terminal  `chariox terminal open | list | send --wait-ms | close` with no --session: two terminals, typed input is
//             answered, the list shows both with scope "kernel", close ends one;
//   no Room   the kernel lists exactly the sessions it had before (the drill's own), none made by the holder;
//   refused   a process outside the holder's tree gets kernel_access_not_covered for the same exec (exit 3).
// Usage: node kernel-terminal-grant.mjs --kernel-binary K --state-root DIR --evidence-dir DIR [--cli apps/cli/dist/index.js]
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { randomBytes } from "node:crypto"
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { userInfo } from "node:os"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"


const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { assert.equal(typeof flags.get(key), "string", `${key} is required`); return flags.get(key) }
const repo = fileURLToPath(new URL("../../", import.meta.url))
const kernelBinary = resolve(need("--kernel-binary")), stateRoot = resolve(need("--state-root")), evidence = resolve(need("--evidence-dir"))
const cli = resolve(flags.get("--cli") ?? join(repo, "apps/cli/dist/index.js"))
assert(!evidence.startsWith(repo), "evidence must be outside the repository")
assert(stateRoot.startsWith("/root/.chariox/dev/"), "state must be lane-owned")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const dir = mkdtempSync(join(stateRoot, "kterm-"))
const steps = [], owned = []
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 2) + "\n", { mode: 0o600 })
  console.log(name, JSON.stringify(data))
}
const until = async (check, what, timeout = 30000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) { const v = await check(); if (v) return v; await delay(100) }
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
// Only a process this drill started, identified by pid and start time, is ever signalled; its descendants go first.
const descendants = pid => readdirSync("/proc").filter(name => /^\d+$/.test(name) && identity(Number(name))?.ppid === pid).flatMap(name => [...descendants(Number(name)), Number(name)])
const stopOwned = ({ pid, start }, signal = "SIGKILL") => {
  if (!(pid > 1) || identity(pid)?.start !== start) return
  for (const child of descendants(pid)) if (child > 1) { try { process.kill(child, signal) } catch { /* already gone */ } }
  try { process.kill(pid, signal) } catch { /* already gone */ }
}
const own = child => { const handle = { pid: child.pid, start: identity(child.pid).start }; owned.push(handle); return handle }
const ms = () => Date.now()

const bin = join(dir, "bin"); mkdirSync(bin)
writeFileSync(join(bin, "chariox"), `#!/bin/sh\nexec node "${cli}" "$@"\n`); chmodSync(join(bin, "chariox"), 0o755)
const passkey = randomBytes(24).toString("hex")
const clockFile = join(dir, "outside-clock-ms")

// One disposable kernel: its own home, vault, ports and socket. `extra` is added to its environment.
async function kernelHome(name) {
  const home = join(dir, name)
  for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(home, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(home, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${home}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${home}/chariox/state.db"\n`, { mode: 0o600 })
  const ports = await Promise.all([freePort(), freePort(), freePort(), freePort()])
  const alias = `kterm-${name}`
  const env = { PATH: `${bin}:${process.env.PATH}`, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash",
    HOME: join(home, "home"), TMPDIR: join(home, "tmp"), XDG_CONFIG_HOME: join(home, "config"), XDG_CACHE_HOME: join(home, "cache"), XDG_STATE_HOME: join(home, "state"),
    CHARIOX_HOME: join(home, "chariox"), CHARIOX_LOG_DIR: join(home, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(home, "history"),
    CHARIOX_DAEMON_SOCKET: join(home, "daemon.sock"), CHARIOX_DAEMON_ALIAS: alias, CHARIOX_MACHINE_ALIAS: alias,
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]) }
  return { name, home, ports, alias, env, kernel: null, client: null, sessionId: null, attachment: null, agent: null }
}

async function start(k, extra = {}) {
  const logFd = openSync(join(evidence, `kernel-${k.name}.log`), "a", 0o600)
  const child = spawn(kernelBinary, [], { env: { ...k.env, ...extra }, stdio: ["ignore", logFd, logFd] })
  k.kernel = own(child)
  k.child = child
  note("kernel-start", { kernel: k.name, pid: child.pid, clock: Boolean(extra.CHARIOX_TEST_OUTSIDE_CLOCK_MS_FILE) })
  k.client = new LocalIpcClient(`ws://127.0.0.1:${k.ports[0]}/kernel`, { localAuthEnvironment: k.env, controlRequestRetryDeadlineMs: 1000 })
  k.rpc = async request => {
    const r = await k.client.send(request)
    if (r.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${r.Error.code}`)
    return r
  }
  await until(async () => { try { return !!(await k.rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, `${k.name} ready`, 60000)
  // The Room: made once, attached again after a restart. The vault prompt takes the drill's passphrase.
  if (!k.sessionId) {
    const made = (await k.rpc({ CreateSession: { workspace_id: join(k.home, "work"), worktree_id: join(k.home, "work"), alias: k.alias } })).SessionCreated
    k.sessionId = made.session.id
    k.agent = made.agent
  }
  const att = (await k.rpc({ AttachToSession: { session_id: k.sessionId, client_id: k.alias, capability_level: "FullTerminal" } })).SessionAttached
  k.attachment = att.attachment
  await k.client.subscribeToKernelEvents(k.sessionId, att.attachment.id)
  // The owner creates the Vault and opens it with the drill's passphrase (the owner's own client, no Room involved).
  await k.rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase: passkey } } })
  await k.rpc({ UnlockCredentialVault: { passphrase: passkey, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  note("vault-unlocked", { kernel: k.name })
  // The owner's terminal approves every access grant.
  k.client.onKernelEvent(async event => {
    if (event.event !== "passkey_prompts_changed") return
    for (const p of event.prompts || []) if (p.kind === "access_grant") {
      note("approve-grant", { kernel: k.name })
      await k.rpc({ RespondToInteraction: { session_id: p.session_id || "kernel-access", interaction_id: p.interaction_id, choice_id: "approve", passkey } }).catch(() => note("approval-failed"))
    }
  })
}
async function stop(k) {
  k.client?.close?.()
  const exited = new Promise(done => k.child.once("exit", done))
  stopOwned(k.kernel, "SIGTERM")
  await Promise.race([exited, delay(20000)])
  stopOwned(k.kernel)
  note("kernel-stop", { kernel: k.name })
}

// A workflow whose single node is an exit: the dev-stub provider submits its output as soon as it is invoked.

const holderDir = join(dir, "holder"); mkdirSync(holderDir)
const home = await kernelHome("home")
await start(home)
const sessionsBefore = (await home.rpc({ ListSessions: {} })).SessionsListed.sessions.length
note("sessions-before", { sessionsBefore })
const script = join(holderDir, "holder.sh")
writeFileSync(script, `#!/bin/bash
cd "$(dirname "$0")"
run() { local name=$1; shift; "$@" > "$name.out" 2> "$name.err"; echo $? > "$name.rc"; }
run grant chariox access request --until-exit
run oneshot chariox exec -- sh -c 'echo OUT-ONE; pwd; exit 7'
run indir chariox exec --workspace /tmp -- pwd
run open1 chariox terminal open --title first
run open2 chariox terminal open --title second
id1=$(node -e 'console.log(JSON.parse(require("fs").readFileSync("open1.out","utf8")).terminal.terminal_id)')
id2=$(node -e 'console.log(JSON.parse(require("fs").readFileSync("open2.out","utf8")).terminal.terminal_id)')
run send chariox terminal send --terminal $id1 --wait-ms 2500 'echo TYPED-$((6*7))'
run list chariox terminal list
run close chariox terminal close --terminal $id2
run list2 chariox terminal list
run closeall chariox terminal close --terminal $id1
touch done
sleep 600
`, { mode: 0o755 })
const child = spawn("bash", ["-c", `bash ${script}; true`], { env: home.env, stdio: "ignore" })
const holderHandle = own(child)
note("holder-start", { pid: child.pid })
const file = f => (existsSync(join(holderDir, f)) ? readFileSync(join(holderDir, f), "utf8") : "")
try {
  await until(() => existsSync(join(holderDir, "done")), "the holder finished", 180000)
  for (const name of ["grant", "oneshot", "indir", "open1", "open2", "send", "list", "close", "list2", "closeall"]) note(`holder-${name}`, { rc: file(`${name}.rc`).trim(), out: file(`${name}.out`).slice(0, 600), err: file(`${name}.err`).slice(0, 300) })
  assert.equal(file("grant.rc").trim(), "0", "the grant is approved")
  assert.equal(file("oneshot.rc").trim(), "7", "the one-shot returns the program's exit code")
  assert.match(file("oneshot.out"), /OUT-ONE/)
  assert.ok(file("oneshot.out").includes(userInfo().homedir), "it starts in the account home folder")
  assert.match(file("indir.out"), /\/tmp/)
  const typed = JSON.parse(file("send.out"))
  assert.match(typed.text, /TYPED-42/, "typed input is answered")
  const listed = JSON.parse(file("list.out")).terminals
  assert.deepEqual(listed.map(t => t.scope), ["kernel", "kernel"])
  assert.equal(JSON.parse(file("list2.out")).terminals.length, 1, "close ends one")
  const sessionsAfter = (await home.rpc({ ListSessions: {} })).SessionsListed.sessions.length
  assert.equal(sessionsAfter, sessionsBefore, "no Room was made")
  // A process outside the holder's tree has no grant.
  // Detached from this drill (and from the harness the grant names), so no ancestor of it holds a grant.
  const outsider = spawn("setsid", ["-f", "bash", "-c", `chariox exec -- uptime > ${join(holderDir, "outsider.out")} 2> ${join(holderDir, "outsider.err")}; echo $? > ${join(holderDir, "outsider.rc")}`], { env: home.env, stdio: "ignore" })
  outsider.unref()
  await until(() => existsSync(join(holderDir, "outsider.rc")), "the outsider answered", 60000)
  note("outsider", { rc: file("outsider.rc").trim(), out: file("outsider.out").slice(0, 300) })
  assert.equal(file("outsider.rc").trim(), "3")
  assert.match(file("outsider.out") + file("outsider.err"), /kernel_access_not_covered/)
  note("PASS")
} finally {
  for (const handle of owned.slice().reverse()) stopOwned(handle)
  await stop(home).catch(() => {})
}

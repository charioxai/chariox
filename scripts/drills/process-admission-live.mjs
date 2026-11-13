// MP-08/09/10/11 (offload OF-30/31/32/34/35): real Claude (and Codex) agents in a Room run commands that really wait
// for memory or disk on a real kernel whose pool is a small memory cgroup and whose disk is a small loop-mounted
// filesystem. The drill samples the one waiting row, an outside follower's `process` events (the real `chariox`
// command), the agents' own transcripts and, with --tui, the terminal client's screen.
// Disk cells (OF-37..OF-42): --scenario temp (every agent's TMPDIR is its own short folder inside the kernel's home),
// trim (idle scratch goes, what is open or busy stays, an ended Room clears at once), runaway (the orphaned writer filling the disk is attributed and
// stopped, other owners' builds are never paused).
// --reach 1 --relay-binary R: a second kernel reaches this one over a private relay and a follower on it also follows the Room (OF-32).
// --script-owners N [--script-command "python3 buildp1.py s{S} 1600 40 5"]: N outside scripts (own grant, `chariox exec`) join the --task round.
// Usage: node process-admission-live.mjs --scenario memory|disk|temp|room-isolation|trim|trim-live|disk-volumes|disk-waits|disk-replay|stop-cycles|runaway --kernel-binary K --state-root DIR --evidence-dir DIR
//   --login-file F [--agents 5] [--mem 7G] [--disk-gb 12] [--task a.txt:marker,b.txt:marker2] [--tui 1]
//   [--codex-agents 5 --codex-profile-dir DIR] [--work-dir DIR] [--seed-peaks FILE]
//   [--opencode-agents 3 --opencode-profile-dir DIR --opencode-model M] [--temp-idle-s 20]
//   --tui-input 1: submit prompts and focus agents through the actual TUI keyboard; requires --tui 1.
//   --scenario ui --ui-handoff FILE: the kernel, Room and agents are left to a browser drill (it reads FILE, sends the
//   prompts itself and creates FILE.done when it is finished).
// Run it under `unshare -m --propagation private` (the loop mount must not leak into other mount namespaces).
// State and evidence belong outside the checkout. Every process it starts is signalled by exact pid and start time.
import assert from "node:assert/strict"
import { execFileSync, spawn } from "node:child_process"
import { createHash, randomBytes } from "node:crypto"
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { dirname, join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { runDiskCells } from "./process-admission-disk-cells.mjs"
import { startReach } from "./process-admission-reach.mjs"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { assert.equal(typeof flags.get(key), "string", `${key} is required`); return flags.get(key) }
const scenario = need("--scenario")
assert(["memory", "disk", "temp", "room-isolation", "trim", "trim-live", "disk-volumes", "disk-waits", "disk-replay", "stop-cycles", "runaway", "ui"].includes(scenario), "unknown scenario")
const kernelBinary = resolve(need("--kernel-binary")), stateRoot = resolve(need("--state-root")), evidence = resolve(need("--evidence-dir"))
const loginFile = resolve(need("--login-file"))
const builds = Number(flags.get("--builds") ?? 5), memoryLimit = flags.get("--mem") ?? "7G"
const boundrun = flags.get("--boundrun") ?? "/root/.chariox/dev/lanes-miguel-r28/agents/boundrun.sh"
assert(stateRoot.startsWith("/root/.chariox/dev/"), "state belongs under /root/.chariox/dev")
assert(!evidence.startsWith(fileURLToPath(new URL("../../", import.meta.url))), "evidence must be outside the repository")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
const dir = mkdtempSync(join(stateRoot, `${scenario}-`))
const timeline = join(evidence, "timeline.jsonl")
const t0 = Date.now()
const note = (name, data = {}) => {
  const row = { t: ((Date.now() - t0) / 1000).toFixed(2), at: new Date().toISOString(), name, ...data }
  appendFileSync(timeline, JSON.stringify(row) + "\n")
  console.log(name, JSON.stringify(data).slice(0, 300))
}
const until = async (check, what, timeout = 30000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) { const v = await check(); if (v) return v; await delay(250) }
  throw new Error(`timeout: ${what}`)
}
const freePort = () => new Promise(done => { const s = createServer().listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => done(p)) }) })
const startTime = pid => { try { const stat = readFileSync(`/proc/${pid}/stat`, "utf8"); return stat.slice(stat.lastIndexOf(")") + 2).split(" ")[19] } catch { return null } }
const owned = []
const own = child => { owned.push({ pid: child.pid, start: startTime(child.pid) }); return child }
const parentOf = pid => { try { const stat = readFileSync(`/proc/${pid}/stat`, "utf8"); return Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]) } catch { return null } }
const descendants = pid => readdirSync("/proc").filter(name => /^\d+$/.test(name) && parentOf(Number(name)) === pid).flatMap(name => [...descendants(Number(name)), Number(name)])
// A process started by this drill is identified by pid and start time; its descendants go first.
const stopOwned = () => {
  for (const { pid, start } of owned) {
    if (!(pid > 1) || startTime(pid) !== start) continue
    for (const child of descendants(pid)) if (child > 1) { try { process.kill(child, "SIGTERM") } catch { /* gone */ } }
    try { process.kill(pid, "SIGTERM") } catch { /* gone */ }
  }
}

const ports = await Promise.all([freePort(), freePort(), freePort(), freePort()])
for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
const work = flags.has("--work-dir") ? resolve(flags.get("--work-dir")) : join(dir, "work")
mkdirSync(work, { recursive: true })
// Disk pressure only inside a small loop-mounted filesystem (the floor is max(5 %, 10 GiB) of the volume, so
// 11 GiB leaves about half a GiB above it); nothing else on the machine is written to.
const diskGb = Number(flags.get("--disk-gb") ?? 0)
const image = join(dir, "volume.img")
let loopDevice = null
const unmountVolume = () => {
  if (loopDevice) {
    try { execFileSync("umount", [work]) } catch { try { execFileSync("umount", ["-l", work]) } catch { /* not mounted */ } }
    try { execFileSync("losetup", ["-d", loopDevice]) } catch { /* detached */ }
    loopDevice = null
  }
  rmSync(image, { force: true })
}
const alias = "admission-drill"
const passkey = randomBytes(24).toString("hex")
writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
const cliWrapper = flags.get("--cli-wrapper") ?? join(dirname(fileURLToPath(import.meta.url)), "../../apps/cli/dist/index.js")
mkdirSync(join(dir, "bin"))
writeFileSync(join(dir, "bin/chariox"), `#!/bin/sh\nexec bun "${resolve(cliWrapper)}" "$@"\n`, { mode: 0o755 })
const kernelEnv = { PATH: `${join(dir, "bin")}:${process.env.PATH}`, LANG: "C.UTF-8", NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", SHELL: "/bin/bash",
  HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
  CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
  CHARIOX_DAEMON_SOCKET: join(dir, "daemon.sock"), CHARIOX_DAEMON_ALIAS: alias, CHARIOX_MACHINE_ALIAS: alias,
  ...(flags.has("--temp-idle-s") ? { CHARIOX_TEMP_IDLE_SECONDS: flags.get("--temp-idle-s") } : {}),
  CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
}
const report = { scenario, binary_sha256: createHash("sha256").update(readFileSync(kernelBinary)).digest("hex"), memory_limit: memoryLimit, builds, started_at: new Date().toISOString() }
const save = () => writeFileSync(join(evidence, "result.json"), JSON.stringify(report, null, 2) + "\n")

// Fixtures. Each dies with its parent (PR_SET_PDEATHSIG) and stays small: a build grows to `mb` MiB in 50 MiB steps,
// a writer writes `mb` MiB in 4 MiB chunks into its own file.
const fixtures = {
  "build.py": `import ctypes, os, sys, time
ctypes.CDLL("libc.so.6").prctl(1, 9)
name, mb, rate, hold = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), float(sys.argv[4])
held = []
for _ in range(mb // 50):
    held.append(bytearray(b"x") * (50 << 20))
    time.sleep(50 / rate)
time.sleep(hold)
open("done-" + name, "w").write("ok")
`,
  "write.py": `import ctypes, os, sys, time
ctypes.CDLL("libc.so.6").prctl(1, 9)
name, mb, rate = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
chunk = os.urandom(4 << 20)
with open("out-" + name, "wb") as out:
    for _ in range(mb // 4):
        out.write(chunk); out.flush(); os.fsync(out.fileno())
        time.sleep(4 / rate)
open("done-" + name, "w").write("ok")
`,
}
// The orphan: it opens its file, deletes it and keeps writing, so no directory listing shows what it takes. It is
// capped (`cap_mb`) and stops at once when STOP appears: a leaked one cannot fill anything but the loop filesystem.
fixtures["hog.py"] = `import os, sys, time
name, rate_mb, cap_mb = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
path = "hogfile-" + name
out = open(path, "wb")
os.unlink(path)
chunk = os.urandom(4 << 20)
written = 0
while written < cap_mb and not os.path.exists("STOP"):
    out.write(chunk); out.flush(); os.fsync(out.fileno())
    written += 4
    time.sleep(4 / rate_mb)
`
// The same build under other names: a command's remembered peak belongs to its name, so a copy starts unknown.
for (const copy of ["buildp1.py", "buildp2.py"]) fixtures[copy] = fixtures["build.py"]

// Everything the cleanup touches is declared here, before anything is allocated: a setup step that fails still
// leaves no kernel, loop volume or cgroup behind and still writes the report.
// The pool is the memory limit of the cgroup that holds the kernel's own: the kernel makes its workload budget
// beside its own cgroup, so the limit sits on the parent (a systemd scope would bound only the kernel itself).
const poolCgroup = `/sys/fs/cgroup/kr06pool-${process.pid}`
const gib = text => { const m = /^(\d+)([MG])$/.exec(text); assert(m, "--mem is like 7G"); return Number(m[1]) * (m[2] === "G" ? 1024 ** 3 : 1024 ** 2) }
const makePool = mem => {
  mkdirSync(poolCgroup)
  writeFileSync(join(poolCgroup, "memory.max"), String(gib(mem)))
  writeFileSync(join(poolCgroup, "memory.swap.max"), "0")
  writeFileSync(join(poolCgroup, "cgroup.subtree_control"), "+memory")
  mkdirSync(join(poolCgroup, "kernel"))
}
const removeCgroups = path => {
  if (!path.startsWith("/sys/fs/cgroup/kr06pool-")) return
  for (const e of readdirSync(path, { withFileTypes: true })) if (e.isDirectory()) removeCgroups(join(path, e.name))
  try { rmSync(path, { recursive: false }) } catch { try { execFileSync("rmdir", [path]) } catch { /* still has processes */ } }
}
const kernelEvents = []
let pump
let sampling = true
let following = true
const holderDir = join(dir, "holder")
const reachHolderDir = join(dir, "holder-reach")
const readFollowed = (from = holderDir) => existsSync(join(from, "records.log")) ? readFileSync(join(from, "records.log"), "utf8").split("\n").filter(Boolean).map(line => { const i = line.indexOf(" "); return { received_ms: Number(line.slice(0, i)), wait: JSON.parse(line.slice(i + 1)) } }) : []
const tmuxName = `admission-${process.pid}`
let tui = false
const tuiScreens = join(evidence, "tui")
const tmux = (...args) => execFileSync("tmux", args, { encoding: "utf8" })
const readlinkSafe = p => { try { return execFileSync("readlink", [p], { encoding: "utf8" }).trim() } catch { return null } }
// Processes of this drill's folder whose command line names `name` (the fixtures; used to find and end leftovers).
const osProcesses = name => readdirSync("/proc").filter(p => /^\d+$/.test(p)).flatMap(p => {
  try {
    const cmd = readFileSync(`/proc/${p}/cmdline`, "utf8").split("\0")
    return cmd.includes(name) && readlinkSafe(`/proc/${p}/cwd`) === work ? [{ pid: Number(p), cmd }] : []
  } catch { return [] }
})
const osTrace = []
const transcripts = () => {
  const root = join(dir, "home/.claude/projects")
  const out = []
  const walk = d => { if (!existsSync(d)) return; for (const e of readdirSync(d, { withFileTypes: true })) e.isDirectory() ? walk(join(d, e.name)) : e.name.endsWith(".jsonl") && out.push(join(d, e.name)) }
  walk(root)
  return out.map(f => readFileSync(f, "utf8")).join("\n")
}

const runCleanup = async () => {
  sampling = false; following = false; clearInterval(pump)
  // A fixture that outlived its agent (the orphan writers) is asked to stop, then ended by exact pid.
  try { writeFileSync(join(work, "STOP"), "") } catch { /* the folder is gone */ }
  await delay(1500)
  const fixtureNames = [...Object.keys(fixtures), "occupancy.py"]
  for (const name of fixtureNames) for (const { pid } of osProcesses(name)) if (pid > 1) { try { process.kill(pid, "SIGKILL") } catch { /* gone */ } }
  writeFileSync(join(evidence, "os-trace.json"), JSON.stringify(osTrace))
  writeFileSync(join(evidence, "followed.json"), JSON.stringify(readFollowed(), null, 1))
  if (existsSync(reachHolderDir)) writeFileSync(join(evidence, "followed-reach.json"), JSON.stringify(readFollowed(reachHolderDir), null, 1))
  for (const [label, from] of [["holder", holderDir], ["reach", reachHolderDir]]) for (const f of ["grant.err", "subscribe.err", "wait.err", "loop.log"]) if (existsSync(join(from, f))) appendFileSync(join(evidence, "holder-logs.txt"), `== ${label} ${f}\n${readFileSync(join(from, f), "utf8")}`)
  writeFileSync(join(evidence, "transcript.jsonl"), transcripts())
  if (tui) { try { writeFileSync(join(tuiScreens, "final.txt"), tmux("capture-pane", "-p", "-t", tmuxName)); tmux("kill-session", "-t", tmuxName) } catch { /* gone */ } }
  stopOwned()
  // Startup probes can finish spawning after the kernel exits. Drain only
  // this run's cgroup, with PID identity and its isolated home checked again.
  const members = path => existsSync(path) ? [
    ...readFileSync(join(path, "cgroup.procs"), "utf8").trim().split(/\s+/).filter(Boolean).map(Number),
    ...readdirSync(path, { withFileTypes: true }).filter(entry => entry.isDirectory()).flatMap(entry => members(join(path, entry.name))),
  ] : []
  report.late_processes_stopped = []
  for (let attempt = 0; attempt < 10; attempt++) {
    await delay(250)
    const pids = [...new Set(members(poolCgroup))]
    if (!pids.length) break
    for (const pid of pids) {
      if (!Number.isInteger(pid) || pid <= 1) throw new Error("invalid owned cgroup PID")
      const start = startTime(pid)
      if (!start) continue
      let homeMatches = false
      try {
        homeMatches = readFileSync(`/proc/${pid}/environ`, "utf8").split("\0").includes(`CHARIOX_HOME=${kernelEnv.CHARIOX_HOME}`)
          || readlinkSafe(`/proc/${pid}/cwd`)?.startsWith(dir + "/")
      } catch { continue }
      if (startTime(pid) !== start || !members(poolCgroup).includes(pid)) continue
      if (!homeMatches) {
        // Exiting processes lose environ/cwd before their final /proc entry.
        try {
          const stat = readFileSync(`/proc/${pid}/stat`, "utf8")
          if (["Z", "X"].includes(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[0])) continue
        } catch { continue }
      }
      if (startTime(pid) !== start || !members(poolCgroup).includes(pid)) continue
      assert(homeMatches, `unrecognized process in this drill's cgroup: ${pid}`)
      if (startTime(pid) !== start || !members(poolCgroup).includes(pid)) continue
      const signal = attempt < 5 ? "SIGTERM" : "SIGKILL"
      try { process.kill(pid, signal); report.late_processes_stopped.push({ pid, start, signal }) } catch { /* gone */ }
    }
  }
  report.remaining_fixture_processes = fixtureNames.flatMap(name => osProcesses(name).map(row => row.pid))
  if (report.remaining_fixture_processes.length) report.error = "Owned fixture processes remain after cleanup"
  note("cleanup-census", {remaining_fixture_processes: report.remaining_fixture_processes})
  for (let attempt = 0; attempt < 10 && existsSync(poolCgroup); attempt++) { removeCgroups(poolCgroup); if (existsSync(poolCgroup)) await delay(1000) }
  report.remaining_pool_processes = members(poolCgroup)
  if (report.remaining_pool_processes.length) {
    report.error = "Owned processes remain after cleanup; preserving the volume for exact cleanup"
  } else {
    unmountVolume()
  }
  for (const f of readdirSync(join(dir, "logs"))) appendFileSync(join(evidence, "kernel-logs.txt"), readFileSync(join(dir, "logs", f), "utf8"))
  save()
}

let cleaning
const cleanup = () => cleaning ??= runCleanup()

process.once("unhandledRejection", async error => { report.error = `unhandled: ${String(error)}`; await cleanup(); process.exit(1) })
for (const signal of ["SIGTERM", "SIGINT"]) process.once(signal, async () => { report.error = `stopped by ${signal}`; await cleanup(); process.exit(2) })
try {
  if (diskGb) {
    assert(work.startsWith("/root/.chariox/dev/kr06/"), "the volume is mounted under the lane's own state")
    execFileSync("truncate", ["-s", `${diskGb}G`, image])
    execFileSync("mkfs.ext4", ["-q", "-m", "0", "-F", image])
    loopDevice = execFileSync("losetup", ["-f", "--show", image], { encoding: "utf8" }).trim()
    execFileSync("mount", [loopDevice, work])
    const [total, free] = execFileSync("df", ["-B1", "--output=size,avail", work], { encoding: "utf8" }).trim().split("\n")[1].trim().split(/\s+/).map(Number)
    console.log("volume", loopDevice, JSON.stringify({ total, free, floor: Math.max(total / 20, 10 * 1024 ** 3) }))
  }
  for (const [name, text] of Object.entries(fixtures)) writeFileSync(join(work, name), text)

  // A command's peak is remembered by its argv and folder: a second run on a smaller machine starts from the first one's.
  if (flags.has("--seed-peaks")) {
    mkdirSync(join(dir, "chariox/state"), { recursive: true })
    writeFileSync(join(dir, "chariox/state/admission-peaks.json"), readFileSync(resolve(flags.get("--seed-peaks"))), { mode: 0o600 })
  }
  const logFd = openSync(join(evidence, "kernel.log"), "w", 0o600)

  let kernel
  const startKernel = (mem = memoryLimit) => {
    makePool(mem)
    kernel = own(spawn("sh", ["-c", `echo $$ > ${poolCgroup}/kernel/cgroup.procs; echo 0 > /proc/self/oom_score_adj; exec "$0"`, kernelBinary], { env: kernelEnv, cwd: dir, stdio: ["ignore", logFd, logFd] }))
    note("kernel-start", { pid: kernel.pid, mem, pool: poolCgroup })
  }
  startKernel()
  const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, { timeoutMs: 15000, localAuthEnvironment: kernelEnv, controlRequestRetryDeadlineMs: 1000 })
  const rpc = async request => { const r = await client.send(request); if (r.Error) throw new Error(`refused ${Object.keys(request)[0]}: ${JSON.stringify(r.Error)}`); return r }
  await until(async () => { try { return !!(await rpc({ RelayStatus: null })).RelayStatus } catch { return false } }, "kernel ready", Number(flags.get("--ready-timeout-ms") ?? 60000))
  report.kernel_pid = kernel.pid
  report.pool_cgroup = readFileSync(`/proc/${kernel.pid}/cgroup`, "utf8").trim()
  note("kernel-ready", { cgroup: report.pool_cgroup })

  const made = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias } })).SessionCreated
  const sessionId = made.session.id
  const sibling = scenario === "room-isolation" ? (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: `${alias}-sibling` } })).SessionCreated : null
  if (sibling) {
    assert.equal(sibling.session.id.slice(0, 8), sessionId.slice(0, 8), "the Rooms must share the old temp-path prefix; rerun this boundary-crossing attempt")
    report.sibling_room = sibling.session.id
    report.room = sessionId
  }
  const att = (await rpc({ AttachToSession: { session_id: sessionId, client_id: alias, capability_level: "FullTerminal" } })).SessionAttached
  await client.subscribeToKernelEvents(sessionId, att.attachment.id)
  const unlock = rpc({ ManageCredentialVault: { session_id: sessionId, agent_id: made.agent.id } }); unlock.catch(() => {})
  const vaultPrompt = await until(async () => (await rpc({ GetSessionState: { session_id: sessionId } })).SessionState.session.active_interactions?.find(x => x.id.startsWith("vault-unlock-")), "vault prompt")
  await rpc({ RespondToInteraction: { session_id: sessionId, interaction_id: vaultPrompt.id, choice_id: "passphrase", custom_reply: passkey } })
  await unlock
  note("vault-unlocked")
  client.onKernelEvent(async event => {
    if (event.event === "passkey_prompts_changed") {
      for (const p of event.prompts || []) if (p.kind === "access_grant") {
        note("approve-grant", { requester: p.requester?.executable })
        await rpc({ RespondToInteraction: { session_id: p.session_id || "kernel-access", interaction_id: p.interaction_id, choice_id: "approve", passkey } }).catch(() => note("approval-failed"))
      }
    }
    if (event.event === "runtime_notices") for (const n of event.notices || []) {
      kernelEvents.push(n); note("runtime-notice", { message: n.message, level: n.level })
    }
  })
  // An attachment that stays quiet is reaped: keep it alive.
  pump = setInterval(() => { rpc({ PumpTerminalOutput: { session_id: sessionId, attachment_id: att.attachment.id } }).catch(() => {}) }, 2000)
  const claudeProfile = (await rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: readFileSync(loginFile, "utf8").trim(), overwrite: true } })).ProviderAccountCredentialStored.account_profile
  note("login-stored")
  const agentCount = Number(flags.get("--agents") ?? 1)
  // --codex-agents 5,4: those agent numbers run Codex (a linked profile, --codex-profile-dir) instead of Claude;
  // --opencode-agents 3 the same for OpenCode (--opencode-profile-dir, --opencode-model).
  const codexNumbers = (flags.get("--codex-agents") ?? "").split(",").filter(Boolean).map(Number)
  const opencodeNumbers = (flags.get("--opencode-agents") ?? "").split(",").filter(Boolean).map(Number)
  if (codexNumbers.length) {
    await rpc({ LinkProviderAccountProfile: { provider: "codex", label: "drill-codex", path: resolve(need("--codex-profile-dir")) } })
    note("codex-linked")
  }
  if (opencodeNumbers.length) {
    const linked = (await rpc({ LinkProviderAccountProfile: { provider: "opencode", label: "drill-opencode", path: resolve(need("--opencode-profile-dir")) } })).ProviderAccountProfile.profile
    await rpc({ RefreshProviderAccountProfile: { provider: "opencode", account_profile: linked.profile_id } })
    note("opencode-linked", { profile: linked.profile_id })
  }
  const opencodeProfile = opencodeNumbers.length ? (await rpc({ ListProviderAccountProfiles: { provider: "opencode" } })).ProviderAccountProfilesListed.profiles.find(p => p.label === "drill-opencode").profile_id : null
  const providerOf = n => codexNumbers.includes(n) ? "codex" : opencodeNumbers.includes(n) ? "opencode" : "claude"
  const agents = []
  for (let n = 1; n <= agentCount; n++) {
    const provider = providerOf(n)
    const choice = provider === "codex" ? { account_profile: "drill-codex", model: "gpt-6-luna", effort: "low" }
      : provider === "opencode" ? { account_profile: opencodeProfile, model: flags.get("--opencode-model") ?? "opencode-go/deepseek-v4.1-flash" }
      : { account_profile: claudeProfile, model: "claude-haiku-5-5" }
    const spawned = (await rpc({ SpawnAgent: { session_id: sessionId, provider, ...choice, permission_level: "yolo", alias: `builder${n}` } })).AgentSpawned.agent
    note("agent", { n, provider, id: spawned.id, model: choice.model })
    agents.push(spawned)
  }
  const agent = agents[0]
  report.agent_ids = agents.map(a => a.id)
  let tuiInputQueue = Promise.resolve()
  const ask = async (text, target = agent) => {
    if (flags.get("--tui-input") === "1") {
      const pending = tuiInputQueue.then(async () => {
        assert(tui, "--tui-input requires --tui")
        await focusInTui(target)
        execFileSync("tmux", ["load-buffer", "-b", tmuxName, "-"], { input: text })
        tmux("paste-buffer", "-p", "-d", "-b", tmuxName, "-t", tmuxName)
        const tail = text.replace(/\s/g, "").slice(-80)
        await until(() => tuiEditor(target).replace(/\s/g, "").includes(tail), "TUI paste to reach the editor")
        await delay(500)
        assert(tuiEditor(target).replace(/\s/g, "").includes(tail), "the TUI draft changed before Enter")
        tmux("send-keys", "-t", tmuxName, "Enter")
        await until(() => tuiEditor(target).includes("Write your next prompt here"), `TUI submitted draft for ${target.alias}`)
        note("tui-prompt", { agent: target.id, text: text.slice(0, 120) })
      })
      tuiInputQueue = pending
      return pending
    }
    await rpc({ SubmitPrompt: { session_id: sessionId, attachment_id: att.attachment.id, target_agent_id: target.id, prompt: text, attachments: [] } })
    note("prompt", { agent: target.id, text: text.slice(0, 120) })
  }

  // The follower: an outside holder (its own process tree, the `chariox` command) that gets a grant, follows the
  // agents and logs every record with the time it received it.
  // `reach` follows through a second kernel (--reach 1): its holder runs against that kernel and names this one with --kernel.
  const startHolder = (from, env, kernelName) => {
    mkdirSync(from)
    writeFileSync(join(from, "holder.sh"), `#!/bin/bash
cd "$(dirname "$0")"
chariox access request --until-exit > grant.json 2> grant.err || exit 2
chariox events subscribe --kernel ${kernelName} --session ${sessionId} > subscribe.json 2> subscribe.err || exit 3
echo subscribed > ready
cursor=0
while :; do
  chariox events wait --since $cursor --timeout 20 > wait.json 2> wait.err; rc=$?
  received=$(date +%s%3N)
  [ $rc -eq 5 ] && continue
  [ $rc -ne 0 ] && { echo "wait exit $rc" >> loop.log; sleep 1; continue; }
  printf '%s %s\\n' "$received" "$(tr -d '\\n' < wait.json)" >> records.log
  cursor=$(node -e 'const r=JSON.parse(require("fs").readFileSync("wait.json","utf8"));console.log(r.next_cursor)')
  chariox events ack $cursor > /dev/null 2>&1
done
`, { mode: 0o755 })
    const child = own(spawn("bash", ["-c", `bash ${join(from, "holder.sh")}; true`], { env, stdio: "ignore" }))
    note("holder-start", { pid: child.pid, kernel: kernelName })
    return until(() => existsSync(join(from, "ready")), `follower ${kernelName} subscribed`, 120000).then(() => note("follower-ready", { kernel: kernelName })).catch(e => note("follower-failed", { kernel: kernelName, error: String(e) }))
  }
  const reach = flags.get("--reach") === "1" ? await startReach({ relayBinary: resolve(need("--relay-binary")), kernelBinary, dir, evidence, drillEnv: kernelEnv, drillRpc: rpc, passkey, own, until, note }) : null
  const followed = []
  const runScriptOwner = (n, template) => new Promise(done => {
    const scriptDir = join(dir, `script-owner-${n}`)
    mkdirSync(scriptDir)
    const argv = template.replaceAll("{S}", String(n)).split(" ")
    writeFileSync(join(scriptDir, "run.sh"), `#!/bin/bash
cd ${JSON.stringify(work)}
chariox access request --until-exit > ${scriptDir}/grant.json 2> ${scriptDir}/grant.err || exit 2
chariox exec --kernel ${alias} --session ${sessionId} -- ${argv.map(a => JSON.stringify(a)).join(" ")} > ${scriptDir}/out.txt 2> ${scriptDir}/err.txt
echo $? > ${scriptDir}/rc
`, { mode: 0o755 })
    const child = own(spawn("bash", [join(scriptDir, "run.sh")], { env: kernelEnv, stdio: "ignore" }))
    note("script-owner-start", { n, pid: child.pid, command: argv.join(" ") })
    const started = Date.now()
    child.once("close", () => {
      const read = f => existsSync(join(scriptDir, f)) ? readFileSync(join(scriptDir, f), "utf8") : null
      const row = { n, exit_code: Number(read("rc")), seconds: (Date.now() - started) / 1000, stderr: read("err.txt")?.slice(0, 2000), stdout: read("out.txt")?.slice(0, 500) }
      note("script-owner-done", row)
      done(row)
    })
  })
  const followerLoop = Promise.all([startHolder(holderDir, kernelEnv, alias), ...(reach ? [startHolder(reachHolderDir, reach.env, reach.target)] : [])])

  // --tui: the real terminal client attached to the Room in tmux; every changed screen is kept.
  if (flags.has("--tui")) {
    mkdirSync(tuiScreens, { recursive: true })
    tmux("new-session", "-d", "-s", tmuxName, "-x", flags.get("--tui-cols") ?? "200", "-y", flags.get("--tui-rows") ?? "50",
      `env -i 'PATH=${kernelEnv.PATH}' 'HOME=${kernelEnv.HOME}' 'TERM=xterm-256color' 'CHARIOX_HOME=${kernelEnv.CHARIOX_HOME}' 'CHARIOX_DAEMON_SOCKET=${kernelEnv.CHARIOX_DAEMON_SOCKET}' 'XDG_CONFIG_HOME=${kernelEnv.XDG_CONFIG_HOME}' bun ${resolve(cliWrapper)} --kernel-url ws://127.0.0.1:${ports[0]}/kernel --session ${sessionId} --client-id tui-drill`)
    tui = true
    note("tui-start", { tmux: tmuxName })
  }
  const tuiEditor = target => {
    const lines = tmux("capture-pane", "-p", "-t", tmuxName).split("\n")
    const status = lines.findLastIndex(line => line.includes(`${target.alias} •`))
    return status < 0 ? "" : lines.slice(status + 1).join("\n")
  }
  const focusInTui = async target => {
    await until(() => tmux("capture-pane", "-p", "-t", tmuxName).includes("Write your next prompt here"), "TUI prompt editor", 60000)
    tmux("send-keys", "-t", tmuxName, "-l", `/agent focus ${target.alias}`)
    tmux("send-keys", "-t", tmuxName, "Enter")
    await until(async () => {
      const session = (await rpc({ GetSessionState: { session_id: sessionId } })).SessionState.session
      return session.focused_agent_id === target.id && tuiEditor(target).includes("Write your next prompt here")
    }, `TUI focus ${target.alias}`, 60000)
    writeFileSync(join(tuiScreens, `focus-${target.alias}-${Date.now()}.txt`), tmux("capture-pane", "-p", "-t", tmuxName))
  }
  let tuiShots = 0, lastScreen = ""
  const watchTui = async () => {
    while (tui && sampling) {
      try {
        const screen = tmux("capture-pane", "-p", "-t", tmuxName)
        if (screen !== lastScreen && /in line|waiting for|paused/i.test(screen)) { lastScreen = screen; writeFileSync(join(tuiScreens, `${String(++tuiShots).padStart(3, "0")}-${((Date.now() - t0) / 1000).toFixed(0)}s.txt`), screen) }
      } catch { /* the pane is gone */ }
      await delay(1000)
    }
  }

  // The one waiting row, sampled twice a second, and the same processes as the OS sees them.
  const rows = new Map()
  let latestRows = []
  const sample = async () => {
    while (sampling) {
      try {
        const state = (await rpc({ GetSessionState: { session_id: sessionId } })).SessionState
        latestRows = state.processes ?? []
        for (const p of state.processes ?? []) {
          const key = String(p.pid ?? p.process_id)
          const view = JSON.stringify({ kind: p.state?.kind, position: p.state?.position, reason: p.state?.reason })
          if (rows.get(key) !== view) { rows.set(key, view); note("row", { pid: p.pid, process_id: p.process_id, cmd: (p.state?.command_line ?? p.command ?? "").slice(0, 80), ...JSON.parse(view), need_mb: p.state?.need_bytes && Math.round(p.state.need_bytes / 1048576), free_mb: p.state?.free_bytes && Math.round(p.state.free_bytes / 1048576), since_ms: p.state?.since_ms }) }
        }
      } catch (e) { note("sample-error", { error: String(e).slice(0, 160) }) }
      await delay(500)
    }
  }
  const sampler = sample()
  const tuiWatch = tui ? watchTui() : null
  const osState = () => {
    const out = []
    for (const name of readdirSync("/proc")) {
      if (!/^\d+$/.test(name)) continue
      try {
        const cmd = readFileSync(`/proc/${name}/cmdline`, "utf8").split("\0")
        if (!cmd.some(a => a === "build.py" || a === "write.py") || cmd[0] !== "python3") continue
        if (readlinkSafe(`/proc/${name}/cwd`) !== work) continue
        const stat = readFileSync(`/proc/${name}/stat`, "utf8")
        out.push({ pid: Number(name), cgroup: readFileSync(`/proc/${name}/cgroup`, "utf8").trim().replace("0::", ""), name: cmd[2], state: stat.slice(stat.lastIndexOf(")") + 2, stat.lastIndexOf(")") + 3), rss_mb: Math.round(Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[21]) * 4 / 1024) })
      } catch { /* gone */ }
    }
    return out
  }
  const osLoop = (async () => { while (sampling) { osTrace.push({ t: (Date.now() - t0) / 1000, procs: osState() }); await delay(1000) } })()

  await followerLoop
  if (scenario === "ui") {
    const handoff = resolve(need("--ui-handoff"))
    writeFileSync(handoff, JSON.stringify({ url: `ws://127.0.0.1:${ports[0]}/kernel`, kernelEnv, sessionId, alias, work, agents: agents.map(a => a.id), attachment_id: att.attachment.id }, null, 1))
    note("ui-handoff", { handoff })
    await until(() => existsSync(handoff + ".done"), "the browser drill to finish", Number(flags.get("--ui-wait-s") ?? 3600) * 1000)
  } else if (["temp", "room-isolation", "trim", "trim-live", "disk-volumes", "disk-waits", "disk-replay", "stop-cycles", "runaway"].includes(scenario)) {
    await runDiskCells({ scenario, agents, ask, note, until, work, dir, sessionId, siblingId: sibling?.session.id, attachmentId: att.attachment.id, rpc, report, flags, transcripts, providerOf, execFileSync, osProcesses, latestRows: () => latestRows })
  } else
  // --task a.txt:marker,b.txt:marker2: every agent gets the prompt with {N} = its number; the round ends when each
  // agent's transcript shows `<marker>_bN` (two words the prompt never spells joined by an underscore).
  for (const spec of (flags.get("--task") ?? "").split(",").filter(Boolean)) {
    const [file, marker] = spec.split(":")
    const started = Date.now()
    // A Codex agent gets <name>-codex.txt and ends by writing answer-bN.txt (its transcripts are not read).
    const answersInFile = n => providerOf(n) !== "claude"
    const templateFor = n => readFileSync(resolve(answersInFile(n) ? file.replace(/\.txt$/, "-codex.txt") : file), "utf8")
    // A script owner is an outside process with its own grant that runs the same build through `chariox exec`.
    const scripts = Array.from({ length: Number(flags.get("--script-owners") ?? 0) }, (_, i) => runScriptOwner(i + 1, flags.get("--script-command") ?? "python3 buildp1.py s{S} 1600 40 5"))
    await Promise.all(agents.map((target, i) => ask(templateFor(i + 1).replaceAll("{N}", String(i + 1)), target)))
    await Promise.all(agents.map((_, i) => until(() => answersInFile(i + 1) ? existsSync(join(work, `answer-b${i + 1}.txt`)) : transcripts().includes(`${marker}_b${i + 1}`), `agent ${i + 1} answer ${marker}`, Number(flags.get("--wait-s") ?? 900) * 1000)
      .then(() => note("answered", { agent: i + 1, marker, seconds: ((Date.now() - started) / 1000).toFixed(1) }))))
    report.script_owners = await Promise.all(scripts)
    report[`round_${marker}_s`] = (Date.now() - started) / 1000
  }
  report.peaks_file = join(dir, "chariox/state/admission-peaks.json")
  report.done_files = readdirSync(work).filter(n => n.startsWith("done-"))
} catch (e) {
  report.error = String(e)
  note("failed", { error: String(e) })
} finally {
  await delay(Number(flags.get("--settle-ms") ?? 3000))
  report.runtime_notices = kernelEvents
  await cleanup()
}
process.exit(report.error ? 1 : 0)

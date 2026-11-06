// MP-08 / MP-09 / MP-10 / MP-11 (ON-17, ON-18): an outside agent holding a real access grant follows a workflow
// exit through `chariox events sources` / `events subscribe --source` and is woken by it without polling.
// Real kernel (disposable home, vault and passkey), the real built CLI as the scripted holder, a workflow of the
// dev-stub provider that finishes through a notifying exit (the reviewer workflow's shape: it exits when a review
// arrives). No paid service and no provider login are used.
// Proves, in order:
//   follow     the grant lists its sources (the workflow exit and "Agent needs your action", which exists before any
//              task blocked), follows both, and one `events wait` returns exactly ONE `notification` record the moment
//              a run finishes: the wait started before the run and ended long before its timer, no list or repeated
//              request, the output text is the exit's, the deadline is 7 days;
//   twice      a second run is a second record, never a repeat of the first;
//   restart    an occurrence emitted while the holder is not reading survives a kernel restart and reaches the same
//              grant once, still following (the grant lasts until its holder exits and its holder lives);
//   expire     an occurrence unread past its deadline is dropped and the holder reads one `gap` (the kernel's debug
//              clock is moved 8 days; no sleeping);
//   revoke     after the owner revokes the grant its wait ends (exit 3), a run finishing later reaches nobody, and
//              the holder's next grant follows nothing;
//   refused    a source of ANOTHER kernel and an unknown id are refused by the kernel with the same sentence.
// Red on a base kernel without the arm (protocol 999 / S1 head): `events sources` is refused as an unknown request.
// `--scenario owner-action --provider claude --login-file F | --provider codex --codex-scope DIR` (AT-40, one real
// turn: Claude Haiku on the drill kernel's own Vault, or Codex gpt-6-luna low on a linked profile): a driven agent
// reports through its `chariox.events.blocked` tool that its task needs the owner; the holder, following "Agent needs
// your action" and not polling, is woken by one notification that names the task and the decision, and answers it.
// `--scenario workflow-runs` (ON-17, retiring a 60-s poll of workflow runs): the coordinator's replacement client
// scripts/reviewer/reviewer-wait-events.mjs, run in the holder's process tree against a workflow with webhook-style
// invocation prompts: a completed run and a failed run wake it within seconds with the right line, a burst of runs is
// printed once each and in order, a run that ended while the kernel restarted is not lost, a missing notification is
// covered by the fallback listing, and a stopped kernel or a revoked grant ends the wait with the unreachable line.
// Usage: node notif-external-agent.mjs --kernel-binary K --state-root DIR --evidence-dir DIR [--cli apps/cli/dist/index.js]
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { randomBytes } from "node:crypto"
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { createServer } from "node:net"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { addWorkflowNodeRequest, createWorkflowEndpointRequest, createWorkflowRequest, invokeWorkflowEndpointRequest, setWorkflowNodeCanCompleteRunRequest } from "../../packages/kernel-client/dist/ipc-workflow-requests.js"

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
const dir = mkdtempSync(join(stateRoot, "notif-"))
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
  const alias = `notif-${name}`
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
  // A restarted kernel keeps its vault unlocked or asks again; either way the drill gives its passphrase once.
  let unlocked = false
  const unlock = k.rpc({ ManageCredentialVault: { session_id: k.sessionId, agent_id: k.agent.id } }).then(() => { unlocked = true }, error => { note("vault-manage", { kernel: k.name, answer: String(error.message ?? error).slice(0, 160) }); unlocked = true })
  const prompt = await until(async () => unlocked ? "unlocked" : (await k.rpc({ GetSessionState: { session_id: k.sessionId } })).SessionState.session.active_interactions?.find(x => x.id.startsWith("vault-unlock-")), "vault prompt", 30000)
  if (prompt !== "unlocked") await k.rpc({ RespondToInteraction: { session_id: k.sessionId, interaction_id: prompt.id, choice_id: "passphrase", custom_reply: passkey } })
  await unlock
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
async function reviewerWorkflow(k, alias, agentId = k.agent.id) {
  const send = request => k.client.send(request)
  const variant = (r, name) => { assert(r[name], `${name}: ${JSON.stringify(r).slice(0, 200)}`); return r[name] }
  const workflow = variant(await send(createWorkflowRequest(k.sessionId, alias)), "WorkflowCreated").workflow
  const node = variant(await send(addWorkflowNodeRequest(k.sessionId, workflow.id, agentId)), "WorkflowNodeAdded").node
  variant(await send(setWorkflowNodeCanCompleteRunRequest(k.sessionId, workflow.id, node.id, true)), "WorkflowNodeCanCompleteRunUpdated")
  const endpoint = variant(await send(createWorkflowEndpointRequest(k.sessionId, workflow.id, node.id, `${alias}-entry`)), "WorkflowEndpointCreated").endpoint
  const source = variant(await send({ RegisterWorkflowNotificationSource: { session_id: k.sessionId, workflow_ref: workflow.id, exit_node_id: node.id, enabled: true } }), "WorkflowNotificationSourceRegistered").source
  return { workflow, node, endpoint, source }
}
async function launchReviewer(k) {
  const launched = await k.rpc({ LaunchProviderRun: { session_id: k.sessionId, agent_id: k.agent.id, adapter_key: "dev-stub", provider: "dev-stub", account_profile: "default", model: "workflow-single-turn-node", variant: "low" } })
  const run = (launched.ProviderRunLaunchAccepted ?? launched.ProviderRunLaunched).provider_run
  await until(async () => (await k.rpc({ GetSessionState: { session_id: k.sessionId } })).SessionState.session.active_provider_run_id === run.id, "provider run active", 60000)
}
// Finishes one run of the workflow: a provider run, the invocation, and the run's terminal state.
async function finishRun(k, w, text) {
  await launchReviewer(k)
  const at = ms()
  const invoked = (await k.rpc(invokeWorkflowEndpointRequest(k.sessionId, w.workflow.id, w.endpoint.id, text))).WorkflowRunInvoked.workflow_run
  const done = await until(async () => {
    const run = (await k.rpc({ GetWorkflowRun: { session_id: k.sessionId, workflow_run_ref: invoked.id } })).WorkflowRun.workflow_run
    return ["Completed", "Failed", "Stopped"].includes(run.status) ? run : null
  }, `run ${invoked.id} finished`, 90000)
  const status = done.status
  assert.equal(status, "Completed", `run ${invoked.id} completed`)
  note("run-finished", { run: invoked.id, ms_after_invoke: ms() - at })
  return { id: invoked.id, invoked_at: at, finished_at: ms() }
}

// The scripted holder: its own process tree is the grant holder. Every `chariox` call is logged with start and end
// times so the drill can show what it asked and when. Control files steer it: `pause` stops reading, `phase2` lets it
// continue after its wait ended.
const holderDir = join(dir, "holder")
mkdirSync(holderDir)
function holder(k, sourceName) {
  const script = join(holderDir, "holder.sh")
  writeFileSync(script, `#!/bin/bash
cd "$(dirname "$0")"
log() { echo "$(date +%s%3N) $*" >> calls.log; }
run() { local name=$1; shift; log "start $name"; "$@" > "$name.out" 2> "$name.err"; local rc=$?; echo $rc > "$name.rc"; log "end $name rc=$rc"; return $rc; }
run grant chariox access request --until-exit || exit 2
run sources chariox events sources || exit 3
run follow-workflow chariox events subscribe --source ${JSON.stringify(sourceName)} || exit 4
run follow-owner-action chariox events subscribe --source "Agent needs your action" || exit 5
cursor=0; n=0
while :; do
  while [ -e pause ]; do sleep 0.2; done
  if [ -e check-subscription ]; then rm -f check-subscription; run subscription-check chariox events subscription; fi
  n=$((n+1))
  # A pause also ends the wait in flight (the holder's own child), so what arrives meanwhile stays unread.
  log "start wait-$n"; chariox events wait --since $cursor --timeout 100 > wait-$n.out 2> wait-$n.err & wpid=$!
  while kill -0 $wpid 2>/dev/null; do [ -e pause ] && kill $wpid 2>/dev/null; sleep 0.1; done
  wait $wpid; rc=$?; echo $rc > wait-$n.rc; log "end wait-$n rc=$rc"
  [ -e pause ] && [ $rc -ge 128 ] && continue
  [ $rc -eq 5 ] && continue
  if [ $rc -eq 3 ]; then cat wait-$n.out >> records.jsonl; echo >> records.jsonl; echo ended > ended; break; fi
  [ $rc -ne 0 ] && { echo "wait exit $rc" >> loop.log; sleep 1; continue; }
  cat wait-$n.out >> records.jsonl; echo >> records.jsonl
  cursor=$(node -e 'const r=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"));console.log(r.next_cursor)' wait-$n.out)
  run ack-$n chariox events ack $cursor
done
until [ -e phase2 ]; do sleep 0.2; done
run grant2 chariox access request --until-exit
run subscription2 chariox events subscription
run list2 chariox events list
run sources2 chariox events sources
echo '{"SubscribeEvents":{"targets":[],"sources":["'$(cat other-kernel-source)'"]}}' > other.json
run other-kernel-source bash -c 'chariox request < other.json'
echo '{"SubscribeEvents":{"targets":[],"sources":["wf_source_00000000000000000000000000000000"]}}' > unknown.json
run unknown-source bash -c 'chariox request < unknown.json'
touch phase2-done
sleep 600
`, { mode: 0o755 })
  // The kernel names the holder two levels above the CLI: the outer shell makes it this holder's own.
  const child = spawn("bash", ["-c", `bash ${script}; true`], { env: k.env, stdio: "ignore" })
  const handle = own(child)
  note("holder-start", { pid: child.pid })
  const file = f => (existsSync(join(holderDir, f)) ? readFileSync(join(holderDir, f), "utf8") : null)
  const json = f => { const text = file(f); return text ? JSON.parse(text) : null }
  const records = () => (file("records.jsonl") ?? "").split("\n").filter(Boolean).flatMap(l => JSON.parse(l).events ?? [])
  const calls = () => (file("calls.log") ?? "").split("\n").filter(Boolean).map(l => { const [t, kind, name, rc] = l.split(" "); return { t: Number(t), kind, name, rc: rc?.replace("rc=", "") } })
  return { handle, file, json, records, calls, pause: () => writeFileSync(join(holderDir, "pause"), ""), resume: () => rmSync(join(holderDir, "pause"), { force: true }), touch: f => writeFileSync(join(holderDir, f), "") }
}
const notifications = h => h.records().filter(e => e.type === "notification")

const results = {}
const scenario = flags.get("--scenario") ?? "main"
assert(["main", "owner-action", "workflow-runs"].includes(scenario), "unknown scenario")
const home = await kernelHome("home"), other = await kernelHome("other")

// AT-40: a real driven agent blocks its task; the holder is woken by the owner-action source and answers the decision.
async function ownerAction() {
  // Claude Haiku on a login stored in this kernel's Vault, or Codex gpt-6-luna (low) on a linked product profile.
  const provider = flags.get("--provider") ?? "claude"
  assert(["claude", "codex"].includes(provider), "--provider is claude or codex")
  await start(home)
  let agentDefaults
  if (provider === "claude") {
    const token = readFileSync(resolve(need("--login-file")), "utf8").trim()
    await home.rpc({ SetProviderAccountCredential: { provider: "claude", account_profile: "default", value: token, overwrite: true } })
    agentDefaults = { provider: "claude", model: "claude-haiku-5-5", permission_level: "yolo" }
  } else {
    const profile = (await home.rpc({ LinkProviderAccountProfile: { provider: "codex", label: "notif drill", path: resolve(need("--codex-scope")) } })).ProviderAccountProfile.profile.profile_id
    await home.rpc({ GetProviderAuthStatus: { provider: "codex", account_profile: profile } })
    agentDefaults = { provider: "codex", account_profile: profile, model: "gpt-6-luna", effort: "low", permission_level: "yolo" }
  }
  const driven = (await home.rpc({ SpawnAgent: { ...agentDefaults, session_id: home.sessionId, alias: "driven" } })).AgentSpawned.agent
  note("driven-agent", { provider, model: agentDefaults.model })
  const dir2 = join(dir, "holder-owner-action"); mkdirSync(dir2)
  const script = join(dir2, "holder.sh")
  writeFileSync(script, `#!/bin/bash
cd "$(dirname "$0")"
log() { echo "$(date +%s%3N) $*" >> calls.log; }
run() { local name=$1; shift; log "start $name"; "$@" > "$name.out" 2> "$name.err"; local rc=$?; echo $rc > "$name.rc"; log "end $name rc=$rc"; return $rc; }
run grant chariox access request --until-exit || exit 2
run sources chariox events sources || exit 3
run follow chariox events subscribe --source "Agent needs your action" || exit 4
touch ready
run wait-1 chariox events wait --timeout 280 || exit 5
node -e 'const e=JSON.parse(require("fs").readFileSync("wait-1.out","utf8")).events.find(x=>x.type==="notification");require("fs").writeFileSync("answer.json",JSON.stringify({RespondToInteraction:{session_id:e.session_id,interaction_id:e.refs.interaction_id,choice_id:"cancel"}}))'
run answer bash -c 'chariox request < answer.json'
touch done
sleep 600
`, { mode: 0o755 })
  const child = spawn("bash", ["-c", `bash ${script}; true`], { env: home.env, stdio: "ignore" })
  own(child)
  const file = f => (existsSync(join(dir2, f)) ? readFileSync(join(dir2, f), "utf8") : null)
  await until(() => file("ready") !== null, "holder follows the owner-action source", 120000)
  const sent = ms()
  await home.rpc({ SubmitPrompt: { session_id: home.sessionId, attachment_id: home.attachment.id, target_agent_id: driven.id, prompt: "Your task cannot continue without the owner's decision. Call your chariox events blocked tool exactly once, with the reason \"drill: the owner must decide\", then end your turn.", attachments: [] } })
  await until(() => file("done") !== null, "the holder was woken and answered", 280000)
  const woke = Number((file("calls.log").match(/^(\d+) end wait-1/m) ?? [])[1])
  const event = JSON.parse(file("wait-1.out")).events.find(e => e.type === "notification")
  assert.equal(event.value.source_kind, "owner_action")
  assert.equal(event.value.display_name, "Agent needs your action")
  assert(/^task-blocked-/.test(event.refs.interaction_id), "the decision of a blocked task")
  assert(event.refs.task_id, "the task")
  assert.equal(JSON.parse(file("wait-1.out")).events.filter(e => e.type === "notification").length, 1, "exactly one notification")
  assert.equal(file("answer.rc").trim(), "0", `the holder answered the decision: ${file("answer.err")}`)
  assert.equal(file("calls.log").split("\n").filter(l => / start wait-/.test(l)).length, 1, "one wait, no polling")
  await until(async () => !((await home.rpc({ GetSessionState: { session_id: home.sessionId } })).SessionState.session.active_interactions ?? []).some(x => x.id === event.refs.interaction_id), "the decision is closed", 30000)
  results.ownerAction = { provider, model: agentDefaults.model, wake_after_prompt_ms: woke - sent, interaction_id: event.refs.interaction_id, task_id: event.refs.task_id, summary: event.summary, answer: "cancel", answered_rc: 0 }
  for (const f of ["calls.log", "wait-1.out", "answer.rc", "answer.err", "follow.out"]) if (file(f) !== null) writeFileSync(join(evidence, `holder-owner-action-${f}`), file(f))
  note("owner-action", results.ownerAction)
}

// ON-17: the coordinator's wait on workflow runs, without polling.
const waitScript = join(repo, "scripts/reviewer/reviewer-wait-events.mjs")
const PR_HEAD = n => `${randomBytes(20).toString("hex").slice(0, 38)}${String(n).padStart(2, "0")}`
const promptOf = (n, head) => `Handle GitHub pull_request.synchronize for charioxai/chariox #${n} at head ${head}.`
async function workflowRuns() {
  await start(home)
  const w = await reviewerWorkflow(home, "reviewer-gate")
  await home.rpc({ ApplyWorkflowDesignOp: { session_id: home.sessionId, origin_client_id: "notif-drill", op_id: "notif-concurrency", op: { kind: "workflow_update", workflow_id: w.workflow.id, patch: { max_concurrent: 4 } } } })
  const hdir = join(dir, "holder-runs"); mkdirSync(hdir)
  const script = join(hdir, "holder.sh")
  // The holder runs the wait script over and over, as the coordinator does: each invocation ends at its first finished
  // run(s). Control files: `pause` (no new invocation, the one in flight is ended), `fast` (a 6 s fallback), `stop`.
  writeFileSync(script, `#!/bin/bash
cd "$(dirname "$0")"
log() { echo "$(date +%s%3N) $*" >> calls.log; }
chariox access request --until-exit > grant.out 2> grant.err || exit 2
touch granted
( while [ ! -e stop ]; do [ -e unfollow ] && { rm -f unfollow; chariox events unsubscribe --source all > unfollow.out 2> unfollow.err; echo $? > unfollow.rc; }; sleep 0.2; done ) &
n=0
while [ ! -e stop ]; do
  while [ -e pause ]; do sleep 0.2; done
  n=$((n+1)); fb=10; [ -e fast ] && fb=0.1
  log "start rw-$n"
  node ${JSON.stringify(waitScript)} --window-minutes 4 --fallback-minutes $fb --state state.json --chariox "$(command -v chariox)" > rw-$n.out 2> rw-$n.err & pid=$!
  while kill -0 $pid 2>/dev/null; do [ -e pause ] && kill $pid 2>/dev/null; sleep 0.1; done
  wait $pid; rc=$?
  echo $rc > rw-$n.rc; cat rw-$n.out >> lines.log; log "end rw-$n rc=$rc"
  [ $rc -ne 0 ] && sleep 2
done
sleep 600
`, { mode: 0o755 })
  const child = spawn("bash", ["-c", `bash ${script}; true`], { env: home.env, stdio: "ignore" })
  own(child)
  const file = f => (existsSync(join(hdir, f)) ? readFileSync(join(hdir, f), "utf8") : null)
  const calls = () => (file("calls.log") ?? "").split("\n").filter(Boolean).map(l => { const [t, kind, name, rc] = l.split(" "); return { t: Number(t), kind, name, rc: rc?.replace("rc=", "") } })
  const lines = () => (file("lines.log") ?? "").split("\n").filter(Boolean)
  const ctl = (f, on = true) => (on ? writeFileSync(join(hdir, f), "") : rmSync(join(hdir, f), { force: true }))
  await until(() => file("granted") !== null, "the holder's grant", 120000)
  await until(() => calls().some(c => c.kind === "start" && c.name === "rw-1"), "first wait running", 30000)
  await delay(3000)
  const list = (await home.rpc({ ListNotificationSources: {} }).catch(() => null))
  // A run through the workflow: provider turn, webhook-style prompt, terminal state. `fail` runs without a provider turn.
  const invoke = async text => (await home.rpc(invokeWorkflowEndpointRequest(home.sessionId, w.workflow.id, w.endpoint.id, text))).WorkflowRunInvoked.workflow_run
  const finished = async (run, want) => until(async () => {
    const r = (await home.rpc({ GetWorkflowRun: { session_id: home.sessionId, workflow_run_ref: run.id } })).WorkflowRun.workflow_run
    return ["Completed", "Failed", "Stopped"].includes(r.status) ? r : null
  }, `run ${run.id} ${want}`, 90000)
  const okRun = async (n, head) => { await launchReviewer(home); const at = ms(); const run = await invoke(promptOf(n, head)); const done = await finished(run, "completed"); assert.equal(done.status, "Completed"); return { id: run.id, n, head, invoked_at: at, finished_at: ms() } }
  // A run that fails: a second workflow, on the same agent, whose run output must carry a verdict the dev-stub never gives.
  const broken = await reviewerWorkflow(home, "reviewer-strict")
  await home.rpc({ ApplyWorkflowDesignOp: { session_id: home.sessionId, origin_client_id: "notif-drill", op_id: "notif-schema", op: { kind: "schema_add", workflow_id: broken.workflow.id, schema: { id: "verdict", schema: { type: "object", required: ["verdict"] } } } } })
  await home.rpc({ ApplyWorkflowDesignOp: { session_id: home.sessionId, origin_client_id: "notif-drill", op_id: "notif-strict", op: { kind: "workflow_update", workflow_id: broken.workflow.id, patch: { run_output_schema_ref: "verdict" } } } })
  const failRun = async (n, head) => { await launchReviewer(home); const at = ms(); const run = (await home.rpc(invokeWorkflowEndpointRequest(home.sessionId, broken.workflow.id, broken.endpoint.id, promptOf(n, head)))).WorkflowRunInvoked.workflow_run; const done = await finished(run, "failed"); assert.equal(done.status, "Failed", `a run whose output breaks the schema fails: ${done.status}`); return { id: run.id, n, head, invoked_at: at, finished_at: ms(), message: done.failure_events?.[0]?.message } }
  const ended = name => calls().find(c => c.kind === "end" && c.name === name)
  const line = (status, n, head, tail = "") => new RegExp(`^\\d\\d:\\d\\dZ review ${status} charioxai/chariox #${n} @${head.slice(0, 10)}${tail}$`)

  // ---- completed ------------------------------------------------------------------------------------------------
  const r1 = await okRun(1301, PR_HEAD(1))
  await until(() => ended("rw-1"), "the wait returned", 30000)
  assert.equal(ended("rw-1").rc, "0")
  assert.match(file("rw-1.out").trim(), line("Completed", r1.n, r1.head), `line: ${file("rw-1.out")}`)
  results.completed = { line: file("rw-1.out").trim(), wake_after_finish_ms: ended("rw-1").t - r1.finished_at }
  assert(results.completed.wake_after_finish_ms < 15000, "seconds, not a 60 s poll")
  note("completed", results.completed)

  // ---- failed ---------------------------------------------------------------------------------------------------
  await until(() => calls().some(c => c.kind === "start" && c.name === "rw-2"), "next wait running", 30000)
  await delay(1500)
  const r2 = await failRun(1302, PR_HEAD(2))
  await until(() => ended("rw-2"), "the wait returned on the failure", 30000)
  const failedLine = file("rw-2.out").trim()
  assert.match(failedLine, line("Failed", r2.n, r2.head, ": .+"), `line: ${failedLine}`)
  results.failed = { line: failedLine, failure: r2.message, wake_after_finish_ms: ended("rw-2").t - r2.finished_at }
  assert(results.failed.wake_after_finish_ms < 15000)
  note("failed", results.failed)

  // ---- burst ----------------------------------------------------------------------------------------------------
  const before = lines().length
  const burst = []
  for (let i = 0; i < 5; i++) burst.push(await okRun(1310 + i, PR_HEAD(10 + i)))
  await until(() => lines().length >= before + 5, "all five lines", 60000)
  await delay(3000)
  const got = lines().slice(before)
  assert.equal(got.length, 5, `each run once: ${got.join(" | ")}`)
  burst.forEach((r, i) => assert.match(got[i], line("Completed", r.n, r.head), `in order: ${got[i]}`))
  results.burst = { lines: got, last_wake_after_finish_ms: Date.now() - burst.at(-1).finished_at }
  note("burst", results.burst)

  // ---- restart --------------------------------------------------------------------------------------------------
  ctl("pause"); await delay(1500)
  const r3 = await okRun(1320, PR_HEAD(20))
  await delay(1500)
  await stop(home); await start(home)
  const base = lines().length
  ctl("pause", false)
  await until(() => lines().length > base, "the run that ended before the restart", 60000)
  await delay(3000)
  assert.equal(lines().length, base + 1, "once")
  assert.match(lines().at(-1), line("Completed", r3.n, r3.head))
  results.restart = { line: lines().at(-1) }
  note("restart", results.restart)

  // ---- fallback: a notification that never arrives ----------------------------------------------------------------
  ctl("pause"); await delay(1500); ctl("fast"); ctl("pause", false)
  const startedAt = ms()
  await until(() => calls().filter(c => c.kind === "start").length > calls().filter(c => c.kind === "end").length, "the fallback wait running", 30000)
  await delay(1500) // it has followed; now the follow is removed behind its back
  ctl("unfollow")
  await until(() => file("unfollow.rc") !== null, "the follow removed", 30000)
  note("unfollow", { rc: file("unfollow.rc").trim() })
  const base2 = lines().length
  const r4 = await okRun(1330, PR_HEAD(30))
  await until(() => lines().length > base2, "the fallback listing found the run", 60000)
  results.fallback = { line: lines().at(-1), found_after_finish_ms: Date.now() - r4.finished_at, since_start_ms: Date.now() - startedAt }
  assert.match(lines().at(-1), line("Completed", r4.n, r4.head))
  note("fallback", results.fallback)
  ctl("fast", false)

  // ---- unreachable and revoked ----------------------------------------------------------------------------------------
  await until(() => calls().filter(c => c.kind === "start").length > calls().filter(c => c.kind === "end").length, "a wait running", 30000)
  await delay(2000)
  await stop(home)
  await until(() => lines().some(l => l === "reviewer kernel unreachable"), "unreachable line", 60000)
  results.unreachable = { line: "reviewer kernel unreachable", waits_ended_rc: calls().filter(c => c.kind === "end").slice(-2).map(c => c.rc) }
  note("unreachable", results.unreachable)
  // The kernel is back, the holder's grant survived it, and the owner revokes the grant while a wait runs.
  await start(home)
  await until(() => lines().length > 0 && calls().filter(c => c.kind === "start").length > calls().filter(c => c.kind === "end").length, "a wait running again", 60000)
  await delay(2000)
  const grants = (await home.rpc({ ListKernelAccessGrants: {} })).KernelAccessGrantsListed.grants
  assert.equal(grants.length, 1, "one grant")
  const baseRevoke = lines().length
  await home.rpc({ RevokeKernelAccessGrant: { grant_id: grants[0].grant_id } })
  await until(() => lines().length > baseRevoke, "the wait ended by the revoke", 30000)
  results.revoke = { line: lines()[baseRevoke], more_lines_in_2s: await delay(2000).then(() => lines().length - baseRevoke - 1) }
  note("revoke", results.revoke)
  ctl("stop")
  results.sources = (list?.NotificationSources?.sources ?? []).map(x => ({ kind: x.kind, display_name: x.display_name }))
  for (const f of ["calls.log", "lines.log", "state.json"]) if (file(f) !== null) writeFileSync(join(evidence, `holder-runs-${f}`), file(f))
  for (const f of readdirSync(hdir)) if (/^rw-\d+\.(out|err|rc)$/.test(f)) copyFileSync(join(hdir, f), join(evidence, `holder-runs-${f}`))
}
// The lifecycle proofs: follow, twice, restart, expire, revoke and refused.
async function lifecycle() {
  // ---- follow -------------------------------------------------------------------------------------------------
  await start(home)
  const reviewer = await reviewerWorkflow(home, "reviewer-gate")
  results.source = { source_id: reviewer.source.source_id, display_name: reviewer.source.display_name }
  const h = holder(home, reviewer.source.display_name || "reviewer-gate")
  await until(() => h.file("follow-owner-action.rc") !== null, "holder followed both sources", 120000)
  assert.equal(h.file("follow-workflow.rc")?.trim(), "0", `follow workflow: ${h.file("follow-workflow.err")}`)
  assert.equal(h.file("follow-owner-action.rc")?.trim(), "0", `follow owner action: ${h.file("follow-owner-action.err")}`)
  const listed = h.json("sources.out").sources
  results.sources = listed.map(s => ({ kind: s.kind, display_name: s.display_name }))
  assert(listed.some(s => s.source_id === reviewer.source.source_id && s.kind === "workflow_exit"), "the workflow exit is listed")
  assert(listed.some(s => s.kind === "owner_action" && s.display_name === "Agent needs your action"), "the owner-action source exists before any task blocked")
  const subscribed = h.json("follow-owner-action.out")
  assert.equal(subscribed.subscription.sources.length, 2, "the grant follows both sources")
  await until(() => h.calls().some(c => c.kind === "start" && c.name === "wait-1"), "first wait running", 30000)
  await delay(3000)
  const waitStart = h.calls().find(c => c.kind === "start" && c.name === "wait-1").t
  const run1 = await finishRun(home, reviewer, "review approved: ship it")
  await until(() => h.calls().some(c => c.kind === "end" && c.name === "wait-1"), "wait-1 returned", 30000)
  const waitEnd = h.calls().find(c => c.kind === "end" && c.name === "wait-1")
  assert.equal(waitEnd.rc, "0", "the wait returned on the event, not on its timer (exit 5)")
  assert(waitStart < run1.invoked_at, "the wait was already running when the run started")
  results.follow = { wait_started_before_run_ms: run1.invoked_at - waitStart, wake_after_finish_ms: waitEnd.t - run1.finished_at, wait_total_ms: waitEnd.t - waitStart }
  assert(waitEnd.t - run1.finished_at < 15000, `the wait returned ${waitEnd.t - run1.finished_at} ms after the run finished`)
  assert(waitEnd.t - waitStart < 60000, "well before the 100 s timer")
  await until(() => notifications(h).length >= 1, "the notification record", 10000)
  const first = notifications(h)
  assert.equal(first.length, 1, "exactly one notification")
  assert.equal(first[0].value.occurrence_id, run1.id, "the occurrence is the run")
  assert.equal(first[0].value.source_id, reviewer.source.source_id)
  assert.equal(first[0].value.source_kind, "workflow_exit")
  assert(String(first[0].value.output.message).includes("ship it") || String(first[0].value.output.message).length > 0, "the exit's output text")
  assert(Math.abs(first[0].value.deadline_ms - (run1.finished_at + 7 * 86400000)) < 3600000, "the deadline is 7 days")
  results.follow.record = { type: first[0].type, display_name: first[0].value.display_name, occurrence_id: first[0].value.occurrence_id, deadline_ms: first[0].value.deadline_ms, summary: first[0].summary }
  const names = h.calls().map(c => c.name).filter((n, i, a) => a.indexOf(n) === i)
  assert(!names.some(n => /^list/.test(n)), "no list call: the holder never polled")
  results.follow.holder_requests_before_wake = names.filter(n => !/^ack|^wait-[2-9]/.test(n))
  note("follow", results.follow)

  // ---- twice --------------------------------------------------------------------------------------------------
  const run2 = await finishRun(home, reviewer, "second review")
  await until(() => notifications(h).length >= 2, "the second notification", 30000)
  await delay(2000)
  const two = notifications(h)
  assert.deepEqual(two.map(e => e.value.occurrence_id), [run1.id, run2.id], "each run once, in order")
  results.twice = two.map(e => e.value.occurrence_id)

  // ---- restart ------------------------------------------------------------------------------------------------
  h.pause()
  await delay(1500)
  const run3 = await finishRun(home, reviewer, "emitted while the holder is not reading")
  await delay(1500)
  await stop(home)
  await start(home)
  h.touch("check-subscription")
  h.resume()
  await until(() => notifications(h).length >= 3, "the notification emitted before the restart", 60000)
  await delay(3000)
  const afterRestart = notifications(h)
  assert.deepEqual(afterRestart.map(e => e.value.occurrence_id), [run1.id, run2.id, run3.id], "run 3 arrives once, runs 1 and 2 are not delivered again")
  await until(() => h.file("subscription-check.out"), "the subscription after the restart", 30000)
  const resumed = h.json("subscription-check.out")
  results.restart = { occurrences: afterRestart.map(e => e.value.occurrence_id), followed_after_restart: resumed?.subscription?.sources?.map(s => s.display_name) }
  assert.equal(resumed?.subscription?.sources?.length, 2, "the grant still follows both sources after the restart")
  note("restart", results.restart)

  // ---- expire -------------------------------------------------------------------------------------------------
  h.pause()
  await delay(1500)
  const run4 = await finishRun(home, reviewer, "will be unread for 8 days")
  await delay(1500)
  await stop(home)
  writeFileSync(clockFile, String(ms() + 8 * 86400000))
  await start(home, { CHARIOX_TEST_OUTSIDE_CLOCK_MS_FILE: clockFile })
  await delay(6000)
  h.resume()
  await until(() => h.records().some(e => e.type === "gap"), "the gap", 60000)
  await delay(2000)
  const gap = h.records().find(e => e.type === "gap")
  assert.equal(gap.value.dropped, 1, "one unread notification expired")
  assert(!notifications(h).some(e => e.value.occurrence_id === run4.id), "the expired notification is not delivered")
  results.expire = { gap: gap.value, summary: gap.summary, run: run4.id }
  note("expire", results.expire)
  h.pause()
  await delay(1500)
  await stop(home)
  rmSync(clockFile, { force: true })
  await start(home)

  // ---- refused ------------------------------------------------------------------------------------------------
  await start(other)
  const foreign = await reviewerWorkflow(other, "foreign-gate")
  writeFileSync(join(holderDir, "other-kernel-source"), foreign.source.source_id)
  results.foreign_source = foreign.source.source_id

  // ---- revoke -------------------------------------------------------------------------------------------------
  const grants = (await home.rpc({ ListKernelAccessGrants: {} })).KernelAccessGrantsListed.grants
  assert.equal(grants.length, 1, "one grant")
  h.resume()
  await delay(500)
  await home.rpc({ RevokeKernelAccessGrant: { grant_id: grants[0].grant_id } })
  await until(() => h.file("ended") !== null, "the revoked wait ended", 60000)
  const ended = h.records().find(e => e.type === "subscription_ended") ?? { value: { error: (h.file("wait-ended.err") ?? "").trim().slice(0, 200) } }
  const lastWait = h.calls().filter(c => c.kind === "end" && c.name.startsWith("wait-")).at(-1)
  assert.equal(lastWait.rc, "3", "the old connection's wait exits 3")
  const run5 = await finishRun(home, reviewer, "after the revoke")
  await delay(3000)
  h.touch("phase2")
  await until(() => h.file("phase2-done") !== null, "the new grant's checks", 120000)
  // The holder's next grant is a new subscriber: it follows nothing and reads nothing of what came before.
  const fresh = h.json("subscription2.out")?.subscription
  assert(fresh && (fresh.sources ?? []).length === 0 && fresh.targets.length === 0, `a new grant follows nothing: ${h.file("subscription2.out")}`)
  assert.equal(h.json("list2.out")?.events?.length, 0, "a new grant reads no records")
  assert.notEqual(h.json("grant2.out")?.grant_id, grants[0].grant_id, "a new grant")
  assert(!JSON.stringify(h.records()).includes(run5.id), "the revoked grant received nothing more")
  results.revoke = { ended: ended.value, wait_exit: lastWait.rc, later_run: run5.id, new_grant: { sources: fresh.sources ?? [], targets: fresh.targets, records: h.json("list2.out").events.length } }
  note("revoke", results.revoke)

  // ---- refused (the kernel's own sentence, asked with a raw request) --------------------------------------------
  const refusal = name => { assert.equal(h.file(`${name}.rc`)?.trim(), "3", `${name} is refused (exit 3): ${h.file(`${name}.err`)}`); return (h.file(`${name}.err`) ?? "").trim() }
  const foreignRefusal = refusal("other-kernel-source"), unknownRefusal = refusal("unknown-source")
  assert(/source not available/.test(foreignRefusal) && /source not available/.test(unknownRefusal), "both say source not available")
  assert.equal(foreignRefusal.replace(foreign.source.source_id, "ID"), unknownRefusal.replace("wf_source_00000000000000000000000000000000", "ID"), "the two refusals read the same")
  results.refused = { other_kernel: foreignRefusal.slice(0, 200), unknown: unknownRefusal.slice(0, 200) }
  note("refused", results.refused)
  results.sources_after_revoke = (h.json("sources2.out")?.sources ?? []).map(s => s.display_name)
  note("result", results)
}
try {
  await (scenario === "owner-action" ? ownerAction() : scenario === "workflow-runs" ? workflowRuns() : lifecycle())
  if (scenario !== "main") note("result", results)
} catch (error) {
  results.failure = String(error.message ?? error).slice(0, 400)
  note("failure", { failure: results.failure })
  process.exitCode = 1
} finally {
  writeFileSync(join(evidence, "result.json"), JSON.stringify(results, null, 2) + "\n")
  for (const f of existsSync(holderDir) ? readdirSync(holderDir) : []) {
    // The holder's grant is a handle, never a credential, but only what the proof needs is kept.
    if (/^(calls\.log|records\.jsonl|loop\.log|.*\.(out|err|rc))$/.test(f)) copyFileSync(join(holderDir, f), join(evidence, `holder-${f}`))
  }
  for (const k of [home, other]) k.client?.close?.()
  for (const p of [...owned].reverse()) stopOwned(p)
  note("cleanup", { stopped: owned.length })
}
process.exit(process.exitCode ?? 0)

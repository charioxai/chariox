// MP-08 / MP-09 / MP-10 / MP-11: messages preserve a real provider's original background job.
// Run against an owned kernel binary and relay, with state/evidence outside the checkout:
// node scripts/drills/message-task-continuity.mjs --kernel-binary <kernel> --relay-binary <relay>
//   --state-root <~/.chariox/dev/task> --workspace-root <outside-checkout-directory>
//   --evidence-dir <evidence> --provider codex --codex-profile-path <product-linked-CODEX_HOME>
//   --model gpt-6-luna --mode held --seconds 1960 --age-hold yes
// --mode active checks queued non-urgent delivery and an urgent steer during a foreground command.
// Claude uses --provider claude --model <model> and CLAUDE_TOKEN_FILE for product enrollment.
import { spawn, execFileSync } from "node:child_process"
import { createHmac, createHash, randomBytes, randomUUID } from "node:crypto"
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync, rmSync } from "node:fs"
import { createServer } from "node:net"
import { resolve, join } from "node:path"
import { tmpdir } from "node:os"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../../packages/kernel-client/dist/ipc.js"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"

const flags = new Map()
for (let i = 2; i < process.argv.length; i++) flags.set(process.argv[i], process.argv[++i])
const need = key => { const v = flags.get(key); if (typeof v !== "string") throw new Error(`${key} is required`); return resolve(v) }
const binary = need("--kernel-binary"), relayBinary = need("--relay-binary"), stateRoot = need("--state-root"), evidence = need("--evidence-dir")
const provider = flags.get("--provider") ?? "claude"
if (!["codex", "claude"].includes(provider)) throw new Error("--provider must be codex or claude")
const mode = flags.get("--mode") ?? "held"
if (!["held", "active"].includes(mode)) throw new Error("--mode must be held or active")
const codexProfilePath = provider === "codex" ? need("--codex-profile-path") : null
const model = flags.get("--model") ?? (provider === "codex" ? "gpt-6-luna" : "claude-haiku-5-5")
const tokenFile = process.env.CLAUDE_TOKEN_FILE
if (provider === "claude" && !tokenFile) throw new Error("CLAUDE_TOKEN_FILE is required for Claude product enrollment")
const workspaceRoot = need("--workspace-root")
if (!stateRoot.includes("/.chariox/dev/")) throw new Error("state must be lane-owned")
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true })
mkdirSync(workspaceRoot, { recursive: true })
writeFileSync(join(evidence, "driver.mjs"), readFileSync(new URL(import.meta.url)))
const socketRoot = mkdtempSync(join(tmpdir(), "chariox-msgturn-"))
const root = mkdtempSync(join(stateRoot, "msgturn-"))
const children = [], clients = [], steps = [], workspaces = []
const t0 = Date.now()
const note = (name, data = {}) => {
  steps.push({ at: new Date().toISOString(), s: Math.round((Date.now() - t0) / 1000), name, ...data })
  writeFileSync(join(evidence, "steps.json"), JSON.stringify(steps, null, 1) + "\n", { mode: 0o600 })
  console.log(`MP-08/09/10/11 +${Math.round((Date.now() - t0) / 1000)}s ${name} ${JSON.stringify(data).slice(0, 300)}`)
}
const redact = text => String(text).replace(/sk-ant-[A-Za-z0-9_-]+/g, "<redacted>")
const env = { PATH: process.env.PATH, NODE_OPTIONS: "--max-old-space-size=3072", IS_SANDBOX: "1", CHARIOX_ROOM_AGENT_TOOLS: "1", SHELL: "/bin/bash", ...(process.env.CHARIOX_CLAUDE_BIN ? {CHARIOX_CLAUDE_BIN:process.env.CHARIOX_CLAUDE_BIN} : {}) }
const secret = randomBytes(32).toString("base64url"), issuer = `orch-${randomUUID()}`
function token(subject, kind, actions, thumbprint, targets) {
  const head = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const body = Buffer.from(JSON.stringify({ iss: issuer, sub: subject, subject_kind: kind, realm_id: issuer, allowed_actions: actions,
    allowed_targets: targets ?? null, ...(thumbprint ? { public_key_thumbprint: thumbprint } : {}), iat: Math.floor(Date.now() / 1000) - 1, exp: Math.floor(Date.now() / 1000) + 7200, jti: randomUUID(), user_id: "local" })).toString("base64url")
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
async function role(name, extraEnv = {}) {
  const dir = join(root, name), ports = []
  for (let i = 0; i < 4; i++) ports.push(await freePort())
  for (const sub of ["chariox", "home", "tmp", "work", "logs", "history", "config", "cache", "state"]) mkdirSync(join(dir, sub), { recursive: true, mode: 0o700 })
  writeFileSync(join(dir, "chariox/config.toml"), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${dir}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[workflow]\nsession_default_max_agents = 16\n[state]\npath = "${dir}/chariox/state.db"\n`, { mode: 0o600 })
  const localEnv = { ...env, HOME: join(dir, "home"), TMPDIR: join(dir, "tmp"), XDG_CONFIG_HOME: join(dir, "config"), XDG_CACHE_HOME: join(dir, "cache"), XDG_STATE_HOME: join(dir, "state"),
    CHARIOX_HOME: join(dir, "chariox"), CHARIOX_LOG_DIR: join(dir, "logs"), CHARIOX_SESSION_HISTORY_DIR: join(dir, "history"),
    CHARIOX_DAEMON_SOCKET: join(socketRoot, `${name}.sock`), CHARIOX_DAEMON_ALIAS: `orch-${name}`, CHARIOX_MACHINE_ALIAS: `orch-${name}-machine`,
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]), ...extraEnv }
  const work = mkdtempSync(join(workspaceRoot, `msgturn-${name}-`))
  workspaces.push(work)
  const r = { name, dir, work, env: localEnv, passkey: randomBytes(24).toString("hex"), mcpPort: ports[1] }
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
const rows = (db, table) => JSON.parse(execFileSync('sqlite3', ['-readonly','-json',db,`select * from ${table}`],{maxBuffer:8000000}).toString() || '[]').map(row=>row.payload ? JSON.parse(row.payload) : row)
let a, room, parent, attachment, child, original
const checks=[]
function check(id,passed,detail){checks.push({id,status:passed?"PASS":"FAIL",detail});writeFileSync(join(evidence,"verdict.json"),JSON.stringify(checks,null,2));note(id,{passed,detail})}
function captureArtifacts() {
  if(!a)return
  for(const name of ['started.json','parent-ready.json','collected.json','parent-collected.json']){
    const file=join(a.work,name)
    if(existsSync(file))writeFileSync(join(evidence,name),readFileSync(file))
  }
  const db=join(a.dir,'chariox/state.db')
  for(const table of ['agent_tasks','agent_inbox','agent_source_occurrences','agent_registrations'])writeFileSync(join(evidence,`${table}.json`),JSON.stringify(rows(db,table),null,2))
  const historyDb=join(a.dir,'chariox/history/operational.db')
  const events=JSON.parse(execFileSync('sqlite3',['-readonly','-json',historyDb,"SELECT event_json FROM history_events WHERE kind IN ('user_prompt','provider_tool','provider_output','provider_status') ORDER BY sequence"],{maxBuffer:32000000}).toString()||'[]').map(row=>JSON.parse(row.event_json))
  writeFileSync(join(evidence,'public-events.json'),JSON.stringify(events,null,2))
}
async function submit(text) {
  attachment=(await a.rpc({AttachToSession:{session_id:room,client_id:'msgturn-owner',capability_level:'FullTerminal'}})).SessionAttached.attachment.id
  await a.client.subscribeToKernelEvents(room,attachment)
  return a.rpc(submitPromptRequest(room,attachment,parent,text,[]))
}
async function main() {
  note("binaries",{kernel_sha256:createHash("sha256").update(readFileSync(binary)).digest("hex"),relay_sha256:createHash("sha256").update(readFileSync(relayBinary)).digest("hex")})
  const relayPort=await freePort()
  mkdirSync(join(root,'relay'),{recursive:true})
  start(relayBinary,{HOME:root,CHARIOX_HOME:join(root,'relay'),CHARIOX_RELAY_HOST:'127.0.0.1',CHARIOX_RELAY_PORT:String(relayPort),CHARIOX_RELAY_SCOPED_ISSUER:issuer,CHARIOX_RELAY_SCOPED_HMAC_SECRET:secret},'relay')
  a=await role('a')
  await a.rpc({ConfigureRelay:{relay_url:`ws://127.0.0.1:${relayPort}`,relay_token:token(a.status.daemon_id,'kernel',['daemon.register','daemon.heartbeat','packet.route','peer.request','peer.event'])}})
  await until(async()=>(await a.rpc({RelayStatus:null})).RelayStatus.status.connected,'relay connected')
  let profile='default'
  if(provider==='codex'){
    profile=(await a.rpc({LinkProviderAccountProfile:{provider,label:'msgturn-live',path:codexProfilePath}})).ProviderAccountProfile.profile.profile_id
    await a.rpc({RefreshProviderAccountProfile:{provider,account_profile:profile}})
  }else{
    await a.rpc({CreateCredentialVault:{origin:{kind:'passphrase',passphrase:a.passkey}}})
    await a.rpc({SetProviderAccountCredential:{provider,account_profile:profile,value:readFileSync(tokenFile,'utf8').trim(),overwrite:true}})
  }
  const work=a.work
  execFileSync('git',['init','-q','-b','main',work])
  writeFileSync(join(work,'README.md'),'Message continuity drill workspace\n')
  execFileSync('git',['-C',work,'add','README.md'])
  execFileSync('git',['-C',work,'-c','user.name=drill','-c','user.email=drill@example.invalid','commit','-q','-m','Start drill'])
  const made=(await a.rpc({CreateSession:{workspace_id:work,worktree_id:work,alias:'msgturn',agent_defaults:{provider,account_profile:profile,model,effort:'low',permission_level:'yolo'}}})).SessionCreated
  room=made.session.id;parent=made.agent.id
  await a.rpc({LaunchProviderRun:{session_id:room,agent_id:parent,adapter_key:provider,provider,account_profile:profile,model,variant:provider==='codex'?'low':null,native_tui:false}})
  const seconds=Number(flags.get('--seconds')??150)
  if(!Number.isSafeInteger(seconds)||seconds<60)throw new Error('--seconds must be an integer of at least 60')
  if(flags.get('--age-hold')==='yes'&&seconds<1900)throw new Error('--age-hold yes needs --seconds 1900 or longer')
  const activeMode=mode==="active"
  const activeCommand=`python3 -c 'import json,time; open("started.json","w").write(json.dumps({"kind":"foreground"})); time.sleep(${seconds}); print("MSGTURN-COMMAND-COMPLETE", flush=True)'`
  const childBrief=activeMode ? `Run this exact foreground shell command: ${JSON.stringify(activeCommand)}. Use a foreground timeout longer than ${seconds+60} seconds. Do not request run_in_background. Keep this same provider turn active until the command actually exits. If the tool returns a running session, keep polling it. If Chariox inbox messages arrive, read and acknowledge them, then continue waiting for the original command; never end the turn to answer a message while the command is still running. After exit write collected.json with its actual output, marker MSGTURN-COMMAND-COMPLETE and exit code, then finish MSGTURN-ORIGINAL-COMPLETE. No questions, no agents, no timers.` : `This is a message continuity drill. Start one background shell command python3 -c 'import time; time.sleep(${seconds}); print("MSGTURN-COMMAND-COMPLETE", flush=True)' using your shell tool with a short yield so it returns a running session id (Claude: run_in_background true). Write started.json containing the exact returned session id. Then end this provider turn with MSGTURN-LAUNCHED while the command continues running. Do not cancel it. A later message may ask for status; handling that message must keep this original job running. When asked to collect, poll the original session to its actual exit and write collected.json with its result and MSGTURN-COMMAND-COMPLETE, then answer MSGTURN-ORIGINAL-COMPLETE. Never ask questions or use decision cards.`
  const initial=`Use your Chariox Room tools to spawn helper --dir ${work} --provider ${provider} --model ${model} ${provider==='codex'?'--effort low':''} --permission yolo. Prompt helper with exactly ${JSON.stringify(childBrief)}. Write parent-ready.json with the helper id and accepted task_id. Read chariox.events.subscriptions for your current task and yield on EVERY live registration it lists, including both delegate and reply obligations, with a deadline 120 minutes ahead, cursor 0 and reason wait for helper. Discover the events tools if necessary. End immediately after yielding. On wakes, read the inbox. Leave child_finished receipts unhandled until collected.json exists and the queued status check reply MSGTURN-HOUSEKEEPING has arrived; only then acknowledge them. Do not cancel or delete helper until its original job and the status message have both completed. No questions or decision cards.`
  await submit(initial)
  note('parent-started',{room,parent,provider,model})
  const db=join(a.dir,'chariox/state.db')
  const tasks=()=>rows(db,'agent_tasks')
  await until(()=>existsSync(join(work,'started.json')),'child background started',240000)
  await until(()=>{original=tasks().find(t=>t.agent_id!==parent&&t.state==='working');child=original?.agent_id;return !!child},'child task',30000)
  note('background-started',{child,original})
  const historyDb=join(a.dir,'chariox/history/operational.db')
  const settledAt=()=>{
    const prompt=original.prompt_id.replaceAll("'","''")
    const values=JSON.parse(execFileSync('sqlite3',['-readonly','-json',historyDb,`SELECT json_extract(event_json,'$.metadata.prompt_settled_at_ms') AS settled FROM history_events WHERE prompt_id='${prompt}' AND kind='provider_status' AND json_extract(event_json,'$.metadata.prompt_settled_at_ms') IS NOT NULL ORDER BY sequence DESC LIMIT 1`]).toString()||'[]')
    return values[0]?.settled
  }
  let originalSettledAt
  if(!activeMode)await until(()=>{originalSettledAt=settledAt();return !!originalSettledAt},'original turn parked with background work',60000)
  await until(()=>tasks().some(t=>t.agent_id===parent&&t.state==='waiting'),'parent committed its original wait',90000)
  const parentWait=tasks().find(t=>t.agent_id===parent&&t.state==='waiting')
  await delay(1000)
  const s=await a.rpc({GetSessionState:{session_id:room}})
  writeFileSync(join(evidence,'before-message-session.json'),JSON.stringify(s,null,2))
  await submit(`Send a non-urgent message to helper through chariox.send_agent_message with reply_requested:true. Message: "Status check only. Reply MSGTURN-HOUSEKEEPING. Keep your original background command running; do not collect or cancel it yet." After sending, yield on all live registrations with a deadline 120 minutes ahead. On replies, acknowledge what you handled. Do not cancel or delete helper; preserve its original job.`)
  note('message-requested')
  const preservedParentWait=()=>{const t=tasks().find(t=>t.task_id===parentWait.task_id);return t?.state==='waiting'&&JSON.stringify(t.wait?.registration_ids)===JSON.stringify(parentWait.wait.registration_ids)&&JSON.stringify(t.obligations)===JSON.stringify(parentWait.obligations)}
  if(activeMode){
    let queued
    await until(()=>{queued=rows(db,'agent_inbox').find(e=>e.agent_id===child&&e.payload.message?.startsWith('Status check only.'));return !!queued},'non-urgent message queued',60000)
    check('MSGTURN-01-queued-while-active',queued.state==='pending'&&!queued.target_prompt_id,queued)
    const before=tasks().find(t=>t.task_id===original.task_id)
    await submit(`Send helper an urgent message using chariox.send_agent_message urgent:true and reply_requested:false: "Urgent continuity check. Keep running your original command and include MSGTURN-URGENT in its final answer. This does not cancel or replace your original task." Then yield on all live registrations. Do not acknowledge child_finished until collected.json exists and the queued status reply MSGTURN-HOUSEKEEPING arrives.`)
    let urgent
    await until(()=>{urgent=rows(db,'agent_inbox').find(e=>e.agent_id===child&&e.payload.message?.startsWith('Urgent continuity check.'));return urgent&&urgent.state!=='pending'},'urgent steer admitted',60000)
    check('MSGTURN-02-steers-original',urgent.target_prompt_id===original.prompt_id,urgent)
    const after=tasks().find(t=>t.task_id===original.task_id)
    check('MSGTURN-01-preserves-parent-wait',preservedParentWait(),tasks().find(t=>t.task_id===parentWait.task_id))
    check('MSGTURN-02-keeps-original-task',after?.state==='working'&&JSON.stringify(after.obligations)===JSON.stringify(before.obligations)&&JSON.stringify(after.wait)===JSON.stringify(before.wait),after)
    await until(()=>existsSync(join(work,'collected.json')),'original command collected',(seconds+120)*1000)
    await until(()=>tasks().find(t=>t.task_id===original.task_id)?.state==='done','original task completes',90000)
    await until(()=>rows(db,'agent_inbox').find(e=>e.sequence===queued.sequence)?.state==='handled','queued status handled',90000)
    check('MSGTURN-01-original-finished-after-collection',tasks().find(t=>t.task_id===original.task_id)?.state==='done',{})
  }else{
  const observeUntil=Date.now()+Math.min(seconds*1000-10000,120000)
  while(Date.now()<observeUntil){
    const t=tasks();const inbox=rows(db,'agent_inbox')
    writeFileSync(join(evidence,'tasks-during-message.json'),JSON.stringify(t,null,2))
    writeFileSync(join(evidence,'inbox-during-message.json'),JSON.stringify(inbox,null,2))
    const old=t.find(t=>t.task_id===original.task_id)
    if(old?.state!=='working'&&old?.state!=='waiting'){note('original-ended-during-message',{task:old});break}
    if(inbox.some(e=>e.kind==='message'&&e.agent_id===child&&e.payload.message?.startsWith('Status check only.')&&e.state==='handled')){note('message-handled',{original:old,tasks:t.filter(t=>t.agent_id===child)});await delay(5000);break}
    await delay(1000)
  }
  const afterMessage=tasks().filter(t=>t.agent_id===child)
  note('after-message',{tasks:afterMessage})
  check('MSGTURN-01-preserves-parent-wait',preservedParentWait(),tasks().find(t=>t.task_id===parentWait.task_id))
  check('MSGTURN-01-original-preserved',afterMessage.find(t=>t.task_id===original.task_id)?.state==='working',afterMessage)
  check('MSGTURN-01-reply-independent',afterMessage.some(t=>t.task_id!==original.task_id&&t.state==='done'),afterMessage)
  if(flags.get('--age-hold')==='yes'){
    // Cross the old implicit half-hour deadline while the real command is still running.
    await delay(Math.max(0,originalSettledAt+1830000-Date.now()))
    check('MSGTURN-01-long-job-preserved',tasks().find(t=>t.task_id===original.task_id)?.state==='working',tasks().find(t=>t.task_id===original.task_id))
  }
  await submit(`Send helper an urgent message using chariox.send_agent_message urgent:true, reply_requested:false: "Collect your original background command now. Poll the saved session id until actual exit, write collected.json with MSGTURN-COMMAND-COMPLETE and the real exit code, then finish MSGTURN-ORIGINAL-COMPLETE. This continues your original job." Preserve your own original child wait. Read and handle outcomes until collected.json exists and the helper's original task completes; then write parent-collected.json and finish.`)
  note('urgent-collection-requested')
  await delay(3000)
  check('MSGTURN-02-original-preserved',tasks().find(t=>t.task_id===original.task_id)?.state==='working',tasks().find(t=>t.task_id===original.task_id))
  await until(()=>existsSync(join(work,'collected.json')),'collected background result',(seconds+120)*1000)
  await until(()=>tasks().find(t=>t.task_id===original.task_id)?.state==='done','original held task settles after actual command completion',90000)
  check('MSGTURN-01-original-finished-after-collection',tasks().find(t=>t.task_id===original.task_id)?.state==='done',{})
  }
  await delay(10000)
  const history=await a.rpc({GetSessionHistoryOutline:{session_id:room,agent_ids:[child,parent],latest_prompt_count:30,include_prefetched_content:true}})
  writeFileSync(join(evidence,'history.json'),JSON.stringify(history,null,2))
  check('command-collected',readFileSync(join(work,'collected.json'),'utf8').includes('MSGTURN-COMMAND-COMPLETE'),{})
  note('finished',{tasks:tasks().map(t=>({task_id:t.task_id,agent:t.agent_id,state:t.state,reason:t.reason}))})
  if(checks.some(check=>check.status==='FAIL'))throw new Error('Message continuity assertions failed; see verdict.json')
}
try{await main()}catch(error){note('failure',{message:redact(error.message)});process.exitCode=1}
finally{
  try{captureArtifacts()}catch(error){note('evidence-capture-failed',{message:redact(error.message)});process.exitCode=1}
  if(a&&room){try{await a.rpc({EndSession:{session_id:room}})}catch(error){note('session-cleanup-failed',{message:redact(error.message)})}}
  for(const client of clients)await client.close().catch(()=>{})
  for(const child of [...children].reverse())await stop(child)
  for(const workspace of workspaces)rmSync(workspace,{recursive:true,force:true})
  rmSync(root,{recursive:true,force:true})
  rmSync(socketRoot,{recursive:true,force:true})
  writeFileSync(join(evidence,'cleanup.json'),JSON.stringify({processes:children.map(child=>({pid:child.pid,exit_code:child.exitCode,signal:child.signalCode})),state_removed:!existsSync(root),workspaces_removed:workspaces.every(work=>!existsSync(work)),socket_directory_removed:!existsSync(socketRoot)},null,2))
  note('cleanup-finished',{root})
}

#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: real TUI/provider goal drill.
// Builds are supplied explicitly; no fixture provider, direct prompt IPC or account mutation.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { mkdir, readFile, writeFile, readdir, statfs } from 'node:fs/promises'
import { DatabaseSync } from 'node:sqlite'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as sleep } from 'node:timers/promises'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import { getSessionStateRequest, linkProviderAccountProfileRequest, endSessionRequest } from '../../../packages/kernel-client/dist/ipc-requests.js'

const args = new Map()
for (let i = 2; i < process.argv.length; i += 2) args.set(process.argv[i], process.argv[i + 1])
function required(name) { const value = args.get(name); if (!value) throw new Error(`required: ${name}`); return path.resolve(value) }
const kernelBinary = required('--kernel-binary'), workspace = required('--workspace'), stateRoot = required('--state-root'), evidence = required('--evidence')
const cliEntry = args.has('--cli-entry') ? required('--cli-entry') : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../dist/index.js')
const profile = required('--linked-profile'), objective = await readFile(required('--objective-file'), 'utf8')
const before = args.get('--expect') === 'legacy'
const timeoutMs = Number(args.get('--timeout-minutes') ?? 35) * 60_000
assert(!stateRoot.startsWith(workspace + '/'), 'runtime state must be outside the workspace')
await mkdir(stateRoot, { recursive: true, mode: 0o700 }); await mkdir(evidence, { recursive: true })
const env = { ...process.env, HOME: path.join(stateRoot, 'host-home'), CHARIOX_HOME: path.join(stateRoot, 'home'),
  CHARIOX_LOG_DIR: path.join(stateRoot, 'logs'), TMPDIR: path.join(stateRoot, 'tmp'), CHARIOX_ROOM_AGENT_TOOLS: '1',
  CHARIOX_DAEMON_SOCKET: path.join(stateRoot, 'kernel.sock'), TERM: 'xterm-256color', NODE_OPTIONS: '--max-old-space-size=4096' }
for (const key of ['CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'OPENCODE_CONFIG_DIR', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_DATA_HOME']) delete env[key]
for (const dir of [env.HOME, env.TMPDIR]) await mkdir(dir, { recursive: true })
async function freePort() { const server = net.createServer(); await new Promise(r => server.listen(0, '127.0.0.1', r)); const port = server.address().port; await new Promise(r => server.close(r)); return port }
for (const key of ['CHARIOX_KERNEL_PORT','CHARIOX_MCP_PORT','CHARIOX_CODEX_PORT','CHARIOX_OPENCODE_PORT']) env[key] = String(await freePort())
const url = `ws://127.0.0.1:${env.CHARIOX_KERNEL_PORT}/kernel`, socketPath = path.join(stateRoot, 'tui.sock')
const kernelLog = createWriteStream(path.join(stateRoot, 'kernel.log'), { mode: 0o600 })
await new Promise(resolve => kernelLog.once('open',resolve))
const kernel = spawn(kernelBinary, [], { cwd: workspace, env, stdio: ['ignore', kernelLog, kernelLog] })
let tui, automation, client, sessionId, result, failure, db
const processRoots = [kernel.pid]
const ptyLog = createWriteStream(path.join(evidence, 'real-tui.ansi'), { mode: 0o600 })

// Signal only verified descendants of these own process roots. Birth times fence PID reuse.
async function ownProcesses() {
  const rows = new Map()
  for (const name of await readdir('/proc')) if (/^[0-9]+$/.test(name)) {
    try { const stat = (await readFile(`/proc/${name}/stat`, 'utf8')).split(') ')[1].split(' '); rows.set(Number(name), { pid:Number(name), parent:Number(stat[1]), born:stat[19] }) } catch {}
  }
  const owned = new Set(processRoots.filter(pid => Number.isInteger(pid) && pid > 1))
  for (let changed = true; changed;) { changed = false; for (const row of rows.values()) if (owned.has(row.parent) && !owned.has(row.pid)) { owned.add(row.pid); changed = true } }
  return [...rows.values()].filter(row => owned.has(row.pid))
}
async function signalOwn(row, signal) {
  if (!Number.isInteger(row.pid) || row.pid <= 1) throw new Error('unsafe PID')
  try { const stat = (await readFile(`/proc/${row.pid}/stat`, 'utf8')).split(') ')[1].split(' '); if (stat[19] === row.born && stat[0] !== 'Z') process.kill(row.pid, signal) } catch (error) { if (error.code !== 'ENOENT' && error.code !== 'ESRCH') throw error }
}
async function connectAutomation() {
  const socket = net.createConnection(socketPath)
  await new Promise((resolve,reject) => { socket.once('connect',resolve); socket.once('error',reject) })
  let next = 0, buffer = ''; const pending = new Map()
  socket.on('data', chunk => { buffer += chunk.toString(); for (;;) { const end = buffer.indexOf('\n'); if (end < 0) break; const response = JSON.parse(buffer.slice(0,end)); buffer = buffer.slice(end+1); const p = pending.get(response.id); if (!p) continue; pending.delete(response.id); response.ok ? p.resolve(response.data) : p.reject(new Error(response.error)) } })
  socket.on('close', () => { for (const p of pending.values()) p.reject(new Error('TUI socket closed')); pending.clear() })
  return { send(action,fields={}) { const id = ++next; return new Promise((resolve,reject) => { pending.set(id,{resolve,reject}); socket.write(JSON.stringify({id,action,...fields})+'\n') }) }, close(){socket.destroy()} }
}
async function waitUntil(read,predicate,label,timeout=60_000) { const until = Date.now()+timeout; for (;;) { const value = await read(); if (predicate(value)) return value; if (Date.now()>until) throw new Error(`timeout: ${label}`); await sleep(1000) } }
async function resourceSample() { const disk=await statfs(stateRoot); const free=disk.bavail*disk.bsize; if (free < 10*1024**3) throw new Error("disk floor approached; own drill stops"); const mem = await readFile('/proc/meminfo','utf8'); const available = Number(/^MemAvailable:\s+(\d+)/m.exec(mem)[1])*1024; await writeFile(path.join(evidence,'resources.jsonl'),JSON.stringify({date:new Date().toISOString(),mem_available:available,disk_free:free})+'\n',{flag:'a'}); if (available < 12*1024**3) throw new Error('memory floor approached; own drill stops') }
function unwrap(response) { return response.SessionState?.session ?? response.session }
try {
  const until = Date.now()+60_000
  for (;;) {
    try { client = new LocalIpcClient(url, { localAuthEnvironment:env }); await client.send({ ListSessions:null }); break } catch { await client?.close().catch(()=>{}); if (Date.now()>until) throw new Error('kernel unavailable'); await sleep(1000) }
  }
  const linked = await client.send(linkProviderAccountProfileRequest('codex','amgoal-live',profile))
  const payload = Object.values(linked)[0]
  const account = payload.profile ?? payload.account_profile
  assert(account?.id, 'product linking must return a profile id')
  await writeFile(path.join(evidence,'account-status.json'),JSON.stringify({provider:'codex',profile_id:account.id,auth_state:account.auth_state}))
  const ptyDriver = path.join(stateRoot,'pty.py')
  await writeFile(ptyDriver, `import os,pty,fcntl,termios,struct,subprocess,select,sys\nm,s=pty.openpty()\nfcntl.ioctl(s,termios.TIOCSWINSZ,struct.pack('HHHH',55,160,0,0))\np=subprocess.Popen(sys.argv[1:],stdin=s,stdout=s,stderr=s)\nos.close(s)\nwhile p.poll() is None:\n if select.select([m],[],[],0.2)[0]:\n  try: os.write(1,os.read(m,65536))\n  except OSError: break\np.wait()\n`)
  tui = spawn('python3',[ptyDriver,'bun',cliEntry,'--kernel-url',url,'--create-session','--workspace',workspace,'--worktree',workspace,'--provider','codex','--account-profile',account.id,'--model','codex/gpt-6.1-sol','--automation-socket',socketPath],{cwd:workspace,env,stdio:['ignore','pipe','pipe']})
  processRoots.push(tui.pid); tui.stdout.pipe(ptyLog); tui.stderr.pipe(ptyLog,{end:false})
  await waitUntil(async()=>{try { automation = await connectAutomation(); return true } catch { return false }},Boolean,'real TUI startup')
  const initial = await waitUntil(()=>automation.send('snapshot'),s=>s.session?.id && s.session?.agentCount===1,'real TUI attached')
  sessionId = initial.session.id
  await automation.send('submit_prompt',{prompt:`/goal --interval 1 ${objective.trim()}`})
  const deadline = Date.now()+timeoutMs; let lastSignature, sawWaiting=false, sawCheckIn=false
  while (Date.now()<deadline) {
    await resourceSample()
    const session = unwrap(await client.send(getSessionStateRequest(sessionId)))
    if (!db) {
      const kernels = await readdir(path.join(env.CHARIOX_HOME,'kernels'))
      assert.equal(kernels.length,1,'own state must bind exactly one kernel')
      db = new DatabaseSync(path.join(env.CHARIOX_HOME,'kernels',kernels[0],'state.db'),{readOnly:true})
    }
    const task = session.agent_tasks?.find(t=>t.goal)
    if (before) {
      if (session.workflow_runs?.length) { assert(!task,'legacy TUI must not create a native goal'); result={status:'EXPECTED RED',reason:'legacy planner/worker/reviewer route',workflow_count:session.workflow_runs.length,agent_count:session.agents.length}; break }
    } else if (task) {
      const signature = JSON.stringify([task.revision,task.state,task.prompt_id,task.goal.check_count])
      if (signature!==lastSignature) {
        const screen = await automation.send('snapshot')
        await writeFile(path.join(evidence,`goal-${task.revision}.json`),JSON.stringify({date:new Date().toISOString(),task,tui:{statusLine:screen.statusLine,agents:screen.session.agents}},null,2))
        lastSignature=signature
      }
      sawWaiting ||= task.state==='waiting'
      const events = db.prepare("SELECT sequence,source_id,json_extract(payload,'$.kind') AS kind,json_extract(payload,'$.state') AS state,json_extract(payload,'$.prompt_id') AS prompt_id FROM agent_inbox WHERE room_id=? AND agent_id=? ORDER BY sequence").all(sessionId,task.agent_id)
      sawCheckIn ||= events.some(e=>e.kind==='deadline_reached' && ['accepted','acknowledged','handled'].includes(e.state))
      await writeFile(path.join(evidence,'public-inbox-timeline.json'),JSON.stringify(events,null,2))
      if (task.state==='blocked') throw new Error(`first failing seam: ${task.reason}`)
      if (task.state==='done') {
        assert(task.goal.check_count>=2,'goal must evaluate multiple work turns')
        assert(task.goal.continuation_sequence>0,'independent work must continue')
        assert(sawWaiting,'goal must visibly wait on real sources')
        assert(sawCheckIn,'a real periodic check-in must be delivered')
        assert(task.goal.final_answer && task.goal.evidence.length,'verified goal conclusion required')
        result={status:'PASS',session_id:sessionId,goal_id:task.goal.id,check_count:task.goal.check_count,continuations:task.goal.continuation_sequence,saw_waiting:sawWaiting,saw_check_in:sawCheckIn,final_answer:task.goal.final_answer,evidence:task.goal.evidence}; break
      }
    }
    await sleep(2000)
  }
  assert(result,'goal drill did not reach its required disposition')
} catch(error) { failure=error; result={status:'FAIL',first_failing_seam:error.message} }
finally {
  if (client && sessionId) await Promise.race([client.send(endSessionRequest(sessionId)).catch(()=>{}),sleep(3000)])
  const owned = await ownProcesses()
  if (automation) await Promise.race([automation.send('exit').catch(()=>{}),sleep(3000)])
  automation?.close(); db?.close(); await client?.close().catch(()=>{})
  for (const row of owned.slice().reverse()) await signalOwn(row,'SIGTERM')
  await sleep(2000)
  for (const row of owned.slice().reverse()) await signalOwn(row,'SIGKILL')
  ptyLog.end(); kernelLog.end()
  await writeFile(path.join(evidence,'result.json'),JSON.stringify({...result,cleanup:{owned_pids:owned.map(r=>r.pid),remaining:(await ownProcesses()).filter(r=>owned.some(o=>o.pid===r.pid && o.born===r.born)).map(r=>r.pid)},state_root:stateRoot},null,2))
}
console.log(JSON.stringify({area:'MP-08 / MP-09 / MP-10 / MP-11 A12-S02',status:result.status,first_failing_seam:result.first_failing_seam}))
if (failure) process.exitCode=1

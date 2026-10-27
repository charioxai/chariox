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
let kernel = spawn(kernelBinary, [], { cwd: workspace, env, stdio: ['ignore', kernelLog, kernelLog] })
let tui, automation, client, sessionId, result, failure, db
const processRoots = []
await registerRoot(kernel.pid)
async function registerRoot(pid) {
  assert(Number.isInteger(pid) && pid > 1, 'unsafe own process root')
  const fields = (await readFile(`/proc/${pid}/stat`, 'utf8')).split(') ')[1].split(' ')
  assert.equal(Number(fields[1]), process.pid, 'process root must be this runner child')
  processRoots.push({pid,born:fields[19]})
}
const ptyLog = createWriteStream(path.join(evidence, 'real-tui.ansi'), { mode: 0o600 })

// Signal only verified descendants of these own process roots. Birth times fence PID reuse.
async function ownProcesses() {
  const rows = new Map()
  for (const name of await readdir('/proc')) if (/^[0-9]+$/.test(name)) {
    try { const stat = (await readFile(`/proc/${name}/stat`, 'utf8')).split(') ')[1].split(' '); rows.set(Number(name), { pid:Number(name), parent:Number(stat[1]), born:stat[19], state:stat[0] }) } catch {}
  }
  const owned = new Set(processRoots.filter(root => rows.get(root.pid)?.born === root.born).map(root => root.pid))
  // Product catalog servers can detach during startup. Read only public scope keys;
  // never retain or print any other environment value, including credentials.
  const scopeKeys = new Map([['HOME',env.HOME],['TMPDIR',env.TMPDIR],['CHARIOX_HOME',env.CHARIOX_HOME]])
  for (const row of rows.values()) {
    if (row.pid <= 1 || row.pid === process.pid) continue
    try {
      const values = (await readFile(`/proc/${row.pid}/environ`)).toString().split('\0')
      if ([...scopeKeys].some(([key,value]) => values.includes(`${key}=${value}`))) owned.add(row.pid)
    } catch {}
  }
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
  if (args.has('--pair-gate')) {
    await writeFile(path.join(evidence,'live-runtime.json'),JSON.stringify({kernel_url:url,chariox_home:env.CHARIOX_HOME,host_home:env.HOME,kernel_pid:kernel.pid,daemon_socket:env.CHARIOX_DAEMON_SOCKET}))
    await waitUntil(async()=>{try { await readFile(required('--pair-gate')); return true } catch { return false }},Boolean,'normal hosted pairing before session ownership',600_000)
  }
  const linked = await client.send(linkProviderAccountProfileRequest('codex','amgoal-live',profile))
  const payload = Object.values(linked)[0]
  let account = payload.profile ?? payload.account_profile
  assert(account?.profile_id, 'product linking must return a profile id')
  const refreshed = await client.send({RefreshProviderAccountProfile:{provider:'codex',account_profile:account.profile_id}})
  account = refreshed.ProviderAccountProfile?.profile ?? account
  assert.equal(account.auth_state,'authenticated','documented product refresh must verify the linked account')
  await writeFile(path.join(evidence,'account-status.json'),JSON.stringify({provider:'codex',profile_id:account.profile_id,auth_state:account.auth_state}))
  const ptyDriver = path.join(stateRoot,'terminal_driver.py')
  await writeFile(ptyDriver, `import os,pty,fcntl,termios,struct,subprocess,select,sys\nm,s=pty.openpty()\nfcntl.ioctl(s,termios.TIOCSWINSZ,struct.pack('HHHH',55,160,0,0))\np=subprocess.Popen(sys.argv[1:],stdin=s,stdout=s,stderr=s)\nos.close(s)\nwhile p.poll() is None:\n if select.select([m],[],[],0.2)[0]:\n  try: os.write(1,os.read(m,65536))\n  except OSError: break\np.wait()\n`)
  tui = spawn('python3',[ptyDriver,'bun',cliEntry,'--kernel-url',url,'--create-session','--workspace',workspace,'--worktree',workspace,'--provider','codex','--account-profile',account.profile_id,'--model','codex/gpt-6.1-sol','--automation-socket',socketPath],{cwd:workspace,env,stdio:['ignore','pipe','pipe']})
  await registerRoot(tui.pid); tui.stdout.pipe(ptyLog); tui.stderr.pipe(ptyLog,{end:false})
  await waitUntil(async()=>{try { automation = await connectAutomation(); return true } catch { return false }},Boolean,'real TUI startup')
  const initial = await waitUntil(()=>automation.send('snapshot'),s=>s.session?.id && s.session?.agentCount===1,'real TUI attached')
  sessionId = initial.session.id
  await writeFile(path.join(evidence,'live-runtime.json'), JSON.stringify({kernel_url:url,chariox_home:env.CHARIOX_HOME,host_home:env.HOME,kernel_pid:kernel.pid,daemon_socket:env.CHARIOX_DAEMON_SOCKET,tui_pid:tui.pid,session_id:sessionId,source_head:args.get('--source-head')??null}))
  if (args.has('--start-gate')) await waitUntil(async()=>{try { await readFile(required('--start-gate')); return true } catch { return false }},Boolean,'owner-driven hosted client attachment',600_000)
  const submitted = await automation.send('submit_prompt',{prompt:`/goal ${args.get('--goal-interval')==='default'?'':`--interval ${args.get('--goal-interval')??1} `}${objective.trim()}`})
  await writeFile(path.join(evidence,'submit-status.json'),JSON.stringify({statusLine:submitted.statusLine,footer:submitted.footer}))
  if (['resume-red','resume-green'].includes(args.get('--expect'))) {
    const start = await waitUntil(async()=>unwrap(await client.send(getSessionStateRequest(sessionId)))?.agent_tasks?.find(t=>t.goal),Boolean,'real native goal admitted')
    await automation.send('submit_prompt',{prompt:'/goal pause'})
    const paused = await waitUntil(async()=>unwrap(await client.send(getSessionStateRequest(sessionId)))?.agent_tasks?.find(t=>t.goal),t=>t?.goal.paused,'real owner pause')
    await sleep(3000)
    await automation.send('submit_prompt',{prompt:'/goal resume'})
    const resumed = await waitUntil(async()=>unwrap(await client.send(getSessionStateRequest(sessionId)))?.agent_tasks?.find(t=>t.goal),t=>t?.revision>paused.revision&&!t.goal.paused,'real owner resume')
    await writeFile(path.join(evidence,'resume-counter-red.json'),JSON.stringify({area:'MP-08 / MP-09 / MP-10 / MP-11 A12-S02',start,paused,resumed,ansi_bytes:ptyLog.bytesWritten},null,2))
    assert.equal(resumed.goal.continuation_sequence,paused.goal.continuation_sequence+1,'owner Resume must retain the shared incremented continuation sequence')
    if (args.get('--expect') === 'resume-red') throw Error('counter RED reproduction unexpectedly passed')
    result={status:'PASS',control:'real owner pause/resume',before_sequence:paused.goal.continuation_sequence,after_sequence:resumed.goal.continuation_sequence}
  }
  const deadline = Date.now()+timeoutMs; let lastSignature, sawWaiting=false, sawCheckIn=false, restarted=false
  while (!result && Date.now()<deadline) {
    await resourceSample()
    const session = unwrap(await client.send(getSessionStateRequest(sessionId)))
    const task = session.agent_tasks?.find(t=>t.goal)
    if (before) {
      if (session.workflow_runs?.length) { assert(!task,'legacy TUI must not create a native goal'); result={status:'EXPECTED RED',ansi_bytes:ptyLog.bytesWritten,reason:'legacy planner/worker/reviewer route',workflow_count:session.workflow_runs.length,agent_count:session.agents.length}; break }
    } else if (task) {
      if (!db) db = new DatabaseSync(path.join(env.CHARIOX_HOME,'state','kernel.db'),{readOnly:true})
      const signature = JSON.stringify([task.revision,task.state,task.prompt_id,task.goal.check_count])
      if (signature!==lastSignature) {
        const screen = await automation.send('snapshot')
        await writeFile(path.join(evidence,`goal-${task.revision}.json`),JSON.stringify({date:new Date().toISOString(),ansi_bytes:ptyLog.bytesWritten,task,plain_tasks:session.agent_tasks.filter(t=>!t.goal&&t.completion_check).map(t=>({task_id:t.task_id,agent_id:t.agent_id,state:t.state,completion_check:t.completion_check})),tui:{statusLine:screen.statusLine,agents:screen.session.agents}},null,2))
        lastSignature=signature
      }
      if (['source-loss-restart','source-delete-restart'].includes(args.get('--expect')) && task.state==='waiting' && task.no_progress_wakes>=Number(args.get('--min-guard')??0) && !restarted) {
        assert(task.goal.next_check_in_ms>Date.now()+5000, 'restart must precede the check-in deadline')
        if(args.get('--expect')==='source-delete-restart') {
          const delegated=new Set(task.obligations.filter(o=>o.kind==='delegate'&&o.status==='open').map(o=>o.resource_id))
          const victims=session.agents.filter(a=>delegated.has(a.id))
          assert(victims.length>0,'required actual source agent must exist')
          for(const a of victims) await automation.send('submit_prompt',{prompt:`/agent delete ${a.agent_ref??a.id}`})
          await writeFile(path.join(evidence,'source-deletion.json'),JSON.stringify({date:new Date().toISOString(),source_agents:victims.map(a=>({id:a.id,ref:a.agent_ref})),ansi_bytes:ptyLog.bytesWritten},null,2))
        }
        const own=await ownProcesses(), providers=[]
        for(const r of own){try {const comm=(await readFile(`/proc/${r.pid}/comm`,'utf8')).trim(); const a=(await readFile(`/proc/${r.pid}/cmdline`,'utf8')).split('\0'); if(comm==='codex' && a[1]==='app-server') providers.push(r)}catch{}}
        assert(providers.length>=2,'real root and source provider processes must exist')
        await writeFile(path.join(evidence,'before-source-loss.json'),JSON.stringify({date:new Date().toISOString(),task,provider_pids:providers.map(p=>({pid:p.pid,born:p.born})),ansi_bytes:ptyLog.bytesWritten},null,2))
        for(const r of providers) await signalOwn(r,'SIGKILL')
        const kr=own.find(r=>r.pid===kernel.pid);assert(kr,'own exact kernel root required');await signalOwn(kr,'SIGKILL')
        await new Promise(resolve=>kernel.once('exit',resolve))
        kernel=spawn(kernelBinary,[],{cwd:workspace,env,stdio:['ignore',kernelLog,kernelLog]});await registerRoot(kernel.pid)
        await client.close();client=new LocalIpcClient(url,{localAuthEnvironment:env})
        const recovered=await waitUntil(async()=>{try {return unwrap(await client.send(getSessionStateRequest(sessionId)))?.agent_tasks?.find(t=>t.goal)}catch{}},Boolean,'same persisted goal after real restart',45000)
        await writeFile(path.join(evidence,'after-restart.json'),JSON.stringify({date:new Date().toISOString(),task:recovered,ansi_bytes:ptyLog.bytesWritten},null,2))
        assert.equal(recovered.goal.id,task.goal.id);assert.equal(recovered.goal.objective,task.goal.objective);assert(recovered.no_progress_wakes>=task.no_progress_wakes,'restart must not erase progress guard')
        let observation=0; const settled=await waitUntil(async()=>{const state=unwrap(await client.send(getSessionStateRequest(sessionId)));const t=state?.agent_tasks?.find(t=>t.goal);if(t)await writeFile(path.join(evidence,`restart-state-${observation++}.json`),JSON.stringify({date:new Date().toISOString(),task:t,agents:state.agents.map(a=>({id:a.id,state:a.state,is_processing:a.is_processing})),ansi_bytes:ptyLog.bytesWritten},null,2));return t},t=>t?.state==='blocked'||t?.state==='done', 'bounded source-loss recovery or verified reconciliation',360000)
        await writeFile(path.join(evidence,'source-loss-recovery.json'),JSON.stringify({date:new Date().toISOString(),task:settled,ansi_bytes:ptyLog.bytesWritten},null,2))
        if(settled.state==='done')assert(settled.obligations.every(o=>['satisfied','cancelled'].includes(o.status))&&settled.goal.final_answer&&settled.goal.evidence.length,'reconciled goal needs verified completion and zero open obligations')
        const lossEvents=db.prepare("SELECT sequence,source_id,occurrence_id,json_extract(payload,'$.kind') AS kind,json_extract(payload,'$.state') AS state FROM agent_inbox WHERE room_id=? AND agent_id=? AND json_extract(payload,'$.kind')='source_lost' AND json_extract(payload,'$.payload.task_id')=?").all(sessionId,task.agent_id,task.task_id)
        assert(lossEvents.length>0,'actual lost-source occurrence must exist')
        await writeFile(path.join(evidence,'source-loss-events.json'),JSON.stringify(lossEvents,null,2))
        result={status:['blocked','done'].includes(settled.state)?'PASS':'FAIL',cell:'real source loss and kernel restart before due',goal_id:task.goal.id,guard_before:task.no_progress_wakes,guard_after:recovered.no_progress_wakes,disposition:settled.state,reason:settled.reason,scope:'all provider sources were lost; separate required-subset kill and overdue multi-interval coalescing are not established'}
        assert.equal(result.status,'PASS','lost sources must visibly block or independently reconcile, not silently wait');restarted=true;break
      }
      sawWaiting ||= task.state==='waiting'
      const events = db.prepare("SELECT sequence,source_id,json_extract(payload,'$.kind') AS kind,json_extract(payload,'$.state') AS state,json_extract(payload,'$.prompt_id') AS prompt_id FROM agent_inbox WHERE room_id=? AND agent_id=? ORDER BY sequence").all(sessionId,task.agent_id)
      sawCheckIn ||= events.some(e=>e.kind==='deadline_reached' && ['accepted','acknowledged','handled'].includes(e.state))
      await writeFile(path.join(evidence,'public-inbox-timeline.json'),JSON.stringify(events,null,2))
      if (task.state==='blocked' && !task.goal.paused) throw new Error(`first failing seam: ${task.reason}`)
      if (task.state==='done') {
        assert(task.goal.check_count>=2,'goal must evaluate multiple work turns')
        assert(task.goal.continuation_sequence>0,'independent work must continue')
        assert(sawWaiting,'goal must visibly wait on real sources')
        assert(sawCheckIn,'a real periodic check-in must be delivered')
        assert(task.goal.final_answer && task.goal.evidence.length,'verified goal conclusion required')
        if(args.get('--require-plain-check')==='true')assert(session.agent_tasks.some(t=>!t.goal&&t.state==='done'&&t.completion_check?.check_count>=1&&t.completion_check.final_answer&&t.completion_check.evidence.length),'real plain/delegate done candidate must complete a native confirmation without goal mode')
        result={status:'PASS',session_id:sessionId,goal_id:task.goal.id,check_count:task.goal.check_count,continuations:task.goal.continuation_sequence,saw_waiting:sawWaiting,saw_check_in:sawCheckIn,final_answer:task.goal.final_answer,evidence:task.goal.evidence}; break
      }
    }
    await sleep(2000)
  }
  assert(result,'goal drill did not reach its required disposition')
  if(['source-loss-restart','source-delete-restart'].includes(args.get('--expect')))assert(restarted,'requested real source fault and restart must actually run')
  if (args.has('--finish-gate')) await waitUntil(async()=>{try { await readFile(required('--finish-gate')); return true } catch { return false }},Boolean,'real hosted final-state capture',Number(args.get('--finish-timeout-minutes')??10)*60_000)
} catch(error) { failure=error; result={status:'FAIL',first_failing_seam:error.message} }
finally {
  const owned = await ownProcesses()
  if (client && sessionId) await Promise.race([client.send(endSessionRequest(sessionId)).catch(()=>{}),sleep(3000)])
  if (automation) await Promise.race([automation.send('exit').catch(()=>{}),sleep(3000)])
  automation?.close(); db?.close(); await client?.close().catch(()=>{})
  for (const row of owned.slice().reverse()) await signalOwn(row,'SIGTERM')
  await sleep(2000)
  const late = await ownProcesses()
  for (const row of late) if (!owned.some(old => old.pid === row.pid && old.born === row.born)) owned.push(row)
  for (const row of owned.slice().reverse()) await signalOwn(row,'SIGKILL')
  await sleep(500)
  ptyLog.end(); kernelLog.end()
  await writeFile(path.join(evidence,'result.json'),JSON.stringify({...result,cleanup:{owned_pids:owned.map(r=>r.pid),remaining:(await ownProcesses()).filter(r=>r.state !== 'Z').map(r=>r.pid)},state_root:stateRoot},null,2))
}
console.log(JSON.stringify({area:'MP-08 / MP-09 / MP-10 / MP-11 A12-S02',status:result.status,first_failing_seam:result.first_failing_seam}))
if (failure) process.exitCode=1

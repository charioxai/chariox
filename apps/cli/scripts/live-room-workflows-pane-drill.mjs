#!/usr/bin/env node
// MP-08/MP-10/MP-11: real kernel, local relay, official Codex and keyboard TUI flow.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile, readdir, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnOwned, signalOwnedProcess, signalOwnedProcessGroup, ownedProcessGroupHandles } from '../../kernel/slice-linux-docker/owned-process-signals.mjs'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import * as requests from '../../../packages/kernel-client/dist/ipc-requests.js'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const args = Object.fromEntries(process.argv.slice(2).reduce((pairs, arg, index, all) => index % 2 ? pairs : [...pairs, [arg.slice(2), all[index + 1]]], []))
for (const key of ['kernel', 'relay', 'client', 'account-dir', 'output', 'state-parent']) assert.ok(path.isAbsolute(args[key] ?? ''), `MP-08 --${key} requires an absolute path`)
assert.ok(!args.output.startsWith(repo + '/') && !args['state-parent'].startsWith(repo + '/'))
const model=args.model??'gpt-5.5'
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const unwrap = (response, key) => { if (response.Error) throw new Error(response.Error.message); assert.ok(response[key], `MP-08 missing ${key}`); return response[key] }
await mkdir(args.output, { recursive: true, mode: 0o700 })
await mkdir(args['state-parent'], { recursive: true, mode: 0o700 })
const state = await mkdtemp(path.join(args['state-parent'], 'workflow-'))
const workspace = await mkdtemp('/root/work/agent-relost-drill-')
for(const gitArgs of [['init','-q'],['-c','user.name=Chariox relost drill','-c','user.email=noreply@openai.com','commit','--allow-empty','-q','-m','MP-08 isolated TUI drill workspace [skip ci]\n\nCo-Authored-By: GPT-6.1-sol (Codex) <noreply@openai.com>']]) {
  const git=spawnOwned('git',gitArgs,{cwd:workspace,stdio:'ignore'})
  await new Promise((resolve,reject)=>git.once('exit',code=>code===0?resolve():reject(new Error('MP-08 Git fixture initialization failed'))))
}
const ports = []
for (let i = 0; i < 5; i++) {
  const server = net.createServer(); await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  ports.push(server.address().port); await new Promise(resolve => server.close(resolve))
}
const env = { ...Object.fromEntries(Object.entries(process.env).filter(([key])=>!key.startsWith('CHARIOX_'))), HOME: path.join(state, 'user'), CHARIOX_HOME: path.join(state, 'kernel'),
  XDG_CONFIG_HOME: path.join(state, 'config'), XDG_STATE_HOME: path.join(state, 'state'),
  XDG_DATA_HOME: path.join(state, 'data'), XDG_CACHE_HOME: path.join(state, 'cache'),
  CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]),
  CHARIOX_CODEX_PORT: String(ports[2]), CHARIOX_OPENCODE_PORT: String(ports[3]),
  CHARIOX_RELAY_PORT: String(ports[4]), CHARIOX_LOG_DIR: path.join(state,'logs'),
  CHARIOX_DAEMON_SOCKET: path.join(state, 'kernel.sock'),
  TERM: 'xterm-256color', COLORTERM: 'truecolor' }
for (const key of ['CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'OPENCODE_CONFIG_DIR', 'CHARIOX_RELAY_URL', 'CHARIOX_RELAY_TOKEN', 'CHARIOX_CLOUD_PROFILE', 'CHARIOX_CLOUD_TOKEN', 'CHARIOX_CLOUD_RELAY_CONFIG_JSON', 'CHARIOX_CLOUD_RELAY_CONFIG_PATH']) delete env[key]
const receipt = { mpItems: ['MP-08','MP-10','MP-11'], source: args.source, clientSource: args['client-source'], model,
  startedAt: new Date().toISOString(), steps: [], resources: [], limitations: ['Local TUI acceptance; Cloud union Web pane and fresh Path-1 comparison are separate gates.'] }
for (const key of ['kernel','relay','client']) receipt[key + 'Sha256'] = createHash('sha256').update(await readFile(args[key])).digest('hex')
const bundle = createHash('sha256')
for (const name of (await readdir(path.dirname(args.client))).filter(name=>name.endsWith('.js')).sort()) {
  bundle.update(name+'\0');bundle.update(await readFile(path.join(path.dirname(args.client),name)))
}
receipt.clientBundleSha256=bundle.digest('hex')
const resources = async () => { const memory = await readFile('/proc/meminfo','utf8'); const kib = Number(memory.match(/^MemAvailable:\s+(\d+)/m)[1]); receipt.resources.push({ at: new Date().toISOString(), memAvailableKiB: kib }); assert.ok(kib >= 9 * 1024 * 1024, 'MP-11 memory floor'); }
await resources()
let kernel, relay, terminal, client, sessionId
let buffer = '', nextId = 0
const pending = new Map()
const terminalCommand = (action, fields = {}) => new Promise((resolve,reject) => {
  const id = ++nextId
  const timer=setTimeout(()=>{pending.delete(id);reject(new Error('MP-08 terminal command timeout'))},10000)
  pending.set(id,{resolve: result=>{clearTimeout(timer);resolve(result)},reject: error=>{clearTimeout(timer);reject(error)}})
  terminal.stdin.write(JSON.stringify({id,action,...fields})+'\n')
})
async function capture(step, predicate, timeout = 15000) {
  const deadline = Date.now()+timeout
  let result
  do {
    result = await terminalCommand('capture',{prefix:path.join(args.output,step)})
    if (predicate(result.text)) { receipt.steps.push({step, status:'GREEN'}); return result.text }
    assert.equal(result.exitCode, null, `MP-08 TUI exited at ${step}`)
    await sleep(250)
  } while (Date.now()<deadline)
  throw new Error(`MP-08 TUI assertion failed at ${step}`)
}
const key = bytes => terminalCommand('key',{bytes:Buffer.from(bytes).toString('base64')})
let lastDiagnostics=0
async function diagnostics(result) {
  const records=[]
  for(const name of await readdir(path.join(state,'logs')).catch(()=>[])) {
    if(!name.endsWith('.ndjson'))continue
    const lines=(await readFile(path.join(state,'logs',name),'utf8')).trim().split('\n').slice(-100)
    for(const line of lines) {
      let entry;try{entry=JSON.parse(line)}catch{continue}
      if(entry.component!=='daemon.app_events')continue
      const error=String(entry.error??'')
      records.push({component:entry.component,message:entry.message,error:error.includes('is not authenticated;')?'provider account lacks successful product auth observation':/token|credential|bearer|secret|passphrase|auth/i.test(error)?'[credential-related diagnostic suppressed]':error})
    }
  }
  await writeFile(path.join(args.output,'runtime-pump.json'),JSON.stringify({mpItems:['MP-08','MP-11'],messages:records,
    homeKernelId:result.room_workflows?.home_kernel_id,hostDaemonId:result.session.host_daemon_id,
    activity:result.agent_activity,runs:result.session.workflow_runs?.map(run=>({id:run.id,status:run.status,nodes:run.node_runs?.map(node=>({id:node.id,status:node.status,agentId:node.agent_id}))}))},null,2)+'\n',{mode:0o600})
}
async function stateUntil(predicate, timeout=60000) {
  const deadline = Date.now()+timeout
  do { const result=unwrap(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState'); if(predicate(result))return result; if(Date.now()-lastDiagnostics>5000){lastDiagnostics=Date.now();await diagnostics(result)} await sleep(100) }while(Date.now()<deadline)
  throw new Error('MP-08 kernel workflow state did not settle')
}
try {
  relay = spawnOwned(args.relay, [], { cwd:repo,env,stdio:'ignore',detached:true })
  kernel = spawnOwned(args.kernel, [], { cwd:repo,env,stdio:'ignore',detached:true })
  client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}`, {localAuthEnvironment:env,controlRequestRetryDeadlineMs:15000})
  for(let i=0;;i++) {if(kernel.exitCode!==null)throw new Error(`MP-08 kernel startup exited (${kernel.exitCode})`);try{await client.send(requests.listSessionsRequest());break}catch(error){if(i>=80)throw error;await sleep(250)}}
  const profile = unwrap(await client.send(requests.linkProviderAccountProfileRequest('codex','relost-acct-686',args['account-dir'])), 'ProviderAccountProfile').profile
  const auth=unwrap(await client.send(requests.getProviderAuthStatusRequest('codex',profile.profile_id)), 'ProviderAuthStatus').status
  receipt.providerAuth={authState:auth.auth_state,version:auth.detected_version}
  assert.equal(auth.auth_state,'authenticated','MP-08 linked account must pass the product auth observation before new work')
  const created=unwrap(await client.send(requests.createSessionRequest(workspace,workspace,'relost-workflows')), 'SessionCreated')
  sessionId=created.session.id
  const agent=unwrap(await client.send(requests.spawnAgentRequest(sessionId,'codex','workflow-codex',model,workspace,'low','build','yolo',undefined,undefined,undefined,profile.profile_id)), 'AgentSpawned').agent
  unwrap(await client.send(requests.createAgentWorkflowRequest(sessionId,agent.id,'trigger','tui','Review')), 'AgentWorkflowCreated')
  const inventoryState=await stateUntil(s=>s.room_workflows?.workflow_count===1)
  const heading=inventoryState.room_workflows.workflows[0].label+' /'
  receipt.workflowLabel=inventoryState.room_workflows.workflows[0].label
  terminal=spawnOwned('python3',[path.join(repo,'apps/cli/scripts/lib/room-workflows-tui-pty.py'),path.join(repo,'apps/kernel/slice-linux-docker'),
    'bun',args.client,'--kernel-url',`ws://127.0.0.1:${ports[0]}`,'--session',sessionId,'--workspace',workspace,'--worktree',workspace,
    '--provider','codex','--model',model,'--account-profile',profile.profile_id,'--client-id','relost-tui'],{cwd:repo,env,stdio:['pipe','pipe','pipe'],detached:true})
  terminal.stdout.on('data', chunk=> {buffer+=chunk;while(buffer.includes('\n')){const i=buffer.indexOf('\n');const message=JSON.parse(buffer.slice(0,i));buffer=buffer.slice(i+1);const waiter=pending.get(message.id);pending.delete(message.id);if(message.error)waiter?.reject(new Error(message.error));else waiter?.resolve(message.result)}})
  terminal.stderr.on('data', chunk=> { /* Only fixture diagnostics; no provider stdout is connected here. */ receipt.terminalError=String(chunk).slice(-1000) })
  terminal.on('close',()=>{for(const waiter of pending.values())waiter.reject(new Error('MP-08 terminal driver exited'));pending.clear()})
  await capture('01-room-inventory',text=>text.includes(heading))
  await key('\x17') // Ctrl+W
  await key('Run sleep 30; reply HOLD.')
  await capture('02-independent-draft',text=>text.includes('Run sleep 30; reply HOLD.'))
  await key('\x1b')
  await capture('03-dismissed',text=>!text.includes('Start '+heading))
  await key('\x17')
  await capture('04-reopened-draft',text=>text.includes('Run sleep 30; reply HOLD.')&&text.includes('Start '+heading))
  await key('\r')
  const started = await stateUntil(s=>s.room_workflows.workflows[0].runs.length>0)
  receipt.runId=started.room_workflows.workflows[0].runs[0].run_id
  await capture('05-start-confirmed',text=>text.includes('Started run')||text.includes('running'))
  await key('\x10') // Ctrl+P: captured current run
  await stateUntil(s=>s.room_workflows.workflows[0].paused_count===1)
  await capture('06-paused',text=>text.includes('1 paused'))
  await key('\x12') // Ctrl+R
  await stateUntil(s=>s.room_workflows.workflows[0].running_count===1)
  await capture('07-resumed',text=>text.includes('1 running'))
  await key('\x13') // Ctrl+S
  await stateUntil(s=>s.room_workflows.workflows[0].running_count===0&&s.room_workflows.workflows[0].paused_count===0)
  await capture('08-stopped',text=>text.includes('0 running')&&text.includes('0 paused'))
  const oldRunIds=new Set((await stateUntil(()=>true)).room_workflows.workflows[0].runs.map(run=>run.run_id))
  await key('Return the required workflow JSON envelope with summary done and output.message RELOST_WORKFLOW_OK.')
  await key('\r')
  const admitted=await stateUntil(s=>s.room_workflows.workflows[0].runs.some(run=>!oldRunIds.has(run.run_id)))
  const completionRunId=admitted.room_workflows.workflows[0].runs.find(run=>!oldRunIds.has(run.run_id)).run_id
  // Use a real provider completion and the terminal's real agent panes as proof.
  await key('\t')
  const settled=await stateUntil(s=>s.room_workflows.workflows[0].running_count===0,180000)
  const finalRun=settled.session.workflow_runs.find(run=>run.id===completionRunId)
  receipt.finalRun={runId:finalRun.id,status:finalRun.status}
  assert.equal(finalRun.status.toLowerCase(),'completed','MP-08 workflow must complete successfully, not merely stop')
  const history=unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId,[agent.id],4)), 'SessionHistoryOutline')
  const hasMarker=value=>{
    if(value?.entry.kind!=='provider_output')return false
    const text=value.entry.text.trim()
    try {const envelope=JSON.parse(text.replace(/^```json\s*/, '').replace(/\s*```$/, ''));return envelope.output?.message==='RELOST_WORKFLOW_OK'}catch{return false}
  }
  const completed=history.agents.flatMap(agent=>agent.turns).find(turn=>
    [turn.summary,...turn.entries].some(hasMarker))
  receipt.providerTurnEvidence=history.agents.flatMap(value=>value.turns).map(turn=>({turnId:turn.turn_id,lifecycle:turn.lifecycle,hasMarker:[turn.summary,...turn.entries].some(hasMarker)}))
  assert.ok(completed,'MP-08 official provider output must be distinct from the user prompt')
  receipt.providerCompletion={provider:'codex',agentId:agent.id,turnId:completed.turn_id,lifecycle:completed.lifecycle,output:'RELOST_WORKFLOW_OK'}
  await key('\t') // Navigate from the primary agent pane to the workflow agent.
  await capture('09-provider-completion',text=>text.includes('RELOST_WORKFLOW_OK'),60000)
  receipt.status='GREEN'
} catch(error) {
  receipt.status='RED';receipt.firstFailure=error.message
  if(terminal&&terminal.exitCode===null)await terminalCommand('capture',{prefix:path.join(args.output,'failure')}).catch(()=>{})
  if (args['expect-red']==='1' && error.message==='MP-08 TUI assertion failed at 01-room-inventory')receipt.expectedRed=true
  else process.exitCode=1
} finally {
  await writeFile(path.join(args.output,'result.json'),JSON.stringify(receipt,null,2)+'\n',{mode:0o600})
  const cleanupGroups=[]
  for(const child of [kernel,relay].filter(Boolean)){
    for(let attempt=0;;attempt++){
      try{cleanupGroups.push(...ownedProcessGroupHandles(child));break}
      catch(error){if(attempt>=9)throw error;await sleep(100)}
    }
  }
  if(sessionId)await client?.send(requests.endSessionRequest(sessionId)).catch(()=>{})
  if(terminal&&terminal.exitCode===null&&terminal.signalCode===null){terminal.stdin.write(JSON.stringify({id:++nextId,action:'close'})+'\n');await sleep(1000);if(terminal.exitCode===null)signalOwnedProcess(terminal,'SIGTERM')}
  await client?.close().catch(()=>{})
  for(const handle of cleanupGroups)signalOwnedProcessGroup(handle,'SIGINT')
  await sleep(2000)
  for(const handle of cleanupGroups)signalOwnedProcessGroup(handle,'SIGKILL')
  await rm(state,{recursive:true,force:true})
  await rm(workspace,{recursive:true,force:true})
  assert.ok([terminal,kernel,relay].filter(Boolean).every(child=>child.exitCode!==null||child.signalCode!==null),'MP-11 owned process cleanup failed')
  receipt.cleanup='owned session ended, TUI/kernel/relay stopped, disposable state removed; linked account left in place'
  await resources()
  receipt.finishedAt=new Date().toISOString()
  await writeFile(path.join(args.output,'result.json'),JSON.stringify(receipt,null,2)+'\n',{mode:0o600})
  console.log(JSON.stringify({mpItems:receipt.mpItems,status:receipt.status,expectedRed:receipt.expectedRed,firstFailure:receipt.firstFailure,output:args.output}))
}

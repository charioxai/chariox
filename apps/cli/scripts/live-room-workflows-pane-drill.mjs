#!/usr/bin/env node
// MP-08/MP-10/MP-11: real kernel, local relay, official Codex and keyboard TUI flow.
import assert from 'node:assert/strict'
import { createHash, randomBytes } from 'node:crypto'
import { readFile, readdir, mkdir, mkdtemp, rm, statfs, writeFile } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnOwned, signalOwnedProcess, signalOwnedProcessGroup, ownedProcessGroupHandles } from '../../kernel/slice-linux-docker/owned-process-signals.mjs'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import * as requests from '../../../packages/kernel-client/dist/ipc-requests.js'
import { interruptTraces, interruptTiming } from './lib/workflow-interrupt-evidence.mjs'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const args = Object.fromEntries(process.argv.slice(2).reduce((pairs, arg, index, all) => index % 2 ? pairs : [...pairs, [arg.slice(2), all[index + 1]]], []))
for (const key of ['kernel', 'relay', 'client', 'account-dir', 'output', 'state-parent']) assert.ok(path.isAbsolute(args[key] ?? ''), `MP-08 --${key} requires an absolute path`)
assert.ok(!args.output.startsWith(repo + '/') && !args['state-parent'].startsWith(repo + '/'))
const model=args.model??'gpt-5.5'
const relayTransport = args.transport === 'relay'
const interruptRaceRounds = Number(args['interrupt-race-rounds'] ?? 0)
assert.ok(Number.isInteger(interruptRaceRounds) && interruptRaceRounds >= 0 && interruptRaceRounds <= 30)
const noisyInterruptRounds = Number(args['noisy-interrupt-rounds'] ?? 0)
assert.ok(Number.isInteger(noisyInterruptRounds) && noisyInterruptRounds >= 0 && noisyInterruptRounds <= 60)
const reconnectRounds = new Set((args["reconnect-rounds"] ?? "").split(",").filter(Boolean).map(Number))
const busyRefusalRounds = Number(args['busy-refusal-rounds'] ?? 0)
assert.ok(Number.isSafeInteger(busyRefusalRounds)&&busyRefusalRounds>=0&&busyRefusalRounds<=3)
const tracingInterrupts = interruptRaceRounds > 0 || noisyInterruptRounds > 0 || busyRefusalRounds > 0
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const unwrap = (response, key) => { if (response.Error) throw new Error(response.Error.message); assert.ok(response[key], `MP-08 missing ${key}`); return response[key] }
await mkdir(args.output, { recursive: true, mode: 0o700 })
await mkdir(args['state-parent'], { recursive: true, mode: 0o700 })
const state = await mkdtemp(path.join(args['state-parent'], 'workflow-'))
const workspace = await mkdtemp('/root/work/agent-wfpause-drill-')
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
  CHARIOX_LOG_LEVEL: tracingInterrupts ? 'debug' : 'info',
  CHARIOX_PERF_DIAGNOSTICS: tracingInterrupts ? '1' : '0',
  CHARIOX_PERF_DIAGNOSTICS_SAMPLE_RATE: '1',
  CHARIOX_DAEMON_SOCKET: path.join(state, 'kernel.sock'),
  TERM: 'xterm-256color', COLORTERM: 'truecolor' }
for (const key of ['CODEX_HOME', 'CLAUDE_CONFIG_DIR', 'OPENCODE_CONFIG_DIR', 'CHARIOX_RELAY_URL', 'CHARIOX_RELAY_TOKEN', 'CHARIOX_CLOUD_PROFILE', 'CHARIOX_CLOUD_TOKEN', 'CHARIOX_CLOUD_RELAY_CONFIG_JSON', 'CHARIOX_CLOUD_RELAY_CONFIG_PATH']) delete env[key]
// Operator configuration for an isolated loopback relay; kernel/client identities
// and terminal enrollment are generated through the normal product pairing flow.
if (relayTransport) env.CHARIOX_RELAY_TOKEN = randomBytes(32).toString('hex')
const receipt = { mpItems: ['MP-08','MP-10','MP-11'], source: args.source, clientSource: args['client-source'], model,
  drillSource: args['drill-source'] ?? args.source,
  ownedStateRoot: state, ownedWorkspaceRoot: workspace,
  startedAt: new Date().toISOString(), steps: [], resources: [], transport: relayTransport ? 'paired-relay' : 'local', limitations: ['Web room-pane and fresh Path-1 comparison require their own real-path evidence.'] }
for (const key of ['kernel','relay','client']) receipt[key + 'Sha256'] = createHash('sha256').update(await readFile(args[key])).digest('hex')
const bundle = createHash('sha256')
for (const name of (await readdir(path.dirname(args.client))).filter(name=>name.endsWith('.js')).sort()) {
  bundle.update(name+'\0');bundle.update(await readFile(path.join(path.dirname(args.client),name)))
}
receipt.clientBundleSha256=bundle.digest('hex')
const resources = async () => {
  const memory = await readFile('/proc/meminfo','utf8')
  const kib = Number(memory.match(/^MemAvailable:\s+(\d+)/m)[1])
  const disk = await statfs('/')
  const diskAvailableBytes = disk.bavail * disk.bsize
  receipt.resources.push({ at: new Date().toISOString(), memAvailableKiB: kib, diskAvailableBytes })
  assert.ok(kib >= 9 * 1024 * 1024, 'MP-11 memory floor')
  assert.ok(diskAvailableBytes >= 10 * 1024**3, 'MP-11 disk floor')
}
await resources()
let resourceFailure
const resourceMonitor = setInterval(()=>resources().catch(error=>{resourceFailure=error}),5000)
let kernel, relay, terminal, client, sessionId, noiseRoot
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
    if(resourceFailure)throw resourceFailure
    result = await terminalCommand('capture',{prefix:path.join(args.output,step),drainSeconds:tracingInterrupts ? 0.01 : 0.2})
    if (predicate(result.text)) { receipt.steps.push({step, status:'GREEN'}); return result.text }
    assert.equal(result.exitCode, null, `MP-08 TUI exited at ${step}`)
    await sleep(tracingInterrupts ? 25 : 250)
  } while (Date.now()<deadline)
  throw new Error(`MP-08 TUI assertion failed at ${step}`)
}
const key = bytes => terminalCommand('key',{bytes:Buffer.from(bytes).toString('base64')})
const raceKey = bytes => terminalCommand('key',{bytes:Buffer.from(bytes).toString('base64'),drainSeconds:0.01})
let lastDiagnostics=0
async function diagnostics(result) {
  const records=[]
  for(const name of await readdir(path.join(state,'logs')).catch(()=>[])) {
    if(!name.endsWith('.ndjson'))continue
    const lines=(await readFile(path.join(state,'logs',name),'utf8')).trim().split('\n').slice(-100)
    for(const line of lines) {
      let entry;try{entry=JSON.parse(line)}catch{continue}
      if(!['warn','error'].includes(entry.level) && !['daemon.app_events','daemon.prompt_queue'].includes(entry.component))continue
      const error=String(entry.error??'')
      records.push({component:entry.component,message:entry.message,...(entry.component==='daemon.prompt_queue'?{sessionId:entry.session_id,agentId:entry.agent_id,promptId:entry.prompt_id,providerRunId:entry.provider_run_id,startingObserved:entry.starting_observed,currentProviderState:entry.current_provider_state}:{}),error:error.includes('is not authenticated;')?'provider account lacks successful product auth observation':/token|credential|bearer|secret|passphrase|auth/i.test(error)?'[credential-related diagnostic suppressed]':error})
    }
  }
  await writeFile(path.join(args.output,'runtime-pump.json'),JSON.stringify({mpItems:['MP-08','MP-11'],messages:records,
    homeKernelId:result.room_workflows?.home_kernel_id,hostDaemonId:result.session.host_daemon_id,
    activity:result.agent_activity,prompts:Object.entries(result.session.prompt_states??{}).map(([agentId, state])=>({agentId,active:state.active_prompt?{id:state.active_prompt.id,status:state.active_prompt.status,workflowRunId:state.active_prompt.workflow_run_id}:null,queued:(state.queued_prompts??[]).map(prompt=>({id:prompt.id,status:prompt.status,workflowRunId:prompt.workflow_run_id}))})),runs:result.session.workflow_runs?.map(run=>({id:run.id,status:run.status,nodes:run.node_runs?.map(node=>({id:node.id,status:node.status,agentId:node.agent_id}))}))},null,2)+'\n',{mode:0o600})
  // MP-08 / MP-10: retain public queue/counter projections, never URLs,
  // provider diagnostics, prompts, credentials or the whole health response.
  const health=unwrap(await client.send(requests.getDaemonHealthRequest()),'DaemonHealth').projection
  await writeFile(path.join(args.output,'runtime-health.json'),JSON.stringify({mpItems:receipt.mpItems,
    at:Date.now(),sessionLanes:health.session_command_lanes,agentLanes:health.agent_command_lanes,
    workflowLanes:health.workflow_command_lanes,providerLanes:health.provider_runtime_lanes,
    providerActor:health.provider_run_actor,sessionProjection:health.session_projection,
    agentProjection:health.agent_runtime_projection,capabilities:health.capability_executor,
    transport:{activeConnections:health.transport.active_connections,
      outgoingQueueOverflows:health.transport.outgoing_queue_overflows,
      slowConsumerCloses:health.transport.slow_consumer_closes,
      overloadRejections:health.transport.inbound_overload_rejections},
    terminalStream:health.terminal_stream},null,2)+'\n',{mode:0o600})
}
async function stateUntil(predicate, timeout=60000) {
  const deadline = Date.now()+timeout
  do { if(resourceFailure)throw resourceFailure; const result=unwrap(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState'); if(predicate(result))return result; if(Date.now()-lastDiagnostics>5000){lastDiagnostics=Date.now();await diagnostics(result)} await sleep(100) }while(Date.now()<deadline)
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
  let connectionArgs = ['--kernel-url',`ws://127.0.0.1:${ports[0]}`,'--client-id','relost-tui']
  let terminalEnv = env
  if (relayTransport) {
    unwrap(await client.send(requests.configureRelayRequest(`ws://127.0.0.1:${ports[4]}`,env.CHARIOX_RELAY_TOKEN)), 'RelayConfigured')
    let status
    for(let attempt=0;;attempt++) {
      status=unwrap(await client.send(requests.relayStatusRequest()), 'RelayStatus').status
      if(status.connected)break
      assert.ok(attempt<100,'MP-08 relay target must be heartbeat-fresh and connected')
      await sleep(100)
    }
    const pairing=unwrap(await client.send(requests.createTerminalPairingLinkRequest('cli','Wfpause relay TUI',300000)), 'TerminalPairingLinkCreated').pairing
    connectionArgs=['--terminal-pairing-link',pairing.pairing_link]
    terminalEnv={...env,HOME:path.join(state,'tui-user'),CHARIOX_HOME:path.join(state,'tui'),
      XDG_CONFIG_HOME:path.join(state,'tui-config'),XDG_STATE_HOME:path.join(state,'tui-state'),
      XDG_DATA_HOME:path.join(state,'tui-data'),XDG_CACHE_HOME:path.join(state,'tui-cache')}
    delete terminalEnv.CHARIOX_RELAY_TOKEN
    receipt.relay={connected:status.connected,targetDaemonId:status.daemon_id,terminalId:pairing.terminal_id}
  }
  const startTerminal = () => {
    buffer=''
    terminal=spawnOwned('python3',[path.join(repo,'apps/cli/scripts/lib/room-workflows-tui-pty.py'),path.join(repo,'apps/kernel/slice-linux-docker'),
      'bun',args.client,...connectionArgs,'--session',sessionId,'--workspace',workspace,'--worktree',workspace,
      '--provider','codex','--model',model,'--account-profile',profile.profile_id],{cwd:repo,env:terminalEnv,stdio:['pipe','pipe','pipe'],detached:true})
    terminal.stdout.on('data', chunk=> {buffer+=chunk;while(buffer.includes('\n')){const i=buffer.indexOf('\n');const message=JSON.parse(buffer.slice(0,i));buffer=buffer.slice(i+1);const waiter=pending.get(message.id);pending.delete(message.id);if(message.error)waiter?.reject(new Error(message.error));else waiter?.resolve(message.result)}})
    terminal.stderr.on('data', chunk=> { /* Only fixture diagnostics; no provider stdout is connected here. */ receipt.terminalError=String(chunk).slice(-1000) })
    terminal.on('close',()=>{for(const waiter of pending.values())waiter.reject(new Error('MP-08 terminal driver exited'));pending.clear()})
  }
  startTerminal()
  await capture('01-room-inventory',text=>text.includes(heading))
  if (busyRefusalRounds) {
    noiseRoot=await mkdtemp('/tmp/wfpause-busy-');receipt.ownedNoiseRoot=noiseRoot
    for(let round=0;round<busyRefusalRounds;round++) {
      const label=`busy-${String(round+1).padStart(2,'0')}`
      const marker=path.join(noiseRoot,label+'-running')
      await key('\x17')
      await key(`Use exec_command to run printf ready > '${marker}'; sleep 120. Wait for the command before returning the workflow envelope.`)
      await key('\r')
      const deadline=Date.now()+60000
      while(!(await readFile(marker,'utf8').catch(()=>''))) {assert.ok(Date.now()<deadline,'MP-08 busy guard needs a real running provider command');await sleep(100)}
      await capture(label+'-running',text=>text.includes('1 running')&&text.includes('[Start · Enter]'))
      await raceKey('\x10\x13')
      await capture(label+'-refused',text=>text.includes('refused')&&text.includes('busy'))
      await stateUntil(s=>s.room_workflows.workflows[0].paused_count===1)
      await capture(label+'-refusal-retained',text=>text.includes('refused')&&text.includes('busy')&&text.includes('[Start · Enter]'))
      await key('\x13')
      await stateUntil(s=>s.room_workflows.workflows[0].paused_count===0&&s.room_workflows.workflows[0].running_count===0)
      await capture(label+'-retry-stopped',text=>text.includes('0 running')&&text.includes('[Start · Enter]'))
      await key('\t')
    }
  }
  const queuedFollowupRounds=Number(args['queued-followup-rounds']??0)
  assert.ok(Number.isSafeInteger(queuedFollowupRounds)&&queuedFollowupRounds>=0&&queuedFollowupRounds<=20)
  if (queuedFollowupRounds) {
    if(noiseRoot)await rm(noiseRoot,{recursive:true,force:true})
    noiseRoot=await mkdtemp('/tmp/wfpause-queue-');receipt.ownedNoiseRoot=noiseRoot
    for(let round=0;round<queuedFollowupRounds;round++) {
      const label=`queue-${String(round+1).padStart(2,'0')}`
      const marker=path.join(noiseRoot,label+'-started')
      const followup=path.join(noiseRoot,label+'-followup')
      await key('\x17')
      await key(`Use exec_command to run printf ready > '${marker}'; sleep 120. Wait for the command before returning the workflow envelope.`)
      await key('\r')
      const deadline=Date.now()+60000
      while(!(await readFile(marker,'utf8').catch(()=>''))) {assert.ok(Date.now()<deadline,'MP-08 queued-start command must reach real provider');await sleep(100)}
      await capture(label+'-running',text=>text.includes('1 running'))
      await key(`Use exec_command to run printf followed > '${followup}', then return the successful workflow envelope with output message QUEUE_FOLLOWUP_OK.`)
      await key('\r')
      await capture(label+'-queued',text=>text.includes('Queued request'))
      await raceKey('\x13')
      const nextDeadline=Date.now()+90000
      while(!(await readFile(followup,'utf8').catch(()=>''))) {assert.ok(Date.now()<nextDeadline,'MP-08 queued follow-up must run after Stop');await sleep(250)}
      await stateUntil(s=>s.room_workflows.workflows[0].running_count===0&&s.room_workflows.workflows[0].queued_count===0,90000)
      await capture(label+'-followup-completed',text=>text.includes('0 running')&&text.includes('[Start · Enter]'))
      await key('\t')
    }
  }
  if (noisyInterruptRounds) {
    if(noiseRoot)await rm(noiseRoot,{recursive:true,force:true})
    noiseRoot = await mkdtemp('/tmp/wfpause-noisy-')
    receipt.ownedNoiseRoot = noiseRoot
    receipt.noisyInterrupts = []
    receipt.noisyProducerFailures = []
    receipt.noisyControlFailures = []
    for (let round = 0; round < noisyInterruptRounds; round++) {
      const label = `noisy-${String(round + 1).padStart(2,'0')}`
      const action = round % 2 ? 'stop' : 'pause'
      const marker = `${label}.progress`
      const command = `printf '%s\\n' "$BASHPID" > '${path.join(noiseRoot,marker)}.pid'; payload=$(printf '%4096s' x); i=0; end=$((SECONDS+120)); while [ "$SECONDS" -lt "$end" ]; do printf 'WFP_NOISY_OUTPUT %s %s\\n' "$i" "$payload"; i=$((i+1)); if [ $((i%1024)) -eq 0 ]; then printf '%s\\n' "$i" > '${path.join(noiseRoot,marker)}'; fi; done`
      const previous = new Set((await stateUntil(()=>true)).room_workflows.workflows[0].runs.map(run=>run.run_id))
      await key('\x17')
      await key(`Run this exact Bash command now using exec_command with yield_time_ms=30000 as one foreground shell tool call, without a pipe, background session, or output truncation: ${command}. Keep waiting for this command; do not return a workflow envelope until it ends.`)
      // The narrow composer scrolls to the end of this long user command.
      await capture(label+'-draft',text=>text.includes('ends.'))
      const noiseStartedAtMs = Date.now()
      await raceKey('\r')
      const started = await stateUntil(s=>s.room_workflows.workflows[0].runs.some(run=>!previous.has(run.run_id)))
      const runId = started.room_workflows.workflows[0].runs.find(run=>!previous.has(run.run_id)).run_id
      const noiseDeadline = Date.now()+90000
      let progress = 0, output = []
      do {
        if(resourceFailure)throw resourceFailure
        progress = Number(await readFile(path.join(noiseRoot,marker),'utf8').catch(()=>0))
        output = (await interruptTraces(path.join(state,'logs'))).filter(trace=>trace.message==='codex command output received trace' && trace.at>=noiseStartedAtMs)
        if(progress >= 8192 && output.length)break
        await sleep(100)
      } while(Date.now()<noiseDeadline)
      await diagnostics(await stateUntil(()=>true))
      assert.ok(progress >= 8192 && output.length, 'MP-08 real noisy command must be running and emitting provider socket deltas')
      await capture(label+'-command-running',text=>text.includes('1 running')&&text.includes('[Start · Enter]'))
      if (reconnectRounds.has(round+1) && !relayTransport) {
        // MP-08 / MP-11: reconnect the owned real TUI, leaving its provider run alive.
        const closed = new Promise(resolve=>terminal.once('close',resolve))
        terminal.stdin.write(JSON.stringify({id:++nextId,action:'close'})+'\n')
        await Promise.race([closed,sleep(10000).then(()=>{throw new Error('MP-11 owned TUI did not close for reconnect')})])
        receipt.steps.push({step:label+'-local-disconnected',status:'GREEN'})
        startTerminal()
        await capture(label+'-local-reattached',text=>text.includes(heading)&&text.includes('1 running'),30000)
        await key('\x17')
        await capture(label+'-local-reconnected',text=>text.includes('1 running')&&text.includes('[Start · Enter]'))
      }
      if (reconnectRounds.has(round+1) && relayTransport) {
        // MP-08/MP-11: interrupt only the exact relay owned by this run.
        const exited = new Promise(resolve=>relay.once('exit',resolve))
        signalOwnedProcess(relay,'SIGTERM'); await exited
        await capture(label+'-disconnected',text=>text.includes('Offline') || text.includes('Disconnected'))
        if (args['control-while-offline']==='1') {
          await raceKey(action === 'pause' ? '\x10' : '\x13')
          await capture(label+'-offline-control-feedback',text=>text.includes('Refreshing workflow state') || text.includes('failed') || text.includes('refused') || text.includes('connect') && !text.includes('Started run'),2000)
        }
        relay=spawnOwned(args.relay,[],{cwd:repo,env,stdio:'ignore',detached:true})
        await capture(label+'-reconnected',text=>text.includes('Reconnected')&&text.includes('1 running'),30000)
        receipt.steps.push({step:label+'-relay-reconnect',status:'GREEN'})
      }
      // User action is a real TUI key. Time starts at the PTY write, before
      // the TUI/relay/kernel process the control; it does not start at an IPC call.
      // Sample the producer concurrently with the control path: waiting for
      // state/capture first would misattribute their latency to process exit.
      const commandPid=Number(await readFile(path.join(noiseRoot,marker)+'.pid','utf8'))
      assert.ok(Number.isSafeInteger(commandPid)&&commandPid>1,'MP-11 noisy command must report its own valid PID')
      const processState=async()=>{
        const raw=await readFile(`/proc/${commandPid}/stat`,'utf8').catch(()=>null)
        if(raw===null)return null
        const fields=raw.slice(raw.lastIndexOf(')')+2).split(' ')
        return {state:fields[0],startTicks:fields[19]}
      }
      const originalProcess=await processState()
      assert.ok(originalProcess&&originalProcess.state!=='Z','MP-08 noisy command must be a live OS process before control')
      await terminalCommand('watch',{pid:commandPid,progressPath:path.join(noiseRoot,marker),startTicks:originalProcess.startTicks})
      const control = await raceKey(action === 'pause' ? '\x10' : '\x13')
      const sample = {round:round+1,action,runId,command,commandPid,controlSentAtMs:control.sentAtMs,progressBeforeControl:progress}
      receipt.noisyInterrupts.push(sample)
      const deadline = Date.now()+10000
      let traces, timing
      do {
        traces = await interruptTraces(path.join(state,'logs'))
        timing = interruptTiming(traces,control.sentAtMs)
        if(timing.sent && timing.ended)break
        await sleep(25)
      } while(Date.now()<deadline)
      Object.assign(sample,timing)
      await writeFile(path.join(args.output,label+'-timing.json'),JSON.stringify({mpItems:receipt.mpItems,...sample},null,2)+'\n',{mode:0o600})
      assert.ok(timing.sent, 'MP-08 noisy cancellation must actually send turn/interrupt')
      if(timing.stopToInterruptSentMs>1000)receipt.noisyControlFailures.push({round:round+1,action,
        seam:'interrupt sent after one-second budget',stopToInterruptSentMs:timing.stopToInterruptSentMs})
      const settled = await stateUntil(s=>Object.values(s.agent_activity).every(activity=>!activity.active_prompt_count),10000)
      // Recompute after kernel settlement: an initial stale-ID completion
      // must not hide the interrupt/completion of the actual running turn.
      traces = await interruptTraces(path.join(state,'logs'))
      timing = interruptTiming(traces,control.sentAtMs)
      Object.assign(sample,timing,{outputDeltasBeforeControl:traces.filter(trace=>trace.message==='codex command output received trace'
        && trace.at>=noiseStartedAtMs && trace.at<control.sentAtMs && (!timing.sent || trace.providerRunId===timing.sent.providerRunId)).length})
      await writeFile(path.join(args.output,label+'-timing.json'),JSON.stringify({mpItems:receipt.mpItems,...sample},null,2)+'\n',{mode:0o600})
      assert.ok(timing.ended, 'MP-08 noisy cancellation must receive provider turn/completed')
      if(timing.stopToTurnEndedMs>5000)receipt.noisyControlFailures.push({round:round+1,action,
        seam:'provider turn ended after five-second budget',stopToTurnEndedMs:timing.stopToTurnEndedMs})
      await diagnostics(settled)
      const run=unwrap(await client.send(requests.getWorkflowRunRequest(sessionId,runId)), 'WorkflowRun').workflow_run
      sample.status=run.status
      assert.equal(run.status.toLowerCase(),action === 'pause' ? 'paused' : 'stopped')
      await capture(label+'-settled',text=>text.includes('0 running')&&text.includes('[Start · Enter]'))
      // MP-08: require an unchanged producer for 500ms after the 1s cutoff.
      await sleep(Math.max(0,control.sentAtMs+1600-Date.now()))
      const progressSamples=(await terminalCommand('finishWatch')).samples
      sample.progressSamples=progressSamples
      // Output producers must have stopped, not merely their visible cards.
      let after = await readFile(path.join(noiseRoot,marker),'utf8')
      let unchangedSince = Date.now()
      do {
        await sleep(100)
        const current = await readFile(path.join(noiseRoot,marker),'utf8')
        if(current !== after) { after=current; unchangedSince=Date.now() }
        if(Date.now()-unchangedSince>=500)break
      } while(Date.now()<control.sentAtMs+5000)
      const firstAfterBudget=progressSamples.find(entry=>entry.at>=control.sentAtMs+1000)
      const tail=progressSamples.filter(entry=>entry.at>=control.sentAtMs+1000)
      sample.producerStopped=Boolean(firstAfterBudget && tail.at(-1).at-firstAfterBudget.at>=500
        && tail.every(entry=>entry.value===firstAfterBudget.value))
      let lastChange=progressSamples[0]?.at ?? control.sentAtMs
      for(let i=1;i<progressSamples.length;i++)if(progressSamples[i].value!==progressSamples[i-1].value)lastChange=progressSamples[i].at
      const processEnded=progressSamples.find(entry=>entry.at>=control.sentAtMs&&!entry.processAlive)
      sample.stopToCommandExitedMs=processEnded ? processEnded.at-control.sentAtMs : null
      sample.commandExitedWithinBudget=Boolean(processEnded && sample.stopToCommandExitedMs<=1000)
      const finalOutput=(await interruptTraces(path.join(state,'logs'))).filter(trace=>
        trace.message==='codex command output received trace' && trace.providerRunId===timing.sent.providerRunId
        && trace.at>=control.sentAtMs).at(-1)
      sample.stopToLastOutputReceivedMs=finalOutput ? finalOutput.at-control.sentAtMs : 0
      sample.outputStoppedWithinBudget=sample.stopToLastOutputReceivedMs<=1000
      sample.producerStopped &&= lastChange-control.sentAtMs<=1000 && sample.commandExitedWithinBudget
        && sample.outputStoppedWithinBudget
      sample.stopToProducerStoppedMs=sample.producerStopped ? lastChange-control.sentAtMs : null
      await writeFile(path.join(args.output,label+'-timing.json'),JSON.stringify({mpItems:receipt.mpItems,...sample},null,2)+'\n',{mode:0o600})
      if(!sample.producerStopped)receipt.noisyProducerFailures.push({round:round+1,action,
        seam:!sample.commandExitedWithinBudget?'command survived one-second cancellation budget':!sample.outputStoppedWithinBudget?'buffered provider output received after one-second cancellation budget':'producer progress advanced after one-second cancellation budget',
        stopToInterruptSentMs:sample.stopToInterruptSentMs,stopToTurnEndedMs:sample.stopToTurnEndedMs,
        stopToCommandExitedMs:sample.stopToCommandExitedMs,stopToLastOutputReceivedMs:sample.stopToLastOutputReceivedMs})
      if(action === 'pause') {
        await key('\x13')
        await stateUntil(s=>s.room_workflows.workflows[0].paused_count===0)
      }
      await key('\t')
    }
  }
  if (interruptRaceRounds) {
    receipt.interruptRace = []
    for (let round = 0; round < interruptRaceRounds; round++) {
      const label = `race-${String(round + 1).padStart(2,'0')}`
      const previous = new Set((await stateUntil(()=>true)).room_workflows.workflows[0].runs.map(run=>run.run_id))
      await key('\x17')
      await key('Run sleep 30 in the shell, then return the required workflow JSON envelope.')
      const variation = round % 3 === 0 ? 'pause-resume-stop' : round % 3 === 1 ? 'pause-resume-pause' : 'resume-during-start'
      await capture(label+'-draft',text=>text.includes('Run sleep 30 in the shell'))
      await (round % 3 === 2 ? raceKey : key)('\r')
      const started = await stateUntil(s=>s.room_workflows.workflows[0].runs.some(run=>!previous.has(run.run_id)))
      const runId = started.room_workflows.workflows[0].runs.find(run=>!previous.has(run.run_id)).run_id
      // The first two rows interrupt an admitted provider turn; the third
      // deliberately controls the run while provider submission is starting.
      if (round % 3 !== 2) await stateUntil(s=>Object.values(s.agent_activity).some(activity=>activity.active_turn?.status === 'running'))
      await capture(label+'-started',text=>text.includes('1 running')&&text.includes('[Start · Enter]'))
      await key('\x10')
      await stateUntil(s=>s.room_workflows.workflows[0].paused_count===1)
      await capture(label+'-paused',text=>text.includes('1 paused')&&text.includes('[Start · Enter]'))
      await raceKey('\x12')
      const resumed = await stateUntil(s=>s.room_workflows.workflows[0].running_count===1)
      // The pane serializes submitted controls. Wait for its visible RPC
      // acknowledgement so the next key is a real user action, not ignored.
      await capture(label+'-resumed',text=>text.includes('1 running')&&text.includes('[Start · Enter]'))
      await raceKey(round % 3 === 1 ? '\x10' : '\x13')
      const settled = await stateUntil(s=>{
        const run = s.session.workflow_runs.find(run=>run.id===runId)
        return run && ['paused','stopped','failed','completed'].includes(run.status.toLowerCase())
          && Object.values(s.agent_activity).every(activity=>!activity.active_prompt_count)
      })
      await diagnostics(settled)
      const run = unwrap(await client.send(requests.getWorkflowRunRequest(sessionId,runId)), 'WorkflowRun').workflow_run
      receipt.interruptRace.push({round:round+1,variation,runId,status:run.status,
        startActivity:Object.values(started.agent_activity).map(activity=>({promptCount:activity.active_prompt_count,turnStatus:activity.active_turn?.status})),
        resumeActivity:Object.values(resumed.agent_activity).map(activity=>({promptCount:activity.active_prompt_count,turnStatus:activity.active_turn?.status})),nodes:run.node_runs.map(node=>({id:node.id,status:node.status})),
        failureEvents:(run.failure_events??[]).map(event=>({kind:event.kind,message:/token|credential|bearer|secret|passphrase|auth/i.test(event.message)?'[credential-related diagnostic suppressed]':event.message}))})
      await capture(label+'-settled',text=>text.includes('0 running')&&text.includes('[Start · Enter]'))
      if (run.status.toLowerCase()==='failed' && args['expect-red']==='1')receipt.expectedRed=true
      else assert.equal(run.status.toLowerCase(), round % 3 === 1 ? 'paused' : 'stopped', `MP-08 ${variation} must settle without a Failed workflow`)
      if (run.status.toLowerCase()==='paused') {
        await key('\x13')
        await stateUntil(s=>s.room_workflows.workflows[0].paused_count===0)
      }
      await key('\t')
    }
    if(args['expect-red']==='1')assert.ok(receipt.expectedRed,'MP-08 base must reproduce an interrupt race')
  }
  await key('\x17') // Ctrl+W
  await key('Run sleep 30; reply HOLD.')
  await capture('02-independent-draft',text=>text.includes('Run sleep 30; reply HOLD.'))
  await key('\x1b')
  await capture('03-dismissed',text=>!text.includes('Start '+heading))
  await key('\x17')
  await capture('04-reopened-draft',text=>text.includes('Run sleep 30; reply HOLD.')&&text.includes('Start '+heading))
  const normalPrevious = new Set((await stateUntil(()=>true)).room_workflows.workflows[0].runs.map(run=>run.run_id))
  await key('\r')
  const started = await stateUntil(s=>s.room_workflows.workflows[0].runs.some(run=>!normalPrevious.has(run.run_id)&&run.status.toLowerCase()==='running'))
  receipt.runId=started.room_workflows.workflows[0].runs.find(run=>!normalPrevious.has(run.run_id)).run_id
  await capture('05-start-confirmed',text=>text.includes('Started run')&&text.includes('[Start · Enter]')&&text.includes('1 running'))
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
  await stateUntil(s=>s.room_workflows.workflows[0].running_count===0,180000)
  const finalRun=unwrap(await client.send(requests.getWorkflowRunRequest(sessionId,completionRunId)), 'WorkflowRun').workflow_run
  receipt.finalRun={runId:finalRun.id,status:finalRun.status}
  assert.equal(finalRun.status.toLowerCase(),'completed','MP-08 workflow must complete successfully, not merely stop')
  await stateUntil(s=>Object.values(s.agent_activity).every(activity=>!activity.active_prompt_count))
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
  let completionVisible = false
  for(let focusAttempt=0;focusAttempt<6;focusAttempt++) {
    await key('\t') // Cycle the actual user focus order, including the sidebar.
    const step=`09-provider-completion-focus-${focusAttempt+1}`
    try { await capture(step,text=>text.includes('RELOST_WORKFLOW_OK'),1500); completionVisible=true; break }
    catch(error) { if(error.message!==`MP-08 TUI assertion failed at ${step}`)throw error }
  }
  assert.ok(completionVisible,'MP-08 provider completion must be visible in its real TUI agent pane')
  // Finish the other requested controls and provider completion, but retain
  // a RED acceptance result if any shell outlived its cancellation budget.
  const latencies=(receipt.noisyInterrupts??[]).map(sample=>sample.stopToInterruptSentMs).filter(Number.isFinite).sort((a,b)=>a-b)
  receipt.sendLatency={samples:latencies.length,p95Ms:latencies[Math.ceil(latencies.length*0.95)-1],maxMs:latencies.at(-1),budgetMs:1000}
  assert.equal(receipt.noisyProducerFailures?.length ?? 0,0,'MP-08 noisy shell output continued beyond one-second cancellation budget')
  assert.equal(receipt.noisyControlFailures?.length ?? 0,0,'MP-08 noisy control exceeded interrupt or turn-completion budget')
  receipt.status=receipt.expectedRed?'RED':'GREEN'
} catch(error) {
  receipt.status='RED';receipt.firstFailure=error.message
  if(sessionId)await diagnostics(await stateUntil(()=>true)).catch(()=>{})
  if(terminal&&terminal.exitCode===null)await terminalCommand('capture',{prefix:path.join(args.output,'failure')}).catch(()=>{})
  if (args['expect-red']==='1' && error.message==='MP-08 TUI assertion failed at 01-room-inventory')receipt.expectedRed=true
  else process.exitCode=1
} finally {
  clearInterval(resourceMonitor)
  if(tracingInterrupts) {
    const traces=[],queueTraces=[],guardTraces=[]
    for(const name of await readdir(path.join(state,'logs')).catch(()=>[])) {
      if(!name.endsWith('.ndjson'))continue
      for(const line of (await readFile(path.join(state,'logs',name),'utf8')).split('\n')) {
        let entry;try{entry=JSON.parse(line)}catch{continue}
        if(entry.component==='cli.room_workflows')guardTraces.push({at:entry.timestamp_ms,guard:entry.guard})
        if(entry.component==='daemon.command_latency' && !['session.state.get','daemon.health.get'].includes(entry.command_type)) {
          queueTraces.push({at:entry.timestamp_ms,message:entry.message,commandId:entry.command_id,
            commandType:entry.command_type,laneKind:entry.lane_kind,laneId:entry.lane_id,
            status:entry.status,queueWaitMs:entry.queue_wait_ms,executionMs:entry.lane_execution_ms})
        }
        if(entry.component!=='daemon.provider.codex' || !/turn (start|completion|interrupt)/.test(entry.message??''))continue
        traces.push({at:entry.timestamp_ms,message:entry.message,providerRunId:entry.provider_run_id,
          activeTurnId:entry.active_turn_id,previousTurnId:entry.previous_active_turn_id,turnId:entry.turn_id,
          responseTurnId:entry.response?.turn?.id})
      }
    }
    await writeFile(path.join(args.output,'provider-turns.json'),JSON.stringify({mpItems:receipt.mpItems,traces,
      interruptEvidence:await interruptTraces(path.join(state,'logs'))},null,2)+'\n',{mode:0o600})
    await writeFile(path.join(args.output,'command-queues.json'),JSON.stringify({mpItems:receipt.mpItems,
      guardTraces,traces:queueTraces.sort((a,b)=>a.at-b.at)},null,2)+'\n',{mode:0o600})
  }
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
  if(noiseRoot)await rm(noiseRoot,{recursive:true,force:true})
  assert.ok([terminal,kernel,relay].filter(Boolean).every(child=>child.exitCode!==null||child.signalCode!==null),'MP-11 owned process cleanup failed')
  receipt.cleanup='owned session ended, TUI/kernel/relay stopped, disposable state removed; linked account left in place'
  await resources()
  receipt.finishedAt=new Date().toISOString()
  await writeFile(path.join(args.output,'result.json'),JSON.stringify(receipt,null,2)+'\n',{mode:0o600})
  console.log(JSON.stringify({mpItems:receipt.mpItems,status:receipt.status,expectedRed:receipt.expectedRed,firstFailure:receipt.firstFailure,output:args.output}))
}

#!/usr/bin/env node
// MP-07/MP-08/MP-10/MP-11: owned local repetition evidence; no fresh-machine closure.
import assert from 'node:assert/strict'
import {spawn,execFile} from 'node:child_process'
import {randomUUID,createHash} from 'node:crypto'
import {createWriteStream,constants as fsConstants} from 'node:fs'
import {readFile,writeFile,appendFile,mkdir,mkdtemp,readdir,rm,stat,chmod,open} from 'node:fs/promises'
import http from 'node:http'
import net from 'node:net'
import path from 'node:path'
import {fileURLToPath,pathToFileURL} from 'node:url'
import {promisify} from 'node:util'
import {captureRoomRepetitionFailure} from './lib/room-repetition-failure-evidence.mjs'
import {readForeignProxyOwnership} from './lib/room-repetition-listener-ownership.mjs'
import {roomDrillRelayToken} from './lib/room-drill-relay-token.mjs'
import {roomTuiPtyInvocation} from './lib/room-tui-pty.mjs'
import {readAutomationSnapshot,waitForAutomationRoom,stopProcessGroup,sleep,tuiEnvironment} from './lib/room-coordinated-load-tui-process.mjs'
import {startBrowserComputerFixture} from './lib/browser-computer-fixture.mjs'
import {startBrowserStateFixtureProxy,browserStateFixtureGateway} from './lib/browser-state-fixture-proxy.mjs'
import {compareCycle,assertSameRoom,detachOwnedAttachment,classifyOwnedListeners,assertRestoredMachineIdentity} from './lib/room-repetition-gates.mjs'
const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../../..')
const clientRoot=process.env.LOOPS_CLIENT_ROOT,release=process.env.LOOPS_RELEASE_ROOT,cloud=process.env.LOOPS_CLOUD_ROOT
const evidence=process.env.LOOPS_EVIDENCE_ROOT,lane=process.env.LOOPS_STATE_ROOT
for(const p of [clientRoot,release,cloud,evidence,lane])assert.ok(p&&path.isAbsolute(p),'MP-10 roots must be absolute')
assert.ok(['all','reconnect','persistence','create-delete'].includes(process.env.LOOPS_PHASE??'all'),'MP-10 invalid phase')
const image=process.env.LOOPS_SLICE_IMAGE??'sha256:a282bac1efb1be1d34037ca068b7a9fe14575fac25381bd1ec956c9cadb299fc'
const {LocalIpcClient}=await import(pathToFileURL(`${clientRoot}/packages/kernel-client/dist/ipc.js`))
const r=await import(pathToFileURL(`${clientRoot}/packages/kernel-client/dist/ipc-requests.js`))
const exec=promisify(execFile),runId=`loops-${Date.now()}-${process.pid}`,mpItems=['MP-07','MP-08','MP-10','MP-11']
await mkdir(evidence,{recursive:true,mode:0o700});await mkdir(lane,{recursive:true,mode:0o700})
const root=await mkdtemp(`${lane}/runtime-`),owned=[],slices=[],savedImages=new Set(),observedPids=new Map(),ownedPorts=new Set(),failures=[],gates={}
let local,session,slice,environment,relayChild,kernelChild,fixture,webServer,webContainer,monitor,stage='provenance',interrupted=false,ocrRun=null
const activeTuis=new Map()
// The Web viewer container runs Chromium sandboxed as the slice image's non-root
// user (uid 1001) and gets only its own runtime and evidence-staging directories.
// Both live in the mkdtemp runtime root, which is owner-only, so modes alone give
// uid 1001 access without host chown; the host still reads and removes its files.
// Staged evidence is copied to the evidence root only after the viewer is gone.
const WEB_UID=1001,webRoot=`${root}/web`,webStaging=`${root}/web-evidence`,webEvidence=`${evidence}/web`
async function publishWebEvidence(){
 if(webContainer)return
 const staged=await readdir(webStaging).catch(()=>[])
 if(!staged.length)return
 await mkdir(webEvidence,{recursive:true,mode:0o700})
 // Never follow a staged symlink, even if the viewer could still change the directory.
 for(const name of staged){
  // O_NONBLOCK: a staged FIFO must not block the drill; the isFile check skips it.
  const handle=await open(path.join(webStaging,name),fsConstants.O_RDONLY|fsConstants.O_NOFOLLOW|fsConstants.O_NONBLOCK).catch(()=>null)
  if(!handle)continue
  try{if((await handle.stat()).isFile())await writeFile(path.join(webEvidence,name),await handle.readFile(),{mode:0o600})}finally{await handle.close()}
 }
}
const writeWeb=async(name,value)=>{await writeFile(`${webRoot}/${name}`,JSON.stringify(value));await chmod(`${webRoot}/${name}`,0o644)}
const write=(name,value)=>writeFile(`${evidence}/${name}.json`,JSON.stringify({mpItems,...value},null,2)+'\n',{mode:0o600})
const cmd=async(program,args,timeout=120000)=>{
 const started=Date.now()
 try{const result=await exec(program,args,{timeout,maxBuffer:8*1024**2,env:{...process.env,DOCKER_HOST:'unix:///var/run/docker.sock',DOCKER_CONFIG:`${root}/docker-config`}});await appendFile(`${evidence}/commands.jsonl`,JSON.stringify({mpItems,program,args:program==='docker'&&args.includes('-e')?args.map(a=>a.includes('TOKEN=')?'REDACTED':a):args,exitCode:0,started,elapsedMs:Date.now()-started})+'\n');return result.stdout.trim()}
 catch(e){await appendFile(`${evidence}/commands.jsonl`,JSON.stringify({mpItems,program,args,exitCode:e.code,started,elapsedMs:Date.now()-started})+'\n');throw new Error(`${program} ${args.slice(0,3).join(' ')}: ${e.stderr?.slice(-700)??e.message}`)}
}
const docker=args=>cmd('docker',args)
const unwrap=(v,k)=>{assert.ok(v?.[k],`expected ${k}, got ${Object.keys(v??{})}`);return v[k]}
const send=req=>local.send(req)
const freePort=()=>new Promise(resolve=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})})
const wait=async(fn,label,ms=60000)=>{const end=Date.now()+ms;let last;while(Date.now()<end){if(interrupted)throw Error('MP-10 interrupted/resource guard');try{const value=await fn();if(value)return value}catch(e){if(e.fatal)throw e;last=e}await sleep(200)}throw Error(`${label}: ${last?.message??'timeout'}`)}
const guard=async()=>{const m=await readFile('/proc/meminfo','utf8');const memAvailable=Number(m.match(/MemAvailable:\s+(\d+)/)[1])*1024;const diskAvailable=Number((await exec('df',['-B1','--output=avail','/'])).stdout.trim().split(/\s+/).at(-1));const row={at:new Date().toISOString(),stage,memAvailable,diskAvailable};await appendFile(`${evidence}/resources.jsonl`,JSON.stringify({mpItems,...row})+'\n');if(memAvailable<4*1024**3||diskAvailable<10*1024**3){interrupted=true;throw Error('MP-10 resource reserve breached')}return row}
const launch=(binary,env,name)=>{const child=spawn(binary,[],{env,cwd:root,detached:true,stdio:['ignore','pipe','pipe']});const log=createWriteStream(`${root}/${name}.log`);child.stdout.pipe(log);child.stderr.pipe(log);owned.push(child);return child}
const cname=s=>`chariox-slice-${s.name}`
const screen=(...args)=>docker(['exec','-u','slice',cname(slice),'/opt/chariox-slice/slice-screen.sh',...args])
const proc=async pid=>{try{const text=await readFile(`/proc/${pid}/stat`,'utf8');const f=text.slice(text.lastIndexOf(')')+2).split(' ');return {pid,name:(await readFile(`/proc/${pid}/comm`,'utf8')).trim(),role:(await readFile(`/proc/${pid}/cmdline`,'utf8')).includes('browser-controller')?'browser-controller':null,state:f[0],rss:Number(f[21])*4096,fds:(await readdir(`/proc/${pid}/fd`)).length,startTicks:Number(f[19])}}catch{return null}}
async function inventory(label,ownershipAttempt=0){
 const listeners=(await cmd('ss',['-ltnp'])).split('\n')
 const containers=(await docker(['ps','-a','--format','{{.ID}}|{{.Names}}|{{.Status}}|{{.Ports}}'])).split('\n').filter(Boolean)
 const volumes=(await docker(['volume','ls','--format','{{.Name}}'])).split('\n').filter(Boolean)
 const images=(await docker(['image','ls','--no-trunc','--format','{{.Repository}}:{{.Tag}}|{{.ID}}|{{.Size}}'])).split('\n').filter(Boolean)
 const processes=(await cmd('ps',['-axo','pid=,ppid=,pgid=,comm='])).split('\n')
 const servicePids=[process.pid,kernelChild?.pid,relayChild?.pid].filter(Boolean),descendants=new Set(servicePids)
 const processRows=processes.map(line=>line.trim().split(/\s+/)).filter(row=>row.length>=4)
 let previousSize;do{previousSize=descendants.size;for(const row of processRows)if(descendants.has(Number(row[1])))descendants.add(Number(row[0]))}while(previousSize!==descendants.size)
 const aliveOwnedPids=new Set()
 for(const [pid,startTicks] of observedPids){const p=await proc(pid);if(p&&p.startTicks===startTicks)aliveOwnedPids.add(pid)}
 const foreignHostPids=new Set(processRows.map(row=>Number(row[0])).filter(pid=>!descendants.has(pid)&&!aliveOwnedPids.has(pid)))
 const publicProxyOwnership=await readForeignProxyOwnership(listeners,containers,runId,docker,readFile)
 const listenerOwnership=classifyOwnedListeners(listeners,ownedPorts,containers,runId,servicePids,aliveOwnedPids,foreignHostPids,publicProxyOwnership.confirmedPids,publicProxyOwnership.ceasedPids)
 if(listenerOwnership.unresolved.length&&ownershipAttempt<3){
  await write(`${label}-unresolved-${ownershipAttempt}`,{ownershipAttempt,unresolvedListeners:listenerOwnership.unresolved,publicForeignBindings:publicProxyOwnership.containers})
  await sleep(250);return inventory(label,ownershipAttempt+1)
 }
 const row={ownershipAttempts:ownershipAttempt+1,label,at:new Date().toISOString(),containers:containers.filter(x=>x.includes(runId)),volumes:volumes.filter(x=>x.includes(runId)),images:images.filter(x=>x.includes(runId)),ownedPids:[],listeners:listenerOwnership.owned,reusedForeignListeners:listenerOwnership.reused,publicForeignBindings:publicProxyOwnership.containers,kernel:await proc(kernelChild?.pid)??{rss:0,fds:0},relay:await proc(relayChild?.pid)??{rss:0,fds:0},diskBytes:Number((await cmd('du',['-sb',root])).split(/\s+/)[0]),resources:await guard(),global:{containers,volumes,images,listeners,processes}}
 for(const [pid,startTicks] of observedPids){const p=await proc(pid);if(p&&p.startTicks===startTicks)row.ownedPids.push(`${pid}:${p.state}`)}
 await write(label,row);return row
}
async function observeWorker(){
 const container=JSON.parse(await docker(['inspect',cname(slice)]))[0]
 const publishedPorts=new Set(Object.values(container.HostConfig.PortBindings??{}).flat().map(p=>Number(p.HostPort)))
 for(const port of publishedPorts)ownedPorts.add(port)
 for(const listener of (await cmd('ss',['-ltnp'])).split('\n')){
  const port=Number(listener.trim().split(/\s+/)[3]?.match(/:(\d+)$/)?.[1])
  if(!publishedPorts.has(port)||!listener.includes('docker-proxy'))continue
  for(const match of listener.matchAll(/pid=(\d+),/g)){const pid=Number(match[1]),metric=await proc(pid);if(metric?.name==='docker-proxy')observedPids.set(pid,metric.startTicks)}
 }
 const top=(await docker(['top',cname(slice),'-eo','pid,comm'])).split('\n').slice(1).map(l=>Number(l.trim().split(/\s+/)[0])).filter(Number.isInteger)
 const metrics=[];for(const pid of top){const metric=await proc(pid);if(metric){metrics.push(metric);observedPids.set(pid,metric.startTicks)}}
 return {containerId:container.Id,limits:{memory:container.HostConfig.Memory,swap:container.HostConfig.MemorySwap,nanoCpus:container.HostConfig.NanoCpus},metrics}
}
async function exposeFixture(){
 if(!fixture)return
 const port=Number(new URL(fixture.origin).port)
 const networks=JSON.parse(await docker(['inspect',cname(slice),'--format','{{json .NetworkSettings.Networks}}']))
 const upstreamHost=browserStateFixtureGateway(networks)
 await docker(['exec','-d','-u','slice',cname(slice),'node','--input-type=module','-e',`await (${startBrowserStateFixtureProxy.toString()})(${JSON.stringify({port,upstreamPort:port,upstreamHost})})`])
 await wait(()=>docker(['exec',cname(slice),'curl','--fail','--silent','--max-time','2',`http://127.0.0.1:${port}/state/check`]),'loopback fixture')
}
async function createSlice(index){
 for(let retry=0;retry<=3;retry++){
  slice=unwrap(await send(r.createSliceRequest({name:`${runId}-${index}-${retry}`,displayMode:'headed',displayBackend:'selkies',workspaceMount:`${root}/workspace`,base:'clean'})),'SliceCreated').slice;slices.push(slice)
  await send(r.bindRoomEnvironmentSliceRequest(session.id,slice.id))
  try{unwrap(await send(r.startSliceRequest(slice.id)),'SliceStarted');slice=await wait(async()=>{const s=unwrap(await send(r.getSliceRequest(slice.id)),'Slice').slice;if(s.status==='failed')throw Error(s.last_error??'slice failed');return s.status==='running'?s:false},'slice running',120000);await screen('status');await observeWorker();
   if(process.env.LOOPS_WORKER_KERNEL_HASH){
    const actual=await docker(['exec',cname(slice),'sha256sum','/opt/chariox-slice/bin/chariox-kernel','/opt/chariox-slice/bin/chariox-relay'])
    const hashes=Object.fromEntries(actual.split('\n').map(line=>{const [hash,file]=line.trim().split(/\s+/);return [path.basename(file),hash]}))
    await write(`worker-binaries-${index}-${retry}`,{sliceId:slice.id,hashes})
    assert.equal(hashes['chariox-kernel'],process.env.LOOPS_WORKER_KERNEL_HASH,'MP-10 actual worker kernel hash')
    if(process.env.LOOPS_WORKER_RELAY_HASH)assert.equal(hashes['chariox-relay'],process.env.LOOPS_WORKER_RELAY_HASH,'MP-10 actual worker relay hash')
   }
   await exposeFixture();return}
  catch(error){if(!/host port.*already in use|port is already allocated|address already in use/i.test(error.message)||retry===3)throw error;await write(`port-race-${index}-${retry}`,{index,retry,error:error.message,sliceId:slice.id});await deleteSlice(slice)}
 }
}
async function restoreSavedWithPortRetry(saved,cycle){
 for(let retry=0;retry<=3;retry++){
  try{await send(r.startSliceRequest(slice.id));slice=await wait(async()=>{const s=unwrap(await send(r.getSliceRequest(slice.id)),'Slice').slice;return s.status==='running'?s:false},'restore running',120000);return retry}
  catch(error){
   if(!/host port.*already in use|port is already allocated|address already in use/i.test(error.message)||retry===3)throw error
   await write(`port-race-restore-${cycle}-${retry}`,{cycle,retry,error:error.message,oldSliceId:slice.id,savedStateId:saved.state.id})
   await deleteSlice(slice)
   slice=unwrap(await send(r.createSliceRequest({name:`${runId}-restore-${cycle}-${retry+1}`,displayMode:'headed',displayBackend:'selkies',workspaceMount:`${root}/workspace`,base:'clean',fromSavedState:saved.state.id})),'SliceCreated').slice
   slices.push(slice);await send(r.bindRoomEnvironmentSliceRequest(session.id,slice.id))
  }
 }
}
async function deleteSlice(s){
 assert.ok(s.name.startsWith(runId),'owned slice name')
 const current=unwrap(await send(r.getSliceRequest(s.id)),'Slice').slice
 if(current.status==='running')await send(r.stopSliceRequest(s.id))
 unwrap(await send(r.deleteSliceRequest(s.id)),'SliceDeleted')
}
async function roomSnapshot(){
 const state=unwrap(await send(r.getSessionStateRequest(session.id)),'SessionState').session
 const e=unwrap(await send(r.getRoomEnvironmentStateRequest(session.id)),'RoomEnvironmentState').environment
 const history=await send(r.getSessionHistoryOutlineRequest(session.id))
 const actions=await send(r.listRoomEnvironmentActionHistoryRequest(session.id))
 return {sessionId:state.id,environmentId:e.environment_id,generation:e.runtime_generation,tabs:e.tabs.map(t=>({id:t.tab_id,url:t.url})).sort((a,b)=>a.id.localeCompare(b.id)),agents:state.agents.map(a=>({id:a.id,provider:a.provider,model:a.model})),history,actions}
}
async function startTui(kind,env,url,daemonId){
 const home=`${root}/${kind}-home`;await mkdir(home,{recursive:true})
 const args=['bun',`${clientRoot}/apps/cli/dist/index.js`,...(kind==='local'?['--kernel-url',url]:['--relay-url',env.CHARIOX_RELAY_URL,'--relay-token-env','LOOPS_RELAY_TOKEN','--target-daemon-id',daemonId]),'--automation-socket',`${root}/${kind}.sock`,'--session',session.id,'--workspace',`${root}/workspace`,'--worktree',`${root}/workspace`,'--provider','dev-stub','--model','default','--client-id',`${runId}-${kind}`]
 const invocation=roomTuiPtyInvocation(args)
 const child=spawn(invocation.command,invocation.args,{cwd:clientRoot,env:{...tuiEnvironment(home,kind==='remote'?env.LOOPS_RELAY_TOKEN:null,'LOOPS_RELAY_TOKEN',{}),PATH:process.env.PATH},detached:true,stdio:'ignore'})
 const state={child,automationSocket:`${root}/${kind}.sock`,attachmentId:null};activeTuis.set(kind,state)
 await waitForAutomationRoom(state,session.id)
 return state
}
async function stopTui(kind){const state=activeTuis.get(kind);if(!state)return;await stopProcessGroup(state.child.pid,state.child);if(state.attachmentId)await detachOwnedAttachment(send,r.detachFromSessionRequest(state.attachmentId),state.attachmentId);await rm(state.automationSocket,{force:true});activeTuis.delete(kind)}
async function webCommand(action,sequence){await writeWeb('web-command.json',{action,sequence});return await wait(async()=>{const error=await readFile(`${webRoot}/web-error.json`,'utf8').catch(()=>null);if(error)throw Error(JSON.parse(error).message);const value=JSON.parse(await readFile(`${webRoot}/web-result.json`,'utf8').catch(()=>'{}'));return value.sequence===sequence?value:false},`Web ${action}`,90000)}
async function gate(name,fn){const phase=process.env.LOOPS_PHASE??'all';if(phase!=='all'&&name!=='ocr-tools-list'&&!({'reconnect':'concurrent-and-reconnect','persistence':'save-restart','create-delete':'create-delete'}[phase]===name)){gates[name]={status:'NOT_RUN'};return}stage=name;try{if(interrupted)throw Error('MP-10 interrupted');await fn();gates[name]={status:'GREEN'}}catch(error){gates[name]={status:'RED',firstFailingSeam:stage,error:error.message};failures.push({gate:name,stage,error:error.message});await write(`failure-${name}`,{gate:name,stage,error:error.stack});await captureRoomRepetitionFailure({docker,write,sliceContainer:slice&&cname(slice),webContainer,privateRelayProbe:process.env.LOOPS_PRIVATE_RELAY_PROBE==='1'});console.log(`MP-08 MP-10 ${name} RED ${error.message}`)}await write('progress',{runId,gates,failures,stage})}
process.on('SIGTERM',()=>{interrupted=true});process.on('SIGINT',()=>{interrupted=true})
try{
 const kernel=`${release}/usr/local/bin/chariox-kernel`,relayBinary=`${release}/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay`
 const hash=async p=>createHash('sha256').update(await readFile(p)).digest('hex')
 assert.equal(await hash(kernel),'0695a7cb3a266546e51634f6fcda8b33b5d8f6f8e4f35430f96d4b2f4e2d229d')
 assert.equal(await hash(relayBinary),'85511eb0b02a533158814688568ad099b3a04632f54cdb6bf14f012cea8f159f')
 const labels=JSON.parse(await docker(['image','inspect',image]))[0].Config.Labels
 assert.equal(labels['io.chariox.runtime-source-revision'],process.env.LOOPS_EXPECT_RUNTIME_REVISION??'25a451deeb43da0d61172a31cc8db0661ca6e394c4737f31dc18b27889ba09fc')
 await write('identity',{runId,root,runtimeCommit:'b37f4504e4ce040a2d6c35dc56475315defbc861',harnessCommit:await cmd('git',['-C',repo,'rev-parse','HEAD']),clientCommit:await cmd('git',['-c',`safe.directory=${clientRoot}`,'-C',clientRoot,'rev-parse','HEAD']),cloudCommit:await cmd('git',['-c',`safe.directory=${cloud}`,'-C',cloud,'rev-parse','HEAD']),workerSource:process.env.LOOPS_WORKER_SOURCE??'b37f4504e4ce040a2d6c35dc56475315defbc861',workerRelayHash:process.env.LOOPS_WORKER_RELAY_HASH??'85511eb0b02a533158814688568ad099b3a04632f54cdb6bf14f012cea8f159f',workerKernelHash:process.env.LOOPS_WORKER_KERNEL_HASH??'0695a7cb3a266546e51634f6fcda8b33b5d8f6f8e4f35430f96d4b2f4e2d229d',kernelHash:await hash(kernel),relayHash:await hash(relayBinary),image,labels,protocol:370,peer:64,phase:process.env.LOOPS_PHASE??'all',limits:{diskReserveGiB:10,memoryReserveGiB:4},cycles:{reconnect:Number(process.env.LOOPS_RECONNECT_CYCLES??50),save:Number(process.env.LOOPS_SAVE_CYCLES??10),createDelete:Number(process.env.LOOPS_CREATE_CYCLES??50)},scope:process.env.LOOPS_RUN_SCOPE??'ordinary signed B loopback; synthetic bootstrap; no providers/hosted/fresh-machine acceptance'})
 await guard();await mkdir(`${root}/home`);await mkdir(`${root}/workspace`);await cmd('chmod',['711',root]);await cmd('chmod',['777',`${root}/workspace`]);await cmd('git',['init','-q',`${root}/workspace`])
 await writeFile(`${root}/home/config.toml`,`version = 1\n[state]\npath = "${root}/state.db"\n[slices]\nroot = "${root}/slices"\n[slices.linux]\ndocker_image = "${image}"\nbuild_image = "never"\nmemory_mb = 2048\ncpus = "1"\nscreen_width = 1280\nscreen_height = 800\n`)
 const ports=[];for(let i=0;i<5;i++)ports.push(await freePort())
 ports.forEach(port=>ownedPorts.add(port))
 const daemonId=`${runId}-home`,machineId=`${runId}-machine`,issuer=`${runId}-issuer`,secret=randomUUID()
 const mint=(subject,kind)=>roomDrillRelayToken({issuer,secret,machineId,subject,subjectKind:kind,actions:kind==='kernel'?['daemon_register','daemon_heartbeat','packet_route','peer_request','peer_event']:['client_metadata_read','client_connect','packet_route'],userId:'local',minimumLifetimeMs:4*3600000,env:{}})
 const env=Object.fromEntries(Object.entries(process.env).filter(([k])=>!/^(CHARIOX|CODEX|CLAUDE|OPENCODE|OPENAI|ANTHROPIC)_|TOKEN|SECRET|CREDENTIAL|AUTH|API_KEY/i.test(k)))
 Object.assign(env,{HOME:root,CHARIOX_HOME:`${root}/home`,CHARIOX_LOG_DIR:`${root}/logs`,XDG_CONFIG_HOME:`${root}/config`,XDG_STATE_HOME:`${root}/xdg-state`,XDG_CACHE_HOME:`${root}/cache`,CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_CODEX_PORT:String(ports[2]),CHARIOX_OPENCODE_PORT:String(ports[3]),CHARIOX_RELAY_PORT:String(ports[4]),CHARIOX_RELAY_HOST:'127.0.0.1',CHARIOX_RELAY_URL:`ws://127.0.0.1:${ports[4]}`,CHARIOX_RELAY_TOKEN:mint(daemonId,'kernel'),CHARIOX_RELAY_SCOPED_ISSUER:issuer,CHARIOX_RELAY_SCOPED_HMAC_SECRET:secret,CHARIOX_DAEMON_ID:daemonId,CHARIOX_DAEMON_ALIAS:daemonId,CHARIOX_MACHINE_ID:machineId,CHARIOX_MACHINE_ALIAS:machineId,CHARIOX_DAEMON_SOCKET:`${root}/daemon.sock`,CHARIOX_SESSION_HISTORY_DIR:`${root}/history`,CHARIOX_SLICE_DOCKER_PROVISIONER:`${process.env.LOOPS_PROVISIONER_ROOT??repo}/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh`,CHARIOX_SLICE_DOCKER_PIDS_LIMIT:'1024',DOCKER_HOST:'unix:///var/run/docker.sock',DOCKER_CONFIG:`${root}/docker-config`})
 relayChild=launch(relayBinary,env,'relay')
 kernelChild=launch(kernel,env,'kernel')
 local=await wait(async()=>{const c=new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`);try{await c.send(r.listSessionsRequest());return c}catch{await c.close();return false}},'kernel readiness')
 monitor=setInterval(()=>guard().catch(()=>{interrupted=true}),10000)
 session=unwrap(await send(r.createSessionRequest(`${root}/workspace`,`${root}/workspace`,runId,{provider:'dev-stub',model:'default'},null,'off')),'SessionCreated').session
 session=unwrap(await send(r.getSessionStateRequest(session.id)),'SessionState').session
 await createSlice('room')
 fixture=await startBrowserComputerFixture({host:'0.0.0.0',port:await freePort(),account:'agent@chariox.test',password:'loops-synthetic-fixture'})
 const fixturePort=Number(new URL(fixture.origin).port)
 const fixtureUrl=`http://127.0.0.1:${fixturePort}`
 await exposeFixture()
 const markers={cookie:`loops-cookie-${runId}`,local:`loops-local-${runId}`,idb:`loops-idb-${runId}`,cache:`loops-cache-${runId}`}
 await screen('open-url',`${fixtureUrl}/state/seed?${new URLSearchParams(markers)}`)
 await wait(async()=> (await screen('browser-text')).includes('CHARIOX_FIXTURE_STATE_SEEDED'),'state seeded')
 environment=unwrap(await send(r.startRoomEnvironmentRequest(session.id,{css_width:1280,css_height:800,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:800})),'RoomEnvironmentUpdated').environment
 assert.equal(environment.lifecycle,'ready')
 const baselineRoom=await roomSnapshot()
 await gate('ocr-tools-list',async()=>{
  const launched=unwrap(await send(r.launchProviderRunsRequest([{sessionId:session.id,provider:'dev-stub',accountProfile:'default',effort:'low',model:'native-tui-idle',agentId:session.agents[0].id,native:{nativeTui:true}}],1)),'ProviderRunsLaunchAccepted')
  assert.equal(launched.failures.length,0)
  const providerRun=await wait(async()=>{const p=unwrap(await send(r.getProviderRunRequest(launched.provider_runs[0].provider_run.id)),'ProviderRun').provider_run;return p.runtime_mcp_auth_token&&['Starting','Running'].includes(p.state)?p:false},'active MCP readiness');ocrRun=providerRun
  const response=await fetch(providerRun.runtime_mcp_server_url,{method:'POST',headers:{Authorization:`Bearer ${providerRun.runtime_mcp_auth_token}`,'content-type':'application/json'},body:JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/list',params:{}})})
  const result=await response.json();await write('ocr-tools-list',{providerRunId:providerRun.id,model:providerRun.model,state:providerRun.state,toolNames:result.result?.tools?.map(t=>t.name),error:result.error??null})
  assert.ok(result.result?.tools?.some(t=>t.name==='slice_ocr'),'slice_ocr missing in native dev-stub tools/list')
  await wait(async()=>{const p=unwrap(await send(r.getProviderRunRequest(providerRun.id)),'ProviderRun').provider_run;return p.state==='Running'?p:false},'stub running')
  const running=await fetch(providerRun.runtime_mcp_server_url,{method:'POST',headers:{Authorization:`Bearer ${providerRun.runtime_mcp_auth_token}`,'content-type':'application/json'},body:JSON.stringify({jsonrpc:'2.0',id:2,method:'tools/list',params:{}})}).then(x=>x.json())
  await write('ocr-tools-list-running',{providerRunId:providerRun.id,state:'Running',toolNames:running.result?.tools?.map(t=>t.name)})
  assert.ok(running.result?.tools?.some(t=>t.name==='slice_ocr'),'slice_ocr absent after Running')
 })
 if(process.env.LOOPS_PRIVATE_RELAY_PROBE==='1'){
  const probe=await readFile(`${repo}/apps/cli/scripts/lib/room-repetition-private-relay-probe.py`,'utf8')
  await docker(['exec','-d','-u','root','-e',`LOOPS_PRIVATE_RELAY_PORT=${slice.local_docker_ports.relay}`,cname(slice),'python3','-c',probe])
 }
 if(process.env.LOOPS_PRIVATE_STREAM_PROBE==='1'){
  const probe=await readFile(`${repo}/apps/cli/scripts/lib/room-repetition-private-stream-probe.py`,'utf8')
  await write('private-stream-probe',JSON.parse(await docker(['exec','-u','slice',cname(slice),'/opt/chariox-selkies/bin/python','-c',probe])))
 }
 await gate('concurrent-and-reconnect',async()=>{
  env.LOOPS_RELAY_TOKEN=mint(`${runId}-tui`,'client')
  await startTui('local',env,`ws://127.0.0.1:${ports[0]}/kernel`,daemonId);await startTui('remote',env,null,daemonId)
  const {bindLocalRoomWebRelayToken}=await import(pathToFileURL(`${cloud}/scripts/lib/room-web-companion-token.mjs`))
  webServer=http.createServer(async(req,res)=>{try{
   if(req.url==='/loops/bootstrap'){
    let body='';for await(const chunk of req)body+=chunk;const input=JSON.parse(body||'{}')
    const webClientId=`${runId}-web-${input.publicKeyThumbprint?.slice(0,24)??'unbound'}`
    let token=mint(webClientId,'client')
    await appendFile(`${evidence}/web-bootstrap-public-pins.jsonl`,JSON.stringify({mpItems,at:new Date().toISOString(),webClientId,publicKeyThumbprint:input.publicKeyThumbprint??null})+'\n')
    if(input.publicKeyThumbprint){await send(r.recordPairedClientRequest(webClientId,input.publicKeyThumbprint,'loops-web'));token=bindLocalRoomWebRelayToken({relayToken:token,relayScopedSecret:secret,relayScopedIssuer:issuer},input.publicKeyThumbprint)}
    res.setHeader('content-type','application/json');res.end(JSON.stringify({relayUrl:env.CHARIOX_RELAY_URL,relayToken:token,relayTokenExpiresAt:new Date(Date.now()+3600000).toISOString(),target:{kernelRef:daemonId,daemonId,daemonAlias:daemonId,machineId,status:'ONLINE',localDaemonProtocolVersion:370},profile:{userId:'local',accountId:'local-dev-account',realmId:'local-dev'}}));return
   }
   const pathname=new URL(req.url,'http://localhost').pathname
   const file=pathname.includes('.')?path.join(cloud,'apps/web/dist',pathname):`${cloud}/apps/web/dist/index.html`
   assert.ok(file.startsWith(`${cloud}/apps/web/dist/`))
   res.setHeader('content-type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':file.endsWith('.html')?'text/html':file.endsWith('.svg')?'image/svg+xml':'application/octet-stream');res.end(await readFile(file))
  }catch{res.statusCode=500;res.end('loop fixture failed')}})
  const webPort=await freePort();await new Promise(resolve=>webServer.listen(webPort,'127.0.0.1',resolve))
  for(const directory of [webRoot,webStaging]){await mkdir(directory,{recursive:true});await chmod(directory,0o777)}
  await writeWeb('web-config.json',{baseUrl:`http://127.0.0.1:${webPort}`,relayUrl:env.CHARIOX_RELAY_URL,daemonId,machineId,environmentId:environment.environment_id,target:`${session.id}:${session.agents[0].id}:${slice.id}`})
  webContainer=`${runId}-web`;await docker(['run','-d','--name',webContainer,'--user',`${WEB_UID}:${WEB_UID}`,'--security-opt',`seccomp=${repo}/apps/kernel/slice-linux-docker/chromium-seccomp.json`,'--label',`io.chariox.drill-run=${runId}`,'--network','host','--memory','1536m','--memory-swap','1536m','--cpus',process.env.LOOPS_WEB_CPUS??'1','--pids-limit','256','--mount',`type=bind,src=${cloud},dst=/cloud,readonly`,'--mount',`type=bind,src=${webRoot},dst=/runtime`,'--mount',`type=bind,src=${webStaging},dst=/evidence`,'--mount',`type=bind,src=${repo}/apps/cli/scripts/lib/room-repetition-web.mjs,dst=/web.mjs,readonly`,'--mount',`type=bind,src=${repo}/apps/cli/scripts/lib/room-repetition-gates.mjs,dst=/room-repetition-gates.mjs,readonly`,'--entrypoint','node',image,'/web.mjs'])
  // The viewer runs as uid 1001, so /cloud and the two mounted scripts must be world-readable
  // (an ordinary umask-022 checkout). A viewer that exits before reporting is fatal at once.
  await wait(async()=>{const error=await readFile(`${webRoot}/web-error.json`,'utf8').catch(()=>null);if(error){const e=Error(JSON.parse(error).message);e.fatal=true;throw e}
   const ready=JSON.parse(await readFile(`${webRoot}/web-ready.json`,'utf8').catch(()=>'null'));if(ready)return ready
   // A direct probe keeps these frequent polls out of commands.jsonl.
   const state=await exec('docker',['inspect','--format','{{.State.Status}} {{.State.ExitCode}}',webContainer],{env:{...process.env,DOCKER_HOST:'unix:///var/run/docker.sock',DOCKER_CONFIG:`${root}/docker-config`}}).then(r=>r.stdout.trim(),()=>'')
   if(state.startsWith('exited')||state.startsWith('dead')){const logs=await cmd('sh',['-c','docker logs --tail 40 "$1" 2>&1','drill',webContainer]).catch(e=>e.message);const e=Error(`Web viewer stopped before it was ready (${state}): ${String(logs).slice(-1500)}`);e.fatal=true;throw e}
   return null},'two Web viewers',120000)
  await write('concurrent',{room:await roomSnapshot(),worker:await observeWorker(),web:JSON.parse(await readFile(`${webRoot}/web-ready.json`,'utf8')),tuis:await Promise.all([...activeTuis.values()].map(t=>readAutomationSnapshot(t.automationSocket)))})
  // Generate an active browser/stream workload through a fixture-only CDP setup.
  await docker(['exec',cname(slice),'node','--input-type=module','-e',`const targets=await(await fetch('http://127.0.0.1:9222/json/list')).json();const target=targets.find(t=>t.type==='page');const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise(r=>ws.onopen=r);const reply=new Promise(r=>ws.onmessage=e=>r(JSON.parse(e.data)));ws.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:"window.loopsTicks=0;setInterval(()=>{document.querySelector('h1').textContent='MP-08 MP-10 active browser tick '+(++window.loopsTicks)},250)",returnByValue:true}}));await reply;ws.close()`])
  const before=await roomSnapshot()
  for(let cycle=1;cycle<=Number(process.env.LOOPS_RECONNECT_CYCLES??50);cycle++){
   await guard();const started=performance.now()
   await Promise.all([stopTui('local'),stopTui('remote')])
   const webResult=await webCommand('reconnect',cycle)
   await startTui('local',env,`ws://127.0.0.1:${ports[0]}/kernel`,daemonId);await startTui('remote',env,null,daemonId)
   assertSameRoom(before,await roomSnapshot())
   await write(`reconnect-${cycle}`,{cycle,elapsedMs:performance.now()-started,webResult,worker:await observeWorker(),sample:await inventory(`reconnect-inventory-${cycle}`)})
   console.log(`MP-08 MP-10 reconnect ${cycle}`)
  }
 })
 if(slice){try{const log=await docker(['exec',cname(slice),'sh','-c',"cat /home/slice/.local/state/chariox/logs/*daemon*.ndjson /home/slice/.chariox/logs/*daemon*.ndjson /opt/chariox-slice/logs/*daemon*.ndjson 2>/dev/null; true"]);await writeFile(`${evidence}/worker-forwarder.log`,log.split('\n').filter(line=>line.includes('\"component\":\"display.')).join('\n')+'\n',{mode:0o600})}catch{}}
 await Promise.all([...activeTuis.keys()].map(stopTui))
 if(webContainer){await docker(['rm','-f',webContainer]);webContainer=null;await publishWebEvidence()}
 if(webServer){await new Promise(resolve=>webServer.close(resolve));webServer=null}
 await gate('save-restart',async()=>{
  let portsBefore=structuredClone(slice.local_docker_ports),idBefore=(await docker(['exec',cname(slice),'cat','/etc/machine-id'])).trim()
  await docker(['exec','-u','slice',cname(slice),'sh','-c',`printf '%s' '${runId}' > /home/slice/loops-document.txt`])
  for(let cycle=1;cycle<=Number(process.env.LOOPS_SAVE_CYCLES??10);cycle++){
   await guard();const started=performance.now(),workerBefore=await observeWorker(),sliceIdBefore=slice.id,machineIdBefore=idBefore
   const saved=unwrap(await send(r.saveSliceStateRequest(slice.id,'shutdown')),'SliceStateSaved')
   savedImages.add(saved.state.image_ref);assert.equal(saved.slice.status,'stopped');assert.ok((await stat(saved.state.home_archive_path)).size>0)
   await docker(['rm','-f',cname(slice)]);await docker(['volume','rm',`${cname(slice)}-home`])
   let collisionListener=null
   if(cycle===1&&process.env.LOOPS_FORCE_RESTORE_PORT_RACE==='1'){
    collisionListener=net.createServer()
    let collisionSource='owned injected listener'
    try{await new Promise((resolve,reject)=>collisionListener.once('error',reject).listen(portsBefore.novnc,'127.0.0.1',resolve))}
    catch(error){if(error.code!=='EADDRINUSE')throw error;collisionListener=null;collisionSource='port already occupied; natural collision retained'}
    await write('forced-restore-port-race',{sliceId:slice.id,ownedTestPort:portsBefore.novnc,savedStateId:saved.state.id,collisionSource})
   }
   let portRaceRetries
   try{portRaceRetries=await restoreSavedWithPortRetry(saved,cycle)}finally{
if(collisionListener)await new Promise(resolve=>collisionListener.close(resolve))}
   if(cycle===1&&process.env.LOOPS_FORCE_RESTORE_PORT_RACE==='1')assert.ok(portRaceRetries>=1,'forced port collision must exercise saved-state-preserving retry')
   await screen('status');await exposeFixture();if(portRaceRetries){portsBefore=structuredClone(slice.local_docker_ports)}else{assert.deepEqual(slice.local_docker_ports,portsBefore)};const machineIdAfter=await docker(['exec',cname(slice),'cat','/etc/machine-id']);assertRestoredMachineIdentity(machineIdBefore,machineIdAfter,sliceIdBefore,slice.id);idBefore=machineIdAfter;assert.equal(await docker(['exec',cname(slice),'cat','/home/slice/loops-document.txt']),runId)
   await screen('open-url',`${fixtureUrl}/state/check`)
   const text=await wait(async()=>{const t=await screen('browser-text');return t.includes(markers.idb)?t:false},'browser storage restored')
   for(const marker of Object.values(markers))assert.ok(text.includes(marker),'browser marker lost');assert.match(text,/"serviceWorker":true/)
   const e=unwrap(await send(r.startRoomEnvironmentRequest(session.id,{css_width:1280,css_height:800,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:800})),'RoomEnvironmentUpdated').environment
   assert.equal(e.environment_id,baselineRoom.environmentId)
   const workerAfter=await observeWorker();assert.notEqual(workerAfter.containerId,workerBefore.containerId)
   await write(`save-restart-${cycle}`,{cycle,portRaceRetries,sliceId:slice.id,durablePortComparison:portRaceRetries?'replacement slice allocated new ports; prior collision retained':'same slice ports unchanged',elapsedMs:performance.now()-started,savedStateId:saved.state.id,containerBefore:workerBefore.containerId,containerAfter:workerAfter.containerId,browserMarkers:'all four PASS; service worker PASS',machineIdBefore,machineIdAfter,machineIdentityComparison:portRaceRetries?'replacement stable identity matches new slice':'same-slice identity unchanged',durablePorts:portsBefore,worker:workerAfter,sample:await inventory(`save-inventory-${cycle}`)})
   console.log(`MP-08 MP-10 save/restart ${cycle}`)
  }
 })
 await deleteSlice(slice)
 for(const ref of savedImages){assert.ok(ref.startsWith(`chariox-slice-state:${runId}-`));await docker(['image','rm',ref]).catch(()=>{})}
 await sleep(2000)
 const baseline=await inventory('create-delete-baseline')
 await gate('create-delete',async()=>{
  for(let cycle=1;cycle<=Number(process.env.LOOPS_CREATE_CYCLES??50);cycle++){
   const started=performance.now();await createSlice(`cycle-${cycle}`)
   await screen('open-url',`${fixtureUrl}/state/check`)
   const worker=await observeWorker();await deleteSlice(slice);await sleep(1000)
   const after=await inventory(`create-delete-${cycle}`),regressions=compareCycle(baseline,after)
   await write(`create-delete-result-${cycle}`,{cycle,elapsedMs:performance.now()-started,worker,regressions})
   assert.deepEqual(regressions,[],`MP-10 leak at cycle ${cycle}`)
   console.log(`MP-08 MP-10 create/delete ${cycle}`)
  }
 })
}catch(error){failures.push({stage,error:error.message});await write('failure',{stage,error:error.stack})}
finally{
 if(slice){try{const log=await docker(['exec',cname(slice),'sh','-c',"cat /home/slice/.local/state/chariox/logs/*daemon*.ndjson /home/slice/.chariox/logs/*daemon*.ndjson /opt/chariox-slice/logs/*daemon*.ndjson 2>/dev/null; true"]);await writeFile(`${evidence}/worker-forwarder.log`,log.split('\n').filter(line=>line.includes('\"component\":\"display.')).join('\n')+'\n',{mode:0o600})}catch{}}
 clearInterval(monitor)
 for(const kind of [...activeTuis.keys()])await stopTui(kind).catch(e=>failures.push({stage:'cleanup-tui',error:e.message}))
 if(webContainer)await docker(['rm','-f',webContainer]).then(()=>{webContainer=null},e=>failures.push({stage:'cleanup-web',error:e.message}))
 // Staged evidence is published only once the viewer container is gone.
 await publishWebEvidence().catch(e=>failures.push({stage:'cleanup-web-evidence',error:e.message}))
 if(webServer)await new Promise(resolve=>webServer.close(resolve))
 await fixture?.close().catch(()=>{})
 for(const s of slices){const exists=await docker(['inspect',cname(s)]).catch(()=>null);if(exists&&local)await deleteSlice(s).catch(e=>failures.push({stage:'cleanup-slice',error:e.message}))}
 for(const ref of savedImages)if(ref.startsWith(`chariox-slice-state:${runId}-`))await docker(['image','rm',ref]).catch(()=>{})
 if(local&&session)await send(r.endSessionRequest(session.id)).catch(e=>failures.push({stage:'cleanup-session',error:e.message}))
 if(local&&ocrRun){try{
  const ended=unwrap(await send(r.getProviderRunRequest(ocrRun.id)),'ProviderRun').provider_run
  const discovery=await fetch(ocrRun.runtime_mcp_server_url,{method:'POST',headers:{Authorization:`Bearer ${ocrRun.runtime_mcp_auth_token}`,'content-type':'application/json'},body:JSON.stringify({jsonrpc:'2.0',id:3,method:'tools/list',params:{}})}).then(x=>x.json())
  const toolNames=discovery.result?.tools?.map(t=>t.name)
  await write('ocr-tools-list-ended',{providerRunId:ended.id,state:ended.state,toolNames})
  assert.equal(ended.state,'Ended');assert.deepEqual(toolNames,[])
 }catch(error){failures.push({stage:'ocr-ended-discovery',error:error.message})}}
 await local?.close().catch(()=>{})
 for(const child of owned){try{process.kill(-child.pid,'SIGTERM')}catch{}}
 await sleep(1500)
 for(const child of owned)if(child.exitCode===null&&child.signalCode===null){try{process.kill(-child.pid,'SIGKILL')}catch{}}
 const after=await inventory('final-inventory').catch(e=>({error:e.message}))
 const cleanupErrors=['containers','volumes','images','ownedPids','listeners'].filter(k=>after[k]?.length)
 await write('result',{runId,status:failures.length||cleanupErrors.length?'RED':'GREEN',gates,failures,cleanupErrors,scope:process.env.LOOPS_RUN_SCOPE??'provider-free ordinary signed B; no MP item closes',root,finishedAt:new Date().toISOString()})
 // Publish only owned kernel/relay logs; synthetic tokens are never emitted intentionally.
 for(const name of ['kernel','relay']){const log=await readFile(`${root}/${name}.log`,'utf8').catch(()=>'');const safe=log.replace(/chariox-scoped-v1\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g,'[REDACTED]');await writeFile(`${evidence}/${name}.log`,safe,{mode:0o600})}
 if(!cleanupErrors.length)await rm(root,{recursive:true,force:true})
 process.exitCode=failures.length||cleanupErrors.length?1:0
}

#!/usr/bin/env node
// Own kernel, relay, cgroup and loop filesystem; never exhaust the host.
import assert from 'node:assert/strict'
import {randomBytes} from 'node:crypto'
import {writeFileSync} from 'node:fs'
import {mkdir,open,readFile,readdir,rmdir,rm,statfs,writeFile} from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import {fileURLToPath} from 'node:url'
import {setTimeout as sleep} from 'node:timers/promises'
import {LocalIpcClient} from '../../../packages/kernel-client/dist/ipc.js'
import {RelayClientIdentity,createRelayKeypair} from '../../../packages/kernel-client/dist/relay-crypto.js'
import {spawnAgentRequest,attachToSessionRequest,submitPromptRequest,endSessionRequest,getUserConfigRequest,listSessionsRequest,relayStatusRequest} from '../../../packages/kernel-client/dist/ipc-requests.js'
import {executeShellCommand} from '../../../packages/kernel-client/dist/shell-executor.js'
import {createDefaultShellContext,parseShellCommand} from '../../../packages/kernel-client/dist/shell-core.js'
import {spawnOwned,signalOwnedProcessGroup,signalOwnedProcess,ownedProcessHandles,registerCapturedProcessGroup} from '../../kernel/slice-linux-docker/owned-process-signals.mjs'

const required = key => { assert(process.env[key], `${key} is required`); return process.env[key] }
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const evidence = required('RESSET_EVIDENCE'), state = required('RESSET_STATE'), binary = required('RESSET_KERNEL_BINARY'), relayBinary = required('RESSET_RELAY_BINARY')
for (const dir of [evidence,state]) { assert(path.isAbsolute(dir) && !dir.startsWith(repo+'/')); await mkdir(dir,{recursive:true}) }
const group = '/sys/fs/cgroup/system.slice/resset-'+process.pid
const workspace = path.join(state,'workspace'), disk = path.join(state,'disk')
const children=[], handles=[], execs=[], receipt={head:required('RESSET_HEAD'),started:new Date().toISOString(),steps:[],resources:[]}
let kernel,relay,local,remote,sessionId,providerSessionId,mounted=false,groupCreated=false
const step = (name,details={}) => {receipt.steps.push({name,...details});writeFileSync(path.join(evidence,'live-progress.json'),JSON.stringify(receipt,null,2)+'\n')}
async function resources() {
  const available=Number((await readFile('/proc/meminfo','utf8')).match(/MemAvailable:\s+(\d+)/)[1])*1024
  const fs=await statfs('/'); receipt.resources.push({at:new Date().toISOString(),memoryAvailable:available,diskAvailable:fs.bavail*fs.bsize})
  assert(available>=12*1024**3 && fs.bavail*fs.bsize>=44*1024**3,'host reserve breached')
}
async function port() {const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p}
async function until(fn,label,timeout=20000) {const end=Date.now()+timeout;let last;while(Date.now()<end){try{const value=await fn();if(value)return value}catch(e){last=e}await sleep(100)}throw Error(`${label} timed out${last ? ': '+last.message : ''}`)}
async function launch(program,args,env=process.env,label='command') {
  const log=await open(path.join(evidence,label+'.log'),'a',0o600);handles.push(log)
  const child=spawnOwned(program,args,{cwd:state,env,stdio:['ignore',log.fd,log.fd],detached:true,retainLeader:true});children.push(child)
  child.done=new Promise((resolve,reject)=>{child.once('error',reject);child.once('close',(code,signal)=>resolve({code,signal}))})
  return child
}
async function run(program,args,label) {const child=await launch(program,args,process.env,label);const result=await child.done;assert.equal(result.code,0,`${label} failed; see its private log`)}
async function stop(child) {
  if(child?.captured) {
    if(child.exited)return
    for(const owned of ownedProcessHandles(child.captured).filter(p=>p.pid!==child.pid))signalOwnedProcess(owned,'SIGTERM')
    signalOwnedProcessGroup(child.captured,'SIGTERM');await Promise.race([child.done,sleep(4000)])
    if(!child.exited){for(const owned of ownedProcessHandles(child.captured))signalOwnedProcess(owned,'SIGKILL');await child.done}
    return
  }
  if(!child || child.exitCode!==null || child.signalCode!==null)return
  for(const owned of ownedProcessHandles(child).filter(p=>p.pid!==child.pid))signalOwnedProcess(owned,'SIGTERM')
  signalOwnedProcessGroup(child,'SIGTERM');await Promise.race([child.done,sleep(4000)])
  if(child.exitCode===null && child.signalCode===null){signalOwnedProcessGroup(child,'SIGKILL');await child.done}
}
function unwrap(value,key) {assert(!value.Error, value.Error?.message);assert(value[key],`missing ${key}`);return value[key]}
async function rpc(request,client=local) {return await client.send(request)}
async function room(operation) {return unwrap(await rpc({RoomTerminal:{session_id:sessionId,operation}}),'RoomTerminal').result}
async function exec(command) {const result=await room({operation:'exec_open',command});assert.equal(result.result,'exec_opened');execs.push(result.exec_id);return result.exec_id}
async function outcome(id) {return (await room({operation:'exec_read',exec_id:id,after_cursor:0,wait_ms:0})).output}
async function finish(id) {return await until(async()=>{const output=await outcome(id);return output.outcome && output},'command result')}
async function close(id) {await room({operation:'exec_cancel',exec_id:id});await room({operation:'exec_close',exec_id:id});execs.splice(execs.indexOf(id),1)}
async function config(resource,value,client=local) {
  const context=createDefaultShellContext({workspace,worktree:workspace})
  const result=await executeShellCommand(parseShellCommand(`config set kernel.resource_protection.${resource} ${value}`),context,{client})
  assert.equal(result.ok,true,result.message);return result
}
async function settings(client=local) {return unwrap(await rpc(getUserConfigRequest(),client),'UserConfig').config.kernel.resource_protection}
async function removeGroups(directory) {for(const e of await readdir(directory,{withFileTypes:true}))if(e.isDirectory())await removeGroups(path.join(directory,e.name));for(let attempt=0;;attempt++){try{await rmdir(directory);break}catch(error){if(error.code!=='EBUSY'||attempt>=50)throw error;await sleep(100)}}}

try {
  await resources();await mkdir(workspace,{recursive:true});await mkdir(disk,{recursive:true})
  await mkdir(group);groupCreated=true
  await writeFile(path.join(group,'memory.max'),String(4*1024**3))
  // Keep the controlled buffers resident so the drill observes the pause path.
  await writeFile(path.join(group,'memory.swap.max'),'0')
  await writeFile(path.join(group,'cgroup.subtree_control'),'+memory +cpu')
  const leaf=path.join(group,'kernel');await mkdir(leaf)
  const ports=[];for(let i=0;i<5;i++)ports.push(await port())
  const env={...Object.fromEntries(Object.entries(process.env).filter(([key])=>!key.startsWith('CHARIOX_')&&!['CODEX_HOME','CLAUDE_CONFIG_DIR','OPENCODE_CONFIG_DIR'].includes(key))),
    HOME:path.join(state,'user'),CHARIOX_HOME:path.join(state,'kernel'),XDG_CONFIG_HOME:path.join(state,'config'),XDG_STATE_HOME:path.join(state,'xdg-state'),XDG_DATA_HOME:path.join(state,'data'),XDG_CACHE_HOME:path.join(state,'cache'),
    CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_CODEX_PORT:String(ports[2]),CHARIOX_OPENCODE_PORT:String(ports[3]),CHARIOX_RELAY_PORT:String(ports[4]),CHARIOX_LOG_DIR:path.join(evidence,'kernel-logs'),CHARIOX_DAEMON_SOCKET:path.join(state,'kernel.sock'),TOKIO_WORKER_THREADS:'2',CHARIOX_RELAY_TOKEN:randomBytes(32).toString('hex')}
  let kernelSequence=0
  const startKernel=async()=>{
    const identity=path.join(state,`kernel-${++kernelSequence}.json`)
    const unit=`resset-fixture-${process.pid}-${kernelSequence}`
    // The older parent kernel must not supervise this kernel's pressure tests.
    // The transient service owns its launch; captured birth metadata owns cleanup.
    const child=await launch('systemd-run',['--quiet','--collect',`--unit=${unit}`,
      `--property=WorkingDirectory=${state}`,`--property=StandardOutput=append:${path.join(evidence,'kernel.log')}`,
      `--property=StandardError=append:${path.join(evidence,'kernel.log')}`,
      ...Object.keys(env).map(name=>`--setenv=${name}`),'python3',path.join(repo,'apps/cli/scripts/lib/resource-protection-kernel.py'),leaf,binary,identity],env,'kernel-launch')
    assert.equal((await child.done).code,0,'private kernel service launch failed')
    const metadata=await until(async()=>JSON.parse(await readFile(identity,'utf8')),'captured kernel identity')
    const captured=registerCapturedProcessGroup(metadata)
    const service={pid:metadata.pid,captured,exited:false}
    service.done=(async()=>{while(ownedProcessHandles(captured).length)await sleep(25);service.exited=true})()
    children.push(service);return service
  }
  relay=await launch(relayBinary,[],env,'relay');kernel=await startKernel()
  local=new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`,{localAuthEnvironment:env})
  await until(async()=>{unwrap(await rpc(listSessionsRequest()),'SessionsListed');return true},'kernel startup')
  assert.deepEqual(await settings(),{memory:true,disk:true,cpu:true});step('defaults-on')
  const relayUrl=`ws://127.0.0.1:${ports[4]}`
  unwrap(await rpc({ConfigureRelay:{relay_url:relayUrl,relay_token:env.CHARIOX_RELAY_TOKEN}}),'RelayConfigured')
  const status=await until(async()=>{const s=unwrap(await rpc(relayStatusRequest()),'RelayStatus').status;return s.connected && s},'real relay registration')
  const identity=new RelayClientIdentity(createRelayKeypair().privateKey)
  const pairing=unwrap(await rpc({CreateTerminalPairingLink:{terminal_type:'cli',alias:'Resource settings drill',expires_in_ms:300000}}),'TerminalPairingLinkCreated').pairing
  unwrap(await rpc({JoinTerminalPairingLink:{pairing_link:pairing.pairing_link,terminal_id:null,terminal_type:'cli',alias:'Resource settings drill',public_key_thumbprint:identity.publicKeyThumbprint}}),'TerminalPairingLinkJoined')
  // This isolated relay uses its configured operator credential. Pairing above
  // records the receiving client's public key through the real kernel API.
  remote=new LocalIpcClient(relayUrl,{relayAuthToken:env.CHARIOX_RELAY_TOKEN,targetDaemonId:status.daemon_id,relayIdentity:identity})
  for(const resource of ['memory','disk','cpu']) {
    await config(resource,false,remote)
    const actual=await settings();assert.equal(actual[resource],false)
    for(const other of ['memory','disk','cpu'].filter(r=>r!==resource))assert.equal(actual[other],true)
    await config(resource,true);step('independent-setting-through-real-relay',{resource})
  }
  sessionId=unwrap(await rpc({CreateSession:{workspace_id:workspace,worktree_id:workspace,alias:'resource-settings'}}),'SessionCreated').session.id
  const profile=unwrap(await rpc({LinkProviderAccountProfile:{provider:'codex',label:'Resource settings drill',path:required('RESSET_CODEX_PROFILE')}}),'ProviderAccountProfile').profile
  const auth=unwrap(await rpc({GetProviderAuthStatus:{provider:'codex',account_profile:profile.profile_id}}),'ProviderAuthStatus').status
  assert.equal(auth.auth_state,'authenticated','official provider authentication must pass')
  providerSessionId=unwrap(await rpc({CreateSession:{workspace_id:workspace,worktree_id:workspace,alias:'resource-provider'}}),'SessionCreated').session.id
  const attached=unwrap(await rpc(attachToSessionRequest(providerSessionId,'resset-provider')),'SessionAttached')
  await config('memory',false)
  const agent=unwrap(await rpc(spawnAgentRequest(providerSessionId,'codex','resource-provider','codex/gpt-6-luna',workspace,'low','build','yolo',undefined,undefined,undefined,profile.profile_id)),'AgentSpawned').agent
  const providerProof=path.join(state,'provider-proof.json'), providerCommand=path.join(state,'provider-proof.py')
  await writeFile(providerCommand,`import json,os,resource,time
from pathlib import Path
b=bytearray(128*1024**2)
time.sleep(2)
Path(${JSON.stringify(providerProof)}).write_text(json.dumps({"pid":os.getpid(),"rss_kib":resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,"cgroup":Path("/proc/self/cgroup").read_text().strip()}))
`)
  await rpc(submitPromptRequest(providerSessionId,attached.attachment.id,agent.id,`Run python3 ${providerCommand} exactly once. It allocates 128 MiB briefly and writes this isolated drill's receipt. Then reply RESOURCE_SETTINGS_PROVIDER_OK.`,[]))
  const processCount=await until(async()=>{const view=unwrap(await rpc({GetSessionState:{session_id:providerSessionId}}),'SessionState');return view.processes?.filter(p=>p.owner.kind==='agent' && p.owner.id===agent.id).length},'real provider resource projection',90000)
  const proof=await until(async()=>JSON.parse(await readFile(providerProof,'utf8')),'real low-effort provider command',180000)
  assert(proof.rss_kib>=128*1024);step('real-provider-command-with-memory-off',{model:'codex/gpt-6-luna',effort:'low',processCount,proof})
  await rpc(endSessionRequest(providerSessionId));providerSessionId=null
  const groupPids=async directory=>{
    const pids=(await readFile(path.join(directory,'cgroup.procs'),'utf8')).trim().split(/\s+/).filter(Boolean).map(Number)
    for(const entry of await readdir(directory,{withFileTypes:true}))if(entry.isDirectory())pids.push(...await groupPids(path.join(directory,entry.name)))
    return pids
  }
  await until(async()=>{
    for(const entry of await readdir(group,{withFileTypes:true}))
      if(entry.isDirectory() && entry.name!=='kernel' && (await groupPids(path.join(group,entry.name))).length)return false
    return true
  },'provider cleanup before pressure',30000)
  // Cgroup memory.stat is batched. Let the provider's charge settle before
  // lowering this test ancestor; stale reclaimable counts can cause real OOM.
  await sleep(2500);await config('memory',true)
  // Two non-harness commands exceed the light threshold, while only the newer
  // one grows. A tight *owned ancestor* reproduces near-OOM without host pressure.
  const idle=path.join(state,'idle.pid'),grow=path.join(state,'grow.pid'),go=path.join(state,'grow-go'),progress=path.join(state,'progress')
  const memorySource=`import os,sys,time\nfrom pathlib import Path\nPath(sys.argv[1]).write_text(str(os.getpid()))\nb=[bytearray(256*1024**2)]\nif len(sys.argv)>2:\n while not Path(sys.argv[2]).exists(): time.sleep(.05)\n for i in range(256):\n  b.append(bytearray(4*1024**2)); Path(sys.argv[3]).write_text(str(i)); time.sleep(.25)\nelse: time.sleep(180)\n`
  const memoryFile=path.join(state,'memory.py');await writeFile(memoryFile,memorySource)
  const memExec=await exec(['/bin/sh','-c','python3 "$1" "$2" & python3 "$1" "$3" "$4" "$5" & wait','resset',memoryFile,idle,grow,go,progress])
  const growPid=await until(async()=>Number(await readFile(grow,'utf8')),'grower starts')
  const idlePid=await until(async()=>Number(await readFile(idle,'utf8')),'idle holder starts')
  await until(async()=>{for(const pid of [idlePid,growPid]) {const status=await readFile(`/proc/${pid}/status`,'utf8');if(Number(status.match(/VmRSS:\s+(\d+)/)?.[1])<=250*1024)return false}return true},'both allocations observed')
  await until(async()=>{
    const view=unwrap(await rpc({GetSessionState:{session_id:sessionId}}),'SessionState')
    return [idlePid,growPid].every(pid=>view.processes?.some(row=>row.pid===pid && row.resident_set_bytes>250*1024**2))
  },'kernel observes both heavy commands',30000)
  await sleep(1000)
  for(const pid of [idlePid,growPid])assert(!(/State:\s+T/.test(await readFile(`/proc/${pid}/status`,'utf8'))),'healthy heavy command was paused')
  step('healthy-heavy-commands-keep-running',{idlePid,growPid})
  const relative=(await readFile(`/proc/${growPid}/cgroup`,'utf8')).trim().split('0::')[1]
  const budget='/sys/fs/cgroup'+relative.split('/unit-')[0]
  assert(budget.startsWith(group+'/chariox-'))
  const controls={};for(const name of ['memory.max','memory.high','memory.swap.max','cpu.weight'])controls[name]=(await readFile(path.join(budget,name),'utf8')).trim()
  assert.deepEqual(controls,{'memory.max':'max','memory.high':'max','memory.swap.max':'max','cpu.weight':'100'});step('healthy-accounting-has-no-private-throttles',{controls})
  // Fix the ancestor above its current charge, then approach its limit by
  // allocating slowly. Lowering a limit below dirty cache can invoke the OS
  // OOM killer before the kernel's watchdog gets a sample.
  const oomKills=async()=>Number((await readFile(path.join(group,'memory.events'),'utf8')).match(/^oom_kill (\d+)/m)[1])
  const beforeOomKills=await oomKills()
  const limit=Number((await readFile(path.join(group,'memory.current'),'utf8')).trim())+128*1024**2
  await writeFile(path.join(group,'memory.max'),String(limit));await writeFile(go,'go')
  const pressureSamples=[]
  await until(async()=>{
    const status=await readFile(`/proc/${growPid}/status`,'utf8')
    pressureSamples.push({at:new Date().toISOString(),limit,current:Number((await readFile(path.join(group,'memory.current'),'utf8')).trim()),state:status.match(/State:\s+(\S+)/)?.[1]})
    await writeFile(path.join(evidence,'memory-pressure.json'),JSON.stringify(pressureSamples,null,2)+'\n')
    return /State:\s+T/.test(status)
  },'last-resort pause',120000)
  step('real-near-oom-pauses-grower')
  await config('memory',false)
  await until(async()=>!(/State:\s+T/.test(await readFile(`/proc/${growPid}/status`,'utf8'))),'memory off resumes process')
  const {value:before}=await until(async()=>({value:Number(await readFile(progress,'utf8'))}),'resumed growth starts');await sleep(1200);const after=Number(await readFile(progress,'utf8'));assert(after>before)
  assert(!(/State:\s+T/.test(await readFile(`/proc/${growPid}/status`,'utf8'))));assert.equal(await oomKills(),beforeOomKills);step('memory-off-resumes-and-allows-growth-under-pressure',{before,after,oomKills:0})
  await writeFile(path.join(group,'memory.max'),String(4*1024**3));await close(memExec);await config('memory',true)
  // A private ext4 loop volume produces real write_bytes and statvfs pressure.
  const image=path.join(state,'disk.img'),file=await open(image,'w');await file.truncate(160*1024**2);await file.close()
  await run('mkfs.ext4',['-q','-F','-m','0',image],'mkfs')
  await run('mount',['-o','loop',image,disk],'mount');mounted=true
  const fillProgram=path.join(state,'fill.py')
  await writeFile(fillProgram,'import os,sys\nf=open(sys.argv[1],"wb")\nwhile True:\n s=os.statvfs(sys.argv[2])\n if s.f_bavail*s.f_frsize<=24*1024**2: break\n f.write(b"x"*1024**2); f.flush()\nf.close()\n')
  await config('disk',false)
  const fill=await exec(['python3',fillProgram,path.join(disk,'filler'),disk])
  assert.deepEqual((await finish(fill)).outcome,{kind:'exited',code:0});await close(fill)
  await config('disk',true)
  const writerFile=path.join(state,'writer.py');await writeFile(writerFile,'import sys,time\nf=open(sys.argv[1],"wb")\nfor i in range(4):\n f.write(b"x"*(4*1024**2)); f.flush(); import os; os.fsync(f.fileno()); time.sleep(.5)\n')
  const writer=['/bin/sh','-c','python3 "$1" "$2" & wait $!','resset',writerFile,path.join(disk,'output')]
  const diskOn=await exec(writer),stopped=await finish(diskOn)
  assert.notEqual(stopped.outcome.code,0,'disk-on writer was not stopped')
  const diskNotice=stopped.chunks.map(chunk=>Buffer.from(chunk.data_base64,'base64').toString()).join('')
  assert.match(diskNotice,/chariox: stopped this command: the disk.*was filling it/s)
  step('disk-on-stops-real-near-full-writer',{outcome:stopped.outcome,notice:diskNotice.trim()});await close(diskOn)
  await rm(path.join(disk,'output'),{force:true});await config('disk',false)
  const diskOff=await exec(writer),completed=await finish(diskOff)
  assert.deepEqual(completed.outcome,{kind:'exited',code:0});step('disk-off-allows-real-near-full-writer',{outcome:completed.outcome});await close(diskOff)
  await config('cpu',false)
  const busy=await exec(['python3','-c','import time\nend=time.monotonic()+1\nwhile time.monotonic()<end: pass'])
  assert.deepEqual((await finish(busy)).outcome,{kind:'exited',code:0});await close(busy);step('cpu-off-keeps-owned-work-running')
  const persisted=await settings();remote.destroy();remote=null;await local.close();local=null;await stop(kernel);kernel=await startKernel()
  local=new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`,{localAuthEnvironment:env})
  await until(async()=>{assert.deepEqual(await settings(),persisted);return true},'settings persist across restart');step('settings-survive-real-restart')
  await resources();receipt.status='PASS'
} catch(error) {receipt.status='FAIL';receipt.error=error.message;await writeFile(path.join(evidence,'live-result.json'),JSON.stringify(receipt,null,2)+'\n');throw error}
finally {
  remote?.destroy();local?.destroy();for(const child of [...children].reverse())await stop(child)
  if(mounted)await run('umount',[disk],'umount')
  for(const h of handles)await h.close()
  if(groupCreated)await removeGroups(group)
  receipt.finished=new Date().toISOString();await writeFile(path.join(evidence,'live-result.json'),JSON.stringify(receipt,null,2)+'\n')
}

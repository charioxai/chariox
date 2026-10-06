#!/usr/bin/env node
// MP-05/MP-08/MP-10/MP-11: real kernels, scoped local relay, CLI copy and TUI review.
// Cloud is a loopback control-plane fixture; hosted Cloud/web acceptance is separate.
import assert from 'node:assert/strict'
import { createHash, createHmac, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { mkdir, mkdtemp, writeFile, readFile, rm, rename, access, chmod } from 'node:fs/promises'
import { spawnOwned, signalOwnedProcessGroup, signalOwnedProcess, processSnapshot } from '../../kernel/slice-linux-docker/owned-process-signals.mjs'
import { createReadStream } from 'node:fs'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const [binaryDir, evidence, mode = 'green'] = process.argv.slice(2)
assert(binaryDir && evidence && path.isAbsolute(binaryDir) && path.isAbsolute(evidence))
const baseEnv = { ...process.env }
for (const key of Object.keys(baseEnv)) if (key.startsWith('CHARIOX_')) delete baseEnv[key]
const children = []
const samples = []
const steps = []
const secrets = [randomBytes(32).toString('hex')]
const issuer = 'byomctx-owner-drill', realm = 'byomctx-owner-realm', user = 'byomctx-owner', account = 'byomctx-account'
const ownerProfiles = new Map(), tickets = new Map(), presences = new Map()
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
let root, workRoot, cloud, source, target, tui, automationSocket, faultContext
let failed = null, cleanup = false, faultInjected = false
const sanitize = text => secrets.reduce((out, secret) => out.replaceAll(secret, '[redacted]'), text)
  .replace(/eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, '[redacted-jwt]')
async function sha256(file) {
  const hash=createHash('sha256');for await(const chunk of createReadStream(file))hash.update(chunk);return hash.digest('hex')
}
async function sample() {
  const mem = Number((await readFile('/proc/meminfo','utf8')).match(/MemAvailable:\s+(\d+)/)[1])*1024
  // Linux statfs avoids invoking a shell or printing unrelated process state.
  const { statfs } = await import('node:fs/promises'); const disk = await statfs('/')
  const row = { at: new Date().toISOString(), memAvailable: mem, diskFree: disk.bavail*disk.bsize }
  samples.push(row); assert(mem >= 9*1024**3 && row.diskFree >= 10*1024**3, 'MP-10 resource floor')
}
async function port() {
  const server = net.createServer(); await new Promise(r=>server.listen(0,'127.0.0.1',r))
  const value = server.address().port; await new Promise(r=>server.close(r)); return value
}
function start(name, command, args, env) {
  const child = spawnOwned(command,args,{cwd:repo,env,detached:true,retainLeader:true,stdio:['pipe','pipe','pipe']})
  child.output = ''; child.name = name; child.command = command; child.args = args; children.push(child)
  child.stdout.on('data',b=>{child.output+=sanitize(b.toString())})
  child.stderr.on('data',b=>{child.output+=sanitize(b.toString())})
  child.done = new Promise((resolve,reject)=>{child.once('error',reject);child.once('close',(code,signal)=>resolve({code,signal}))})
  return child
}
async function run(name, command, args, env, input) {
  const child=start(name,command,args,env); child.stdin.end(input ?? '')
  const result=await child.done
  await writeFile(path.join(evidence,`${name}.log`),child.output)
  steps.push({mpItems:['MP-08','MP-10','MP-11'],name,command,args,exit:result.code,signal:result.signal})
  return {...result,output:child.output}
}
async function stop(child) {
  if (!child) return
  const remaining=()=>processSnapshot().filter(row=>row.sid===child.pid && !['Z','X'].includes(row.state))
  // MP-11: the retained setsid leader pins every descendant; the shared guard
  // validates PID/start identities before either a positive or group signal.
  if (child.exitCode === null) {
    for(const row of remaining().filter(row=>row.pid!==child.pid))signalOwnedProcess(row.pid,'SIGTERM')
    signalOwnedProcessGroup(child,'SIGTERM')
    await Promise.race([child.done,sleep(5000)])
  }
  for(const row of remaining().filter(row=>row.pid!==child.pid))signalOwnedProcess(row.pid,'SIGKILL')
  if (child.exitCode === null) { signalOwnedProcessGroup(child,'SIGKILL'); await child.done }
  await until(()=>remaining().length===0,`${child.name} owned session cleanup`,5000)
}
async function until(fn, label, timeout=45000) {
  const deadline=Date.now()+timeout; let error
  while(Date.now()<deadline) { try { const value=await fn(); if(value) return value } catch(e){error=e} await sleep(100) }
  throw new Error(`MP-10 ${label} timed out${error ? ': '+sanitize(error.message) : ''}`)
}
async function rpc(kernel, request) {
  // Inspect only public status/inventory. Mutations below use real CLI commands.
  const old=process.env.CHARIOX_HOME; process.env.CHARIOX_HOME=kernel.env.CHARIOX_HOME
  const client=new LocalIpcClient(kernel.url)
  try {const response=await client.send(request); if(response.Error) throw new Error(JSON.stringify(response.Error)); return response}
  finally {await client.close(); if(old===undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME=old}
}
async function automation(request) {
  return await new Promise((resolve,reject)=>{
    const socket=net.createConnection(automationSocket); let buf=''
    socket.setTimeout(15000,()=>socket.destroy(new Error('automation timeout')))
    socket.on('error',reject); socket.on('connect',()=>socket.write(JSON.stringify({id:1,...request})+'\n'))
    socket.on('data',chunk=>{buf+=chunk;const end=buf.indexOf('\n');if(end<0)return;socket.end();const reply=JSON.parse(buf.slice(0,end));reply.ok?resolve(reply.data):reject(new Error(reply.error))})
  })
}
async function capture(name) {
  const snapshot=await automation({action:'snapshot'})
  await writeFile(path.join(evidence,`${name}-snapshot.json`),JSON.stringify(snapshot,null,2))
  const terminal=path.join(evidence,`${name}-terminal.log`); await writeFile(terminal,tui.output)
  const result=await run(`${name}-render`,'python3',[path.join(repo,'apps/cli/scripts/lib/owner-context-terminal.py'),'--render',terminal,path.join(evidence,`${name}.png`)],source.env)
  assert.equal(result.code,0)
  return snapshot
}
function jwt(body) {
  const encode=value=>Buffer.from(JSON.stringify(value)).toString('base64url')
  const now=Math.floor(Date.now()/1000)
  const claims={iss:issuer,sub:body.subject,subject_kind:body.subjectKind,realm_id:realm,
    allowed_actions:body.allowedActions??['daemon.register','daemon.heartbeat','packet.route','peer.request','peer.event','client.metadata.read','client.connect'],
    allowed_targets:body.allowedTargets??null,iat:now-1,exp:now+3600,jti:randomBytes(12).toString('hex'),account_id:account,
    organization_id:null,user_id:user,device_id:body.subject,machine_id:body.machineId,client_id:null,
    public_key_thumbprint:body.publicKeyThumbprint,entitlements_version:'drill'}
  const data=encode({alg:'HS256',typ:'JWT'})+'.'+encode(claims)
  return data+'.'+createHmac('sha256',secrets[0]).update(data).digest('base64url')
}
async function handleCloud(req,res) {
  try {
    let raw='';for await (const b of req) {raw+=b;assert(raw.length<256*1024)}
    const body=raw?JSON.parse(raw):{}
    let response
    if(req.url==='/auth/device/poll') {
      assert(body.ticket && body.kernelId && body.publicKeyThumbprint)
      const credential=randomBytes(32).toString('hex'); secrets.push(credential)
      const profile={email:'owner@example.test',accountId:account,userId:user,accountSlug:'owner',realmId:realm,
        relayUrl:relayUrl,issuerId:issuer,kernelId:body.kernelId,machineId:body.machineId,publicKeyThumbprint:body.publicKeyThumbprint}
      ownerProfiles.set(body.kernelId,{profile,credential})
      response={status:'approved',profile,kernelCredential:credential}
    } else if(req.url==='/relay/token') {
      const actor=[...ownerProfiles.values()].find(actor=>actor.credential===body.kernelCredential)
      assert(actor,'token requester must be an enrolled kernel')
      assert.equal(body.accountId,account);assert.equal(body.userId,user);assert.equal(body.realmId,realm)
      if(body.subjectKind==='kernel')assert.equal(body.subject,actor.profile.kernelId)
      else {assert.equal(body.subjectKind,'client');assert.equal(body.subject,`relay-inventory:${actor.profile.kernelId}`)}
      assert.equal(body.publicKeyThumbprint,actor.profile.publicKeyThumbprint)
      response={token:jwt(body),expiresAt:new Date(Date.now()+3600_000).toISOString()}
    } else if(req.url.startsWith('/relay/targets?')) {
      response={targets:[]}
    } else if(req.url==='/kernels/presence') {
      assert.equal(ownerProfiles.get(body.kernelId)?.credential,body.kernelCredential)
      presences.set(body.kernelId,{kernelId:body.kernelId,machineId:body.machineId,status:body.status,metadata:body.metadata})
      response={ok:true}
    } else if(req.url==='/v1/owner-managed-kernels/context/ticket') {
      assert.equal(ownerProfiles.get(body.kernelId)?.credential,body.kernelCredential)
      assert.equal(body.accountId,account); assert.equal(body.userId,user)
      if(body.admission==='source') {
        response={contextPlan:body.contextPlan,target:body.target}
        tickets.set(body.contextPlan.contextId,response)
      } else {
        response=tickets.get(body.contextId); assert(response)
        assert.equal(body.source.kernelId,source.identity.kernelId)
        assert.equal(body.source.keyThumbprint,source.identity.publicKeyThumbprint)
        assert.equal(body.source.userId,user)
        assert.equal(body.source.relayRealmId,realm)
        assert.equal(body.kernelId,target.identity.kernelId)
        assert.equal(body.keyThumbprint,target.identity.publicKeyThumbprint)
        if(faultContext===body.contextId && !faultInjected) {
          const filename=path.join(source.outbound,'.operations',`${faultContext}.json`)
          await rename(filename,filename+'.saved'); await mkdir(filename); faultInjected=true
          steps.push({name:'inject-source-status-storage-failure',mpItems:['MP-08','MP-10','MP-11']})
        }
      }
    } else if(req.url.startsWith('/kernels') || req.url.startsWith('/v1/')) {
      response={kernels:[],machines:[],items:[]}
    } else throw new Error('unexpected Cloud path '+req.url)
    res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify(response))
  } catch(e) {steps.push({name:'cloud-fixture-refusal',path:req.url,message:sanitize(e.message)});res.writeHead(403,{'content-type':'application/json'});res.end('{"code":"admission_refused"}')}
}
let relayUrl
async function kernel(name) {
  const home=path.join(workRoot,name+'-home'), state=path.join(root,name,'state');await mkdir(home,{recursive:true});await mkdir(state,{recursive:true})
  const kp=await port(),mp=await port()
  const env={...baseEnv,HOME:home,CHARIOX_HOME:state,CHARIOX_KERNEL_PORT:String(kp),CHARIOX_MCP_PORT:String(mp),
    CHARIOX_OPENCODE_PORT:String(await port()),CHARIOX_CODEX_PORT:String(await port()),
    CHARIOX_LOG_DIR:path.join(root,name,'logs'),CHARIOX_TEST_TUI:'1',
    BUN_BIN:'/var/lib/chariox/dev/toolchains/bun/bin/bun',PATH:'/var/lib/chariox/dev/toolchains/bun/bin:'+process.env.PATH,
    CODEX_HOME:path.join(home,'.codex'),CLAUDE_CONFIG_DIR:path.join(home,'.claude'),OPENCODE_CONFIG_DIR:path.join(home,'.opencode'),
    XDG_CONFIG_HOME:path.join(home,'.config'),XDG_STATE_HOME:path.join(home,'.local/state'),XDG_DATA_HOME:path.join(home,'.local/share')}
  for(const key of ['CHARIOX_RELAY_URL','CHARIOX_RELAY_TOKEN','CHARIOX_CLOUD_RELAY_CONFIG_PATH','CHARIOX_CLOUD_RELAY_CONFIG_JSON','CHARIOX_KERNEL_LOCAL_AUTH_TOKEN','CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE'])delete env[key]
  const enrolled=await run(`${name}-enroll`,path.join(binaryDir,'chariox-kernel'),['--owner-managed-enroll-stdin'],env,JSON.stringify({ticket:'synthetic-one-time-ticket',apiUrl:cloudUrl,userId:user}))
  assert.equal(enrolled.code,0);const identity=JSON.parse(enrolled.output.trim())
  const child=start(name,path.join(binaryDir,'chariox-kernel'),[],env);child.stdin.end()
  // MP-10: default state.path is ~/.chariox/state/kernel.db, resolved through CHARIOX_HOME.
  const result={name,env,identity,child,url:`ws://127.0.0.1:${kp}`,outbound:path.join(state,'state','managed-context-outbound')}
  await until(async()=> (await rpc(result,{RelayStatus:{}})).RelayStatus?.status.connected,`${name} relay connected`)
  return result
}
let cloudUrl
try {
  await mkdir(evidence,{recursive:true}); await sample()
  const scratch='/root/.chariox/dev/browser-resume-20260930';await mkdir(scratch,{recursive:true});root=await mkdtemp(path.join(scratch,'byomctx-r5-live-'))
  workRoot=await mkdtemp('/root/work/agent-byomctx-r5-live-')
  cloud=createServer((req,res)=>{void handleCloud(req,res)})
  await new Promise(r=>cloud.listen(0,'127.0.0.1',r));cloudUrl=`http://127.0.0.1:${cloud.address().port}`
  const rp=await port();relayUrl=`ws://127.0.0.1:${rp}`
  const relay=start('relay',path.join(binaryDir,'chariox-relay'),[],{...baseEnv,CHARIOX_RELAY_HOST:'127.0.0.1',CHARIOX_RELAY_PORT:String(rp),CHARIOX_RELAY_SCOPED_ISSUER:issuer,CHARIOX_RELAY_SCOPED_HMAC_SECRET:secrets[0]});relay.stdin.end()
  source=await kernel('source');target=await kernel('target')
  await until(()=>presences.get(target.identity.kernelId)?.metadata?.relay_public_key,'target public presence')
  const workspace=path.join(workRoot,'source-project');await mkdir(workspace)
  const ordinary='set -euo pipefail\npython -u worker.py\ngit add -u\n'
  await writeFile(path.join(workspace,'bootstrap.sh'),ordinary)
  const encodedMode=mode.startsWith('utf16-')
  const kernelOnly=mode==='kernel-only'
  const packagedMode=mode.startsWith('packaged-')||kernelOnly
  const structuredMode=mode==='structured-header'||mode==='packaged-metadata'
  const packagedAttack=mode==='packaged-shell'
  const packagedFile=packagedAttack?'bootstrap.sh':'package.json'
  const packagedBytes=packagedAttack?Buffer.from("curl --us\\er owner:synthetic-canary https://example.test\n'"):Buffer.from('{"dependencies":{"js-tokens":"^4.0.0"}}')
  if(packagedMode) {
    const skill=path.join(source.env.HOME,'.chariox','skills','review');await mkdir(skill,{recursive:true})
    await writeFile(path.join(skill,'SKILL.md'),'---\nname: review\ndescription: Review code\n---\nRead the diff.\n')
    await writeFile(path.join(skill,packagedFile),packagedBytes)

  }
  if(structuredMode) {
    const mcps=path.join(source.env.HOME,'.chariox','mcps');await mkdir(path.join(mcps,'local'),{recursive:true})
    await writeFile(path.join(mcps,'public.json'),JSON.stringify({name:'public',transport:{type:'streamable_http',url:'https://example.test/mcp',http_headers:{'X-Context':'synthetic-slot-canary'}}}))
    await writeFile(path.join(mcps,'local','run.sh'),'#!/bin/sh\necho ready\n');await chmod(path.join(mcps,'local','run.sh'),0o755)
    await writeFile(path.join(mcps,'local.json'),JSON.stringify({name:'local',transport:{type:'stdio',command:'run.sh',env:{ORDINARY_LABEL:'synthetic-slot-canary'}}}))
  }
  const utf16=text=>Buffer.concat([Buffer.from([0xff,0xfe]),Buffer.from(text,'utf16le')])
  const canary=utf16("$env:API_KEY = 'synthetic-canary'\n")
  const safeScript=utf16("Write-Output 'MP-08 ordinary 🦀'\n")
  const metadata={
    'package.json':JSON.stringify({dependencies:{'js-tokens':'^4.0.0','secret-tool':'^1.0.0'},devDependencies:{'@example/tokenizer':'^2.0.0'}}),
    'package-lock.json':JSON.stringify({lockfileVersion:3,packages:{'':{dependencies:{'js-tokens':'^4.0.0'}},'node_modules/js-tokens':{version:'4.0.0',resolved:'https://registry.npmjs.org/js-tokens/-/js-tokens-4.0.0.tgz',integrity:'sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='}}}),
    'usage.json':JSON.stringify({input_tokens:42,output_tokens:7,total_tokens:49,token_count:49,max_tokens:4096})
  }
  if(mode==='utf16-history')await writeFile(path.join(workspace,'bootstrap.ps1'),canary)
  if(!encodedMode)for(const [name,contents] of Object.entries(metadata))await writeFile(path.join(workspace,name),contents)
  for(const args of [['init','-b','main'],['config','user.name','Chariox Drill'],['config','user.email','drill@example.test'],['add','.'],['commit','-m','ordinary setup']]) {
    const result=await run(`git-${steps.length}`,'git',['-C',workspace,...args],source.env);assert.equal(result.code,0)
  }
  if(mode==='utf16-history') {
    await writeFile(path.join(workspace,'bootstrap.ps1'),safeScript)
    for(const args of [['add','bootstrap.ps1'],['commit','-m','replace encoded fixture']]) {
      assert.equal((await run(`git-${steps.length}`,'git',['-C',workspace,...args],source.env)).code,0)
    }
  } else await writeFile(path.join(workspace,'bootstrap.ps1'),encodedMode?canary:safeScript)
  if(!encodedMode) {await mkdir(path.join(workspace,'overlay'));await writeFile(path.join(workspace,'overlay','package.json'),metadata['package.json']);await writeFile(path.join(workspace,'overlay','package-lock.json'),metadata['package-lock.json'])}
  await writeFile(path.join(workspace,'README.md'),'MP-05 owner context overlay\n')
  automationSocket=path.join(root,'automation.sock')
  tui=start('source-tui','python3',[path.join(repo,'apps/cli/scripts/lib/owner-context-terminal.py'),path.join(binaryDir,'chariox-cli'),
    '--kernel-url',source.url,'--automation-socket',automationSocket,'--create-session','--workspace',workspace,'--worktree',workspace,'--provider','codex','--model','gpt-5.5'],source.env)
  await until(async()=>{await access(automationSocket);const v=await automation({action:'snapshot'});return v.session?.id&&v},'real CLI Project creation',90000)
  const snapshot=await capture('01-enrolled-project')
  const session=(await rpc(source,{GetSessionState:{session_id:snapshot.session.id}})).SessionState.session
  assert.equal(session.owner_user_id,user)
  const inventory=await rpc(source,{GetWaitingRoomInventory:{}})
  assert(inventory.WaitingRoomInventory.snapshot.projects.some(project=>project.id === session.project_id))
  await writeFile(path.join(evidence,'01-source-inventory.json'),JSON.stringify(inventory,null,2))
  const selection={target:{relayRealmId:realm,machineId:target.identity.machineId,kernelId:target.identity.kernelId,
    relayPublicKey:presences.get(target.identity.kernelId).metadata.relay_public_key,keyThumbprint:target.identity.publicKeyThumbprint},
    contextSelection:{kernelContext:(packagedMode||structuredMode)?'source_kernel_without_credentials':'empty',developmentSetup:kernelOnly?{kind:'empty'}:{kind:'source_project',projectId:session.project_id,
      repositories:[{role:'primary',workspaceId:session.workspace_id,worktreeId:null}]}}}
  const selectionPath=path.join(root,'selection.json');await writeFile(selectionPath,JSON.stringify(selection))
  async function cli(name,kernel,args) {
    const result=await run(name,path.join(binaryDir,'chariox-cli'),['context',...args,'--kernel-url',kernel.url],kernel.env)
    assert.equal(result.code,0,`MP-08 ${name}: ${result.output}`);return JSON.parse(result.output.trim())
  }
  async function copy(name, inject=false, cancel=false) {
    const start=await cli(name+'-start',source,['copy',selectionPath]);const initial=start.ManagedContextTransferStarted.status
    if(inject)faultContext=initial.contextId
    const prefix=kernelOnly?'owner-context:':'project-environment:'
    const review=await until(async()=> {
      const snapshot=await automation({action:'snapshot'})
      if(snapshot.interactions?.some(i=>i.id.startsWith(prefix)))return snapshot
      const status=(await cli(name+'-before-review-'+steps.length,source,['status',initial.contextId])).ManagedContextTransferStatus.status
      if(status.phase!=='preparing'||status.packageSizeBytes>0)return {premature:status}
    },'compulsory native owner review')
    if(review.premature){await capture(name+'-missing-review');assert.fail('MP-11 owner copy packaged or sent before compulsory native confirmation')}
    await capture(name+'-review')
    if(kernelOnly) {
      assert(review.interactions.some(i=>i.title?.includes(target.identity.machineId)),'MP-11 review names destination')
      await sleep(500)
      const pending=(await cli(name+'-pending',source,['status',initial.contextId])).ManagedContextTransferStatus.status
      assert.equal(pending.phase,'preparing');assert.equal(pending.packageSizeBytes,0);assert.equal(pending.acceptedBytes,0);assert.equal(pending.receipt??null,null)
      await assert.rejects(access(path.join(source.outbound,initial.contextId)))
      await assert.rejects(access(path.join(target.env.HOME,'.chariox','skills','review',packagedFile)))
      steps.push({name:name+'-no-package-or-publication-before-approval',mpItems:['MP-08','MP-10','MP-11'],status:pending})
    }
    await automation({action:'interaction_submit',choiceIndex:cancel?1:0})
    const terminal=await until(async()=>{
      const status=(await cli(name+'-poll-'+steps.length,source,['status',initial.contextId])).ManagedContextTransferStatus.status
      return (status.phase === 'completed' || (status.phase === 'failed' && (!inject || status.receipt || !faultInjected))) && status
    },'owner copy terminal status',90000)
    await capture(name+'-result')
    return terminal
  }
  if(kernelOnly) {
    const cancelled=await copy('02-kernel-only-cancel',false,true)
    assert.equal(cancelled.phase,'failed');assert.equal(cancelled.failureCode,'managed_context_review_cancelled');assert.equal(cancelled.retryable,false)
    assert.equal(cancelled.packageSizeBytes,0);assert.equal(cancelled.acceptedBytes,0);assert.equal(cancelled.receipt??null,null)
    await assert.rejects(access(path.join(source.outbound,cancelled.contextId)))
    await assert.rejects(access(path.join(target.env.HOME,'.chariox','skills','review',packagedFile)))
    steps.push({name:'kernel-only-cancellation-prevents-export-and-publication',mpItems:['MP-08','MP-10','MP-11']})
    const approved=await copy('03-kernel-only-approve')
    assert.notEqual(approved.contextId,cancelled.contextId);assert.equal(approved.phase,'completed');assert(approved.receipt)
    const launch=(await cli('04-kernel-only-target-launch',target,['launch-target',approved.contextId,approved.planDigest])).ManagedContextLaunchTarget.target
    assert.equal(launch.development.kind,'empty');assert.equal(approved.receipt.kernelContext.kind,'from_kernel')
    assert.deepEqual(await readFile(path.join(target.env.HOME,'.chariox','skills','review',packagedFile)),packagedBytes)
    await assert.rejects(access(path.join(source.outbound,approved.contextId)))
    steps.push({name:'kernel-only-approved-exact-bytes-published',mpItems:['MP-08','MP-10','MP-11'],launch})
  } else if(encodedMode || packagedAttack) {
    const refused=await copy('02-encoded-refusal')
    assert.equal(refused.phase,'failed','MP-11 encoded credential must be refused on real copy path')
    assert.match(JSON.stringify(refused.error??refused),/credential-free context/)
    assert.equal(refused.receipt??null,null,'MP-11 no destination receipt on source refusal')
    steps.push({name:'encoded-source-refusal-before-transfer',mpItems:['MP-08','MP-10','MP-11'],mode})
  } else {
  // MP-08/MP-10: inject on the first copy into a fresh target. A separate fresh
  // copy of an already-imported Project conflicts with its Workspace binding.
  const broken=await copy('02-storage-fault',true);assert.equal(broken.phase,'failed');assert(broken.retryable&&broken.receipt);assert(faultInjected)
  const launch=(await cli('03-target-launch',target,['launch-target',broken.contextId,broken.planDigest])).ManagedContextLaunchTarget.target
  const imported=launch.development.repositories[0].workspacePath
  assert.equal(await readFile(path.join(imported,'bootstrap.sh'),'utf8'),ordinary)
  assert.equal(await readFile(path.join(imported,'README.md'),'utf8'),'MP-05 owner context overlay\n')
  await writeFile(path.join(evidence,'03-imported-files.json'),JSON.stringify({bootstrapSha256:createHash('sha256').update(ordinary).digest('hex'),overlay:true,launch},null,2))
  await access(path.join(source.outbound,broken.contextId,'managed-context.pkg'))
  await stop(source.child)
  const blocked=path.join(source.outbound,'.operations',`${broken.contextId}.json`);await rm(blocked,{recursive:true});await rename(blocked+'.saved',blocked)
  source.child=start('source-restarted',path.join(binaryDir,'chariox-kernel'),[],source.env);source.child.stdin.end()
  await until(async()=> (await rpc(source,{RelayStatus:{}})).RelayStatus?.status.connected,'source process restart')
  const recovered=await copy('05-recovery');assert.equal(recovered.contextId,broken.contextId);assert.equal(recovered.phase,'completed');assert.deepEqual(recovered.receipt,broken.receipt)
  const targetReceipt=(await cli('06-recovered-target-launch',target,['launch-target',recovered.contextId,recovered.planDigest])).ManagedContextLaunchTarget.target
  assert.equal(targetReceipt.contextId,recovered.contextId)
  await assert.rejects(access(path.join(source.outbound,recovered.contextId)))
  for(const [name,contents] of Object.entries(metadata)) {
    assert.equal(await readFile(path.join(imported,name),'utf8'),contents)
    if(name!=='usage.json')assert.equal(await readFile(path.join(imported,'overlay',name),'utf8'),contents)
  }
  assert.deepEqual(await readFile(path.join(imported,'bootstrap.ps1')),safeScript)
  if(packagedMode) {
    assert.deepEqual(await readFile(path.join(target.env.HOME,'.chariox','skills','review',packagedFile)),packagedBytes)
    steps.push({name:'packaged-skill-bytes-unchanged-in-ordinary-registry',mpItems:['MP-08','MP-10','MP-11'],path:packagedFile})
  }
  if(structuredMode) {
    const config=JSON.parse(await readFile(path.join(target.env.HOME,'.chariox','mcps','public.json'),'utf8'))
    assert.equal(Object.keys(config.transport.http_headers??{}).length,0,'MP-11 header slot omitted by role')
    assert(!JSON.stringify(config).includes('synthetic-slot-canary'))
    steps.push({name:'structured-mcp-header-values-omitted',mpItems:['MP-08','MP-10','MP-11']})
    const stdio=JSON.parse(await readFile(path.join(target.env.HOME,'.chariox','mcps','local.json'),'utf8'))
    assert.equal(Object.keys(stdio.transport.env??{}).length,0,'MP-11 env slot omitted by role')
    assert(!JSON.stringify(stdio).includes('synthetic-slot-canary'))
    steps.push({name:'structured-mcp-env-values-omitted',mpItems:['MP-08','MP-10','MP-11']})
  }
  steps.push({name:'package-lock-counter-and-encoded-overlay-bytes-unchanged',mpItems:['MP-05','MP-08','MP-10','MP-11']})
  }
  await sample()
} catch(e) {failed=sanitize(e.stack??String(e))}
finally {
  for(const child of [...children].reverse()) {try{await stop(child);await writeFile(path.join(evidence,child.name+'-process.log'),child.output)}catch(e){failed??=sanitize(e.message)}}
  if(cloud)await new Promise(r=>cloud.close(r))
  // Only the exact mkdtemp root owned by this run contains disposable runtime keys.
  if(root){assert(path.dirname(root)==='/root/.chariox/dev/browser-resume-20260930'&&path.basename(root).startsWith('byomctx-r5-live-'));await rm(root,{recursive:true,force:true});cleanup=true}
  if(workRoot){assert(path.dirname(workRoot)==='/root/work'&&path.basename(workRoot).startsWith('agent-byomctx-r5-live-'));await rm(workRoot,{recursive:true,force:true})}
  await sample()
  const report={mpItems:['MP-05','MP-08','MP-10','MP-11'],mode,sourceHead:process.env.OWNER_DRILL_SOURCE_HEAD??null,binaryDir,
    result:failed?'RED':'PASS',failure:failed,steps,samples,cleanup,
    processes:children.map(child=>({name:child.name,pid:child.pid,command:child.command,args:child.args,exit:child.exitCode,signal:child.signalCode})),
    harnessSha256:await sha256(fileURLToPath(import.meta.url)),
    binaries:Object.fromEntries(await Promise.all(['chariox-kernel','chariox-cli','chariox-relay'].map(async name=>[name,await sha256(path.join(binaryDir,name))]))),
    client:Object.fromEntries(await Promise.all(['index.js','cli-main.js','context-command.js','kernel-feature-minimum.js'].map(async name=>[name,await sha256(path.join(repo,'apps/cli/dist',name))]))),
    limits:['Cloud control plane is loopback fixture; Cloud web flow is unavailable','No provider run or credential transfer is part of this credential-free copy drill','PNG captures render the real TUI PTY screen']}
  await writeFile(path.join(evidence,'report.json'),JSON.stringify(report,null,2))
  console.log('MP-05/MP-08/MP-10/MP-11 owner context real-path drill',report.result,'cleanup',cleanup)
  if(failed)console.log(failed)
  process.exitCode=failed?1:0
}

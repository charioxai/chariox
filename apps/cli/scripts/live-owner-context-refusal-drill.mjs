#!/usr/bin/env node
// MP-08 / MP-10 / MP-11 BY-05: real Cloud/kernel boundary, built Web DPR1/2 and real TUI.
// The lane-owned API proxy retains capabilities in RAM only. It forwards to the
// actual Cloud service; no fake responses, database writes or provider login changes.
import assert from 'node:assert/strict'
import {createServer} from 'node:http'
import net from 'node:net'
import path from 'node:path'
import {pathToFileURL, fileURLToPath} from 'node:url'
import {mkdir, mkdtemp, open, readFile, writeFile, rm, statfs} from 'node:fs/promises'
import {createHash} from 'node:crypto'
import {LocalIpcClient} from '../../../packages/kernel-client/dist/ipc.js'
import {spawnOwned, signalOwnedProcessGroup, ownedProcessHandles, signalOwnedProcess} from '../../kernel/slice-linux-docker/owned-process-signals.mjs'

const required = key => { assert(process.env[key], `${key} required`); return process.env[key] }
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const E = required('BYOM_EVIDENCE'), state = required('BYOM_STATE_ROOT'), work = required('BYOM_WORK_ROOT')
const origin = required('BYOM_ORIGIN'), relay = required('BYOM_RELAY_URL'), binary = required('BYOM_KERNEL_BINARY'), cli = required('BYOM_CLI_BINARY')
const {chromium} = await import(pathToFileURL(required('PLAYWRIGHT_MODULE')))
const {startHostNetwork} = await import(pathToFileURL(path.join(required('BYOM_CLOUD_ROOT'), 'scripts/lib/project-environment-host-network.mjs')))
assert.equal(new URL(origin).protocol, 'https:'); assert.equal(new URL(relay).protocol, 'wss:')
for (const dir of [E, state, work]) { assert(path.isAbsolute(dir) && !dir.startsWith(repo + '/')); await mkdir(dir, {recursive:true}) }
const scratch = await mkdtemp(path.join(state, 'owner-refusal-')), workspace = await mkdtemp(path.join(work, 'public-project-'))
const children = [], privateLogs = [], confidential = [], cells = [], resources = [], api = []
let browser, network, proxy, mode, contextId, socket, tuiLog, fault, source, target
const report = {mpItems:['MP-08','MP-10','MP-11'], area:'apps-byom/BY-05', source:{oss:required('BYOM_OSS_HEAD'), cloud:required('BYOM_CLOUD_HEAD')}, started:new Date().toISOString(), cells, api, resources, networkDisposals:[], scope:'Real hosted Cloud and encrypted WSS; actual API boundary replay/revocation/short-TTL expiry; refusal before packaging; this does not establish signed C6 installed-target acceptance.'}
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const safe = text => confidential.reduce((value, secret) => value.replaceAll(secret, '[redacted]'), String(text))
async function save() { await writeFile(path.join(E,'RESULT.json'), JSON.stringify(report,null,2)) }
async function sample() {
  const mem = Number((await readFile('/proc/meminfo','utf8')).match(/MemAvailable:\s+(\d+)/)[1])*1024
  const fs = await statfs('/'); resources.push({at:new Date().toISOString(), memAvailable:mem, diskFree:fs.bavail*fs.bsize})
  assert(mem >= 12*1024**3 && fs.bavail*fs.bsize >= 40*1024**3, 'MP-10 resource floor')
}
async function until(fn, label, timeout=60000) {
  const deadline=Date.now()+timeout
  while(Date.now()<deadline) { const value=await fn(); if(value)return value; await pause(150) }
  throw Error(`${label} timed out`)
}
function spawn(command,args,env,stdio=['pipe','pipe','pipe']) {
  const child=spawnOwned(command,args,{cwd:repo,env,stdio,detached:true,retainLeader:true});children.push(child)
  child.done=new Promise((resolve,reject)=>{child.once('error',reject);child.once('close',(code,signal)=>resolve({code,signal}))})
  return child
}
async function run(command,args,env=process.env,input='') {
  const child=spawn(command,args,env);let out='',err=''
  child.stdout.on('data',bytes=>out+=bytes);child.stderr.on('data',bytes=>err+=bytes);child.stdin.end(input)
  const {code}=await child.done;assert.equal(code,0,`${command} exit ${code}: ${safe(err).slice(-300)}`);return out
}
async function stop(child) {
  if(child.exitCode!==null)return
  for(const handle of ownedProcessHandles(child).filter(value=>value.pid!==child.pid))signalOwnedProcess(handle,'SIGTERM')
  signalOwnedProcessGroup(child,'SIGTERM');await Promise.race([child.done,pause(5000)])
  if(child.exitCode===null) { signalOwnedProcessGroup(child,'SIGKILL');await child.done }
}
async function port() {const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p}
async function rpc(kernel, request) {
  const old=process.env.CHARIOX_HOME;process.env.CHARIOX_HOME=kernel.env.CHARIOX_HOME;const client=new LocalIpcClient(kernel.url)
  try {const response=await client.send(request);if(response.Error)throw Error(safe(response.Error.message??'Kernel refused request'));return response}
  finally {await client.close();if(old===undefined)delete process.env.CHARIOX_HOME;else process.env.CHARIOX_HOME=old}
}
async function automation(request) {
  return new Promise((resolve,reject)=>{const s=net.createConnection(socket);let text='';s.setTimeout(15000,()=>s.destroy(Error('TUI automation timeout')));s.on('error',reject);s.on('connect',()=>s.write(JSON.stringify({id:1,...request})+'\n'));s.on('data',b=>{text+=b;const n=text.indexOf('\n');if(n<0)return;s.end();const reply=JSON.parse(text.slice(0,n));reply.ok?resolve(reply.data):reject(Error('TUI action refused'))})})
}
async function forward(url, method, headers, body) {
  const h={};for(const key of ['content-type','x-chariox-kernel-credential'])if(headers[key])h[key]=headers[key]
  const response=await fetch(origin+url,{method,headers:h,body:['GET','HEAD'].includes(method)?undefined:body,redirect:'error'})
  return {status:response.status, body:await response.text(), type:response.headers.get('content-type')??'application/json'}
}
async function apiProxy() {
  const server=createServer(async(req,res)=>{
    try {
      let body='';for await(const chunk of req){body+=chunk;assert(body.length<=256*1024)}
      const consume=req.url==='/v1/managed-kernels/context/ticket' && body && JSON.parse(body).ownerManaged
      const issue=req.url==='/v1/owner-managed-context-tickets' && req.method==='POST'
      if(issue && mode==='expired')body=JSON.stringify({...JSON.parse(body),ttl_seconds:1})
      if(consume) {
        contextId=consume.contextId
        // Exact replay at the real API seam: Cloud accepts/consumes the first
        // request, then receives identical capability bytes again. Only the
        // second response reaches the kernel; no archive was prepared/copied.
        if(mode==='already_used') {
          const first=await forward(req.url,req.method,req.headers,body);assert.equal(first.status,200)
          api.push({mode,step:'first-consumption',httpStatus:first.status,contextId})
        }
        const result=await forward(req.url,req.method,req.headers,body)
        api.push({mode,step:'consume-or-replay-refusal',httpStatus:result.status,code:JSON.parse(result.body).error?.code,contextId})
        mode=null;res.writeHead(result.status,{'content-type':result.type});res.end(result.body);return
      }
      const result=await forward(req.url,req.method,req.headers,body)
      if(issue && result.status===201 && mode) {
        const ticket=JSON.parse(result.body);confidential.push(ticket.ticket)
        api.push({mode,step:'fresh-ticket-issued',ticketId:ticket.ticket_id,expiresAt:ticket.expires_at,httpStatus:201})
        if(mode==='revoked') {
          const revoked=await forward('/v1/owner-managed-context-tickets/'+ticket.ticket_id,'DELETE',req.headers,'')
          assert.equal(revoked.status,204);api.push({mode,step:'product-revoke-before-first-use',httpStatus:204,ticketId:ticket.ticket_id})
        } else if(mode==='expired') {
          await pause(Math.max(0,Date.parse(ticket.expires_at)-Date.now()+150))
          api.push({mode,step:'natural-short-TTL-expiry',clockModified:false})
        }
      }
      res.writeHead(result.status,{'content-type':result.type});res.end(result.body)
    } catch(error) {fault=Error(safe(error.message));res.writeHead(502);res.end('{}')}
  })
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));return {server,url:'http://127.0.0.1:'+server.address().port}
}
async function enroll(page,name,apiUrl) {
  const home=path.join(workspace,name+'-home');await mkdir(home,{mode:0o700})
  const env={...process.env,HOME:home,CHARIOX_HOME:path.join(scratch,name),CHARIOX_KERNEL_PORT:String(await port()),CHARIOX_MCP_PORT:String(await port()),CHARIOX_CODEX_PORT:String(await port()),CHARIOX_OPENCODE_PORT:String(await port()),CHARIOX_DAEMON_ALIAS:'byomrel-r31-'+name,CHARIOX_MACHINE_ALIAS:'byomrel-r31-'+name,CHARIOX_LOG_DIR:path.join(scratch,name+'-logs'),TMPDIR:path.join(scratch,'tmp'),CODEX_HOME:path.join(home,'.codex'),CLAUDE_CONFIG_DIR:path.join(home,'.claude'),XDG_CONFIG_HOME:path.join(home,'.config'),XDG_STATE_HOME:path.join(home,'.local/state'),XDG_DATA_HOME:path.join(home,'.local/share')}
  for(const key of ['CHARIOX_RELAY_URL','CHARIOX_RELAY_TOKEN','CHARIOX_CLOUD_RELAY_CONFIG_JSON','CHARIOX_CLOUD_RELAY_CONFIG_PATH','CHARIOX_KERNEL_LOCAL_AUTH_TOKEN','CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE'])delete env[key]
  await mkdir(env.TMPDIR,{recursive:true})
  await page.getByRole('button',{name:'Add this machine',exact:true}).click();const dialog=page.getByRole('dialog')
  const command=await dialog.getByLabel('Install and enroll command').inputValue();const ticket=command.split('\n')[1];confidential.push(ticket);assert(ticket)
  const identity=JSON.parse((await run(binary,['--owner-managed-self-enroll-stdin'],env,JSON.stringify({ticket,apiUrl}))).trim())
  const kernel={env,identity,url:'ws://127.0.0.1:'+env.CHARIOX_KERNEL_PORT+'/kernel'}
  const log=await open(path.join(scratch,name+'.log'),'w',0o600);privateLogs.push(path.join(scratch,name+'.log'));spawn(binary,[],env,['ignore',log.fd,log.fd]);await log.close()
  await until(async()=> (await rpc(kernel,{RelayStatus:{}})).RelayStatus.status.connected, name+' hosted WSS')
  await dialog.getByText(/^Connected:/).waitFor({timeout:90000});await dialog.getByRole('button',{name:'Close',exact:true}).click();return kernel
}
const messages={revoked:'This copy approval was revoked by the owner. Ask for a new approval.',expired:'This copy approval expired. Start a new copy to request approval.',already_used:'This copy approval was already used. Start a new copy to request approval.'}
try {
  await sample();await mkdir(path.join(scratch,"tmp"),{recursive:true});proxy=await apiProxy();network=await startHostNetwork(relay,{httpsOrigins:[origin]})
  browser=await chromium.launch({headless:true,executablePath:required('BYOM_CHROMIUM'),proxy:{server:network.proxyUrl},args:['--no-sandbox','--disable-dev-shm-usage'],env:{...process.env,TMPDIR:path.join(scratch,'tmp')}})
  const first=await browser.newContext({deviceScaleFactor:1,viewport:{width:1440,height:1000}});const page=await first.newPage();network.attach(page)
  await page.goto(origin+'/validation/ready');await page.goto(origin+'/terminals');await page.getByRole('heading',{name:'My kernels',exact:true}).waitFor({timeout:60000})
  source=await enroll(page,'source',proxy.url);target=await enroll(page,'target',origin)
  const publicRepo=path.join(workspace,'slugify');await run('git',['clone','--quiet','https://github.com/sindresorhus/slugify.git',publicRepo]);await run('git',['-C',publicRepo,'checkout','--quiet','3b17b2e84b97624a683aafaa38184bf2746fab22'])
  const linked=(await rpc(source,{LinkProviderAccountProfile:{provider:'codex',label:'byomrel-r31',path:required('BYOM_CODEX_PROFILE')}})).ProviderAccountProfile.profile
  await rpc(source,{SetDefaultProviderAccountProfile:{provider:'codex',account_profile:linked.profile_id}})
  const session=(await rpc(source,{CreateSession:{workspace_id:publicRepo,worktree_id:publicRepo,alias:'byomrel-r31-copy',project_selection:{kind:'new'}}})).SessionCreated.session
  socket=path.join(scratch,'tui.sock');tuiLog=path.join(scratch,'tui.log');const log=await open(tuiLog,'w',0o600)
  spawn('python3',[path.join(repo,'apps/cli/scripts/lib/owner-context-terminal.py'),cli,'--kernel-url',source.url,'--automation-socket',socket,'--session',session.id,'--workspace',publicRepo,'--worktree',publicRepo,'--provider','codex','--account-profile',linked.profile_id,'--model','codex/gpt-6.1-sol'],source.env,['pipe',log.fd,log.fd]);await log.close()
  await until(async()=>{try{return (await automation({action:'snapshot'})).session?.id}catch{return false}},'real TUI ready')
  report.networkDisposals.push({stage:'before-enrollment-context-close',snapshot:await network.snapshot()})
  await first.close();await pause(150)
  report.networkDisposals.push({stage:'after-enrollment-context-close',snapshot:await network.snapshot()})
  const before=(await rpc(target,{ListProjects:{}})).ProjectsListed.projects
  for(const dpr of [1,2]) {
    const ctx=await browser.newContext({deviceScaleFactor:dpr,viewport:{width:1440,height:1000}}), page=await ctx.newPage();network.attach(page)
    page.on('pageerror',()=>{report.pageErrors=(report.pageErrors??0)+1})
    await page.goto(origin+'/validation/ready')
    for(const reason of Object.keys(messages)) {
      await sample();mode=reason;contextId=null;fault=null
      const cell={dpr,reason,status:'FAIL'};cells.push(cell)
      await page.goto(origin+'/terminals');await page.getByRole('heading',{name:'My kernels',exact:true}).waitFor()
      const group=page.locator('.kernel-account-group').filter({has:page.getByText(target.identity.kernelId,{exact:true})})
      await until(()=>group.getByRole('button',{name:'Copy a kernel here',exact:true}).isEnabled(),'Copy enabled')
      await group.getByRole('button',{name:'Copy a kernel here',exact:true}).click();const dialog=page.getByRole('dialog',{name:'Copy a kernel here',exact:true})
      const peers=await page.evaluate(async()=> (await(await fetch('/v1/owner-managed-context-tickets/peers')).json()).peers)
      await dialog.getByLabel('Source kernel',{exact:true}).selectOption(peers.find(p=>p.peer.kernelId===source.identity.kernelId).targetId)
      await dialog.getByLabel('Project to copy').selectOption(session.project_id)
      await dialog.getByLabel('Include kernel context, without credentials').uncheck()
      await page.screenshot({path:path.join(E,`dpr${dpr}-${reason}-before.png`),scale:'device'})
      await dialog.getByRole('button',{name:'Review and copy',exact:true}).click()
      await dialog.getByText('Copy stopped.',{exact:true}).waitFor({timeout:60000});if(fault)throw fault
      assert(contextId);const status=(await rpc(source,{GetManagedContextTransferStatus:{contextId}})).ManagedContextTransferStatus.status
      cell.authoritativeStatus=status;cell.visibleText=await dialog.innerText()
      assert.equal(status.phase,'failed');assert.equal(status.acceptedBytes,0);assert.equal(status.packageSizeBytes,0);assert(!status.receipt);assert.equal(status.retryable,false)
      assert.deepEqual((await rpc(target,{ListProjects:{}})).ProjectsListed.projects,before);cell.targetUnchanged=true
      cell.webSpecific=cell.visibleText.includes(messages[reason]);await page.screenshot({path:path.join(E,`dpr${dpr}-${reason}-refused.png`),scale:'device'})
      const response=await automation({action:'submit_prompt',prompt:'/kernel copy-status '+contextId});await pause(300)
      const snapshot=await automation({action:'snapshot'});cell.tuiSpecific=JSON.stringify(snapshot).includes(messages[reason])||JSON.stringify(response).includes(messages[reason])
      const terminal=path.join(E,`dpr${dpr}-${reason}-tui.log`);await writeFile(terminal,safe(await readFile(tuiLog,'utf8')))
      await run('python3',[path.join(repo,'apps/cli/scripts/lib/owner-context-terminal.py'),'--render',terminal,path.join(E,`dpr${dpr}-${reason}-tui.png`)])
      cell.failureCodeSpecific=status.failureCode==='owner_context_authorization_'+reason
      cell.network=await network.snapshot()
      cell.status=cell.webSpecific&&cell.tuiSpecific&&cell.failureCodeSpecific?'PASS':'FAIL';await save()
      console.log('MP-08 / MP-10 / MP-11 BY-05',reason,'DPR'+dpr,cell.status)
      await dialog.getByRole('button',{name:'Close',exact:true}).click()
    }
    report.networkDisposals.push({stage:'before-dpr-context-close',dpr,snapshot:await network.snapshot()})
    await ctx.close();await pause(150)
    report.networkDisposals.push({stage:'after-dpr-context-close',dpr,snapshot:await network.snapshot()})
  }
  report.network=await network.snapshot();assert(report.network.authenticated && report.network.calibration.passed && report.network.hostedRelayObserved)
  report.status=cells.every(cell=>cell.status==='PASS')?'PASS':'FAIL';if(report.status==='FAIL')process.exitCode=1
} catch(error) {report.status='FAIL';report.error=safe(error.message).split('Call log:')[0].slice(0,1200);process.exitCode=1}
finally {
  if(browser)await browser.close();if(network)report.networkCleanup=await network.close()
  for(const child of children.reverse())await stop(child)
  if(proxy){proxy.server.closeAllConnections();await new Promise(resolve=>proxy.server.close(resolve))}
  confidential.length=0
  await rm(scratch,{recursive:true,force:true});await rm(workspace,{recursive:true,force:true})
  report.cleanup={ownedProcessesStopped:true,ownedScratchRemoved:true,capabilitiesRetained:false};report.finished=new Date().toISOString();await sample();await save()
  console.log('MP-08 / MP-10 / MP-11 BY-05 drill',report.status,report.error??'')
}

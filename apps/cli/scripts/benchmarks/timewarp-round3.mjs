#!/usr/bin/env node
// MP-08/MP-10/MP-11: frozen TimeWarp policy on the round3 controller; no submission.
import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { readFile, writeFile, mkdir, cp, appendFile, rm } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { createInterface } from 'node:readline'
import { promisify } from 'node:util'
import net from 'node:net'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { startOwnedRuntime } from './round2/runtime.mjs'
import { Round2Room } from './round2/room.mjs'
import { stopOwnedProcess } from './round2/owned-processes.mjs'
import { openKernelClient, unwrap, loadTurnHistory, waitForSettlement } from './round2/kernel.mjs'
import { writeEvidence, settlementRecord } from './round2/export.mjs'
import { observeKernelRpcErrors } from './round2/rpc-errors.mjs'
import { auditTimeWarpHistory } from './timewarp-history.mjs'
import { permittedBrowserTool } from './round2/browser-track.mjs'
import { verifyTimeWarpContinuation } from './timewarp-continuation.mjs'
import { sanitizeDrillMetadata } from '../lib/drill-secrets.mjs'

const options = JSON.parse(await readFile(process.argv[2], 'utf8'))
const { evidence, lane, image } = options.runtime
const scripts = import.meta.dirname
const exec = promisify(execFile), pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const hash = async file => createHash('sha256').update(await readFile(file)).digest('hex')
const port = () => new Promise(resolve => {
  const server = net.createServer()
  server.listen(0, '127.0.0.1', () => { const value = server.address().port; server.close(() => resolve(value)) })
})
const report = { mpItems:['MP-08','MP-10','MP-11'], benchmark:'TimeWarp', round:3, seed:42, repeat:1,
  model:'gpt-6.1-sol', effort:'high', round1Effort:'low', wallTimeoutMs:180000, maxMutatingActions:60,
  persistentProviderThread:true, noPublicSubmission:true, officialScore:null, attempts:[], resources:[],
  startedAt:new Date().toISOString(), completed:false }
let runtime, api, room, grader, shopContainer, shopPrepared=false, currentEra=null, currentShop=false
let stage='frozen_inputs', interrupted=false, siteChildren=[], siteUrls, graderCall, activeRow
const children=[]
const runId=options.runtime.label
assert.match(runId,/^r2next-timewarp-[a-z0-9-]+$/)
const signal=()=>{interrupted=true}
process.on('SIGTERM',signal);process.on('SIGINT',signal)
await mkdir(evidence,{recursive:true,mode:0o700})
// A restarted process may not overwrite admissions or replay an ambiguous prompt.
const prior=await readFile(`${evidence}/CAMPAIGN.json`,'utf8').catch(error=>{if(error.code==='ENOENT')return null;throw error})
assert.equal(prior,null,'MP-10 fresh campaign output required; preserve prior attempts')
const checkpoint=async()=>{
  await writeEvidence(evidence,'CAMPAIGN.json',report)
  await writeEvidence(evidence,'RESULTS.json',report.attempts)
}
async function command(program,args,timeout=120000) {
  const started=Date.now()
  try {
    const result=await exec(program,args,{timeout,maxBuffer:16*1024**2})
    await appendFile(`${evidence}/commands.jsonl`,JSON.stringify({program,args,exitCode:0,elapsedMs:Date.now()-started})+'\n')
    return result.stdout.trim()
  } catch(error) {
    await appendFile(`${evidence}/commands.jsonl`,JSON.stringify({program,args,exitCode:error.code??null,elapsedMs:Date.now()-started})+'\n')
    throw Error(`MP-10 ${program} command failed (${error.code??error.name})`)
  }
}
const docker=args=>command('docker',args)
async function guard() { if(interrupted)throw Error('MP-10 cooperative interruption'); await runtime.guard() }
async function launch(program,args,label,{cwd=runtime.root,env={PATH:process.env.PATH,LANG:'C.UTF-8',HOME:`${runtime.root}/home`},interactive=false}={}) {
  const child=spawn(program,args,{cwd,env,detached:true,stdio:[interactive?'pipe':'ignore','pipe','pipe']})
  await new Promise((resolve,reject)=>{child.once('spawn',resolve);child.once('error',reject)})
  assert(Number.isSafeInteger(child.pid)&&child.pid>1)
  const log=createWriteStream(`${runtime.root}/${label}.log`,{mode:0o600})
  child.stdout.pipe(log);child.stderr.pipe(log);children.push(child)
  return child
}
function graderRequests(child) {
  let pending=null, poisoned=false
  createInterface({input:child.stdout}).on('line',line=>{
    if(!pending)return
    const {resolve,reject,timer}=pending;pending=null;clearTimeout(timer)
    try { const result=JSON.parse(line);assert(result.ok,`MP-10 grader ${result.operation} ${result.error_class}`);resolve(result) }
    catch(error){reject(error)}
  })
  child.on('exit',()=>{poisoned=true;if(pending){clearTimeout(pending.timer);pending.reject(Error('MP-10 grader exited'));pending=null}})
  return request=>new Promise((resolve,reject)=>{
    assert(!poisoned&&!pending,'MP-10 grader channel unavailable')
    const timer=setTimeout(()=>{poisoned=true;pending=null;reject(Error('MP-10 grader deadline; do not reuse response stream'))},120000)
    pending={resolve,reject,timer};child.stdin.write(JSON.stringify(request)+'\n')
  })
}
async function ready(child,url) {
  const deadline=Date.now()+180000
  while(Date.now()<deadline) {
    await guard();assert(child.exitCode===null&&child.signalCode===null,'MP-10 fixture exited')
    try { if((await fetch(url,{signal:AbortSignal.timeout(1000)})).ok)return }catch{}
    await pause(500)
  }
  throw Error('MP-10 fixture readiness deadline')
}
async function stopSites() {
  for(const child of siteChildren) {
    if(child.shop) {
      child.stdin.end()
      const deadline=Date.now()+10000
      while(child.exitCode===null&&child.signalCode===null&&Date.now()<deadline)await pause(100)
      assert(child.exitCode===0,'MP-11 Shop supervisor did not acknowledge owned server shutdown')
    } else assert(await stopOwnedProcess(child,{detached:true}),'MP-11 fixture cleanup failed')
  }
  siteChildren=[]
}
async function startSites(era,needsShop) {
  if(era===currentEra&&needsShop===currentShop&&!needsShop)return
  await stopSites();await guard()
  siteUrls={webshop:'http://127.0.0.1:9/abc'}
  for(const site of ['wiki','news']) {
    const siteRoot=`${runtime.root}/sites/${site}`
    await mkdir(siteRoot,{recursive:true,mode:0o700})
    await cp(`${options.upstream}/env/${site}`,siteRoot,{recursive:true})
    await cp(`${options.assets}/${site}_index.pkl`,`${siteRoot}/${site}_index.pkl`)
    const sitePort=await port()
    const child=await launch(options.sitePython,[`${scripts}/timewarp-site.py`,siteRoot,site,String(era),String(sitePort)],`${site}-era-${era}`)
    siteChildren.push(child);siteUrls[site]=`http://host.docker.internal:${sitePort}`
    await ready(child,`http://127.0.0.1:${sitePort}`)
  }
  if(needsShop) {
    if(!shopPrepared) {
      stage='official_shop_setup';await guard()
      shopContainer=`chariox-slice-benchtw-${runId}-shop-site`
      await docker(['run','-d','--name',shopContainer,'--label',`io.chariox.r2next.timewarp=${runId}`,
        '--memory','2g','--memory-swap','2g','--cpus','1','--pids-limit','512','--entrypoint','sleep',image,'infinity'])
      report.shopContainer=shopContainer;await checkpoint()
      await docker(['exec','-u','root',shopContainer,'mkdir','-p','/tmp/benchtw/round2'])
      await command('bash',[`${scripts}/timewarp-shop-setup.sh`,shopContainer,options.upstream,lane,evidence],1800000)
      await docker(['cp',`${scripts}/timewarp-shop-supervisor.py`,`${shopContainer}:/tmp/benchtw/supervisor.py`])
      await docker(['cp',`${scripts}/round2/owned_processes.py`,`${shopContainer}:/tmp/benchtw/round2/owned_processes.py`])
      shopPrepared=true
    }
    const shopPort=await port()
    const child=await launch('docker',['exec','-i','-e','JAVA_HOME=/tmp/benchtw/java','-e','LD_LIBRARY_PATH=/tmp/benchtw/java/lib/server',
      '-e','JAVA_TOOL_OPTIONS=-Xmx512m','-e','OMP_NUM_THREADS=1',shopContainer,'/tmp/benchtw/shop-venv/bin/python','-u',
      '/tmp/benchtw/supervisor.py','/tmp/benchtw/shop-venv/bin/python','-u','/tmp/benchtw/shop.py',
      '/tmp/benchtw/webshop',String(era),String(shopPort)],`webshop-era-${era}`,{interactive:true})
    child.shop=true;siteChildren.push(child)
    const inspection=JSON.parse(await docker(['inspect',shopContainer]))[0]
    const ip=Object.values(inspection.NetworkSettings.Networks)[0].IPAddress
    assert.match(ip,/^[0-9.]+$/);siteUrls.webshop=`http://${ip}:${shopPort}/abc`
    await ready(child,siteUrls.webshop)
  }
  currentEra=era;currentShop=needsShop
}
try {
  const plan=JSON.parse(await readFile(options.frozenPlan,'utf8'))
  assert.equal(plan.seed,42);assert.equal(plan.episodes.length,1386)
  report.frozenPlanSha256=await hash(options.frozenPlan)
  report.frozenArchiveSha256=await hash(options.frozenArchive)
  assert.equal(report.frozenArchiveSha256,'0b41ee93f82c6b8f061733433777f037c03f4a49160bfb5dd86389e9885babd3')
  report.harnessCommit=await command('git',['-C',options.runtime.repo,'rev-parse','HEAD'])
  assert.equal(await command('git',['-C',options.runtime.repo,'status','--porcelain']),'','MP-10 clean runner required')
  report.controllerCommit='fda30571d';report.upstreamCommit='dc0e0885e5018b7b120e24d2f80e7f57b64046ac'
  for(const pin of options.pins)assert.equal(await hash(pin.path),pin.sha256,`MP-10 frozen bytes differ: ${path.basename(pin.path)}`)
  report.pins=options.pins;await checkpoint()
  const continuations=await Promise.all((options.priorEvidence?[options.priorEvidence]:options.priorEvidencePaths??[]).map(verifyTimeWarpContinuation))
  const skipped=new Set(continuations.flatMap(c=>[...c.skip]))
  if(continuations.length){report.continuations=continuations.map(c=>c.receipt);await checkpoint()}
  stage='runtime_startup';runtime=await startOwnedRuntime(options.runtime)
  report.runtime=runtime.source;await checkpoint()
  api=await openKernelClient({clientRoot:options.runtime.clientRoot,kernelUrl:runtime.kernelUrl})
  observeKernelRpcErrors(api.client,failure=>appendFile(`${evidence}/rpc-errors.jsonl`,JSON.stringify(failure)+'\n'))
  const helpers=await import(pathToFileURL(`${options.runtime.clientRoot}/dist/session-history-fragments.js`))
  api.helpers=helpers
  room=new Round2Room(api,{workspace:runtime.workspace,runId,checkpoint:owned=>writeEvidence(evidence,'ownership.json',owned)})
  await room.setup({guard,viewport:{css_width:1280,css_height:720,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:720},
    verifySlice:async({sliceId,name})=>{
      const inspection=JSON.parse(await docker(['inspect',`chariox-slice-${name}`]))[0]
      assert.equal(inspection.Image,image);assert.equal(inspection.Config.Labels['io.chariox.slice.id'],sliceId)
      assert.equal(inspection.HostConfig.Memory,2048*1024**2);assert.equal(inspection.HostConfig.NanoCpus,1e9)
      const slice=unwrap(await api.client.send(api.requests.getSliceRequest(sliceId)),'Slice').slice
      report.slice={id:sliceId,name,workerKernelId:slice.worker_kernel_id,volumes:inspection.Mounts.filter(m=>m.Type==='volume').map(m=>m.Name)}
      const expected=runtime.source.artifacts.find(a=>a.name==='chariox-kernel').sha256.replace(/^sha256:/,'')
      assert.equal((await docker(['exec',`chariox-slice-${name}`,'sha256sum','/opt/chariox-slice/bin/chariox-kernel'])).split(/\s+/)[0],expected)
    }})
  const container=`chariox-slice-${report.slice.name}`
  stage='official_grader_setup'
  await docker(['exec','-u','root',container,'mkdir','-p','/tmp/benchtw/screenshots'])
  await docker(['cp',`${options.upstream}/src`,`${container}:/tmp/benchtw/src`])
  for(const name of ['pyproject.toml','requirements.txt','README.md'])await docker(['cp',`${options.upstream}/${name}`,`${container}:/tmp/benchtw/${name}`])
  await docker(['cp',`${scripts}/timewarp-grader.py`,`${container}:/tmp/benchtw/grader.py`])
  await docker(['exec','-u','root',container,'chown','slice:slice','/tmp/benchtw/screenshots'])
  await docker(['exec','-u','root',container,'python3','-m','venv','/tmp/benchtw/venv'])
  await command('docker',['exec','-u','root',container,'/tmp/benchtw/venv/bin/pip','install','--quiet','browsergym-core==0.14.3','nltk','requests'],240000)
  await command('docker',['exec','-u','root',container,'/tmp/benchtw/venv/bin/pip','install','--quiet','--no-deps','/tmp/benchtw'],240000)
  grader=spawn('docker',['exec','-i','-u','slice',container,'/tmp/benchtw/venv/bin/python','-u','/tmp/benchtw/grader.py'],{detached:true,stdio:['pipe','pipe','pipe']})
  await new Promise((resolve,reject)=>{grader.once('spawn',resolve);grader.once('error',reject)})
  assert(Number.isSafeInteger(grader.pid)&&grader.pid>1)
  children.push(grader);grader.stderr.pipe(createWriteStream(`${runtime.root}/grader.log`,{mode:0o600}))
  graderCall=graderRequests(grader)
  const manifest=await graderCall({op:'manifest'});assert.equal(manifest.count,231)
  const expected=[]
  for(const needsShop of [false,true])for(let era=1;era<=6;era++)for(const task of manifest.tasks)
    if(task.sites.includes('webshop')===needsShop)expected.push({taskId:task.taskId,era,sites:task.sites,evalTypes:task.evalTypes})
  assert.deepEqual(expected,plan.episodes,'MP-10 frozen schedule changed')
  await writeEvidence(evidence,'FULL_MANIFEST.json',{...report,episodes:expected})
  if(options.preflight) {
    stage='preflight_fixture_readiness';await startSites(1,false)
    const setup=await graderCall({op:'setup',taskId:expected[0].taskId,seed:42,urls:siteUrls})
    report.preflight={zeroSolverSubmissions:true,setup:{url:setup.url,viewport:setup.viewport,evalTypes:setup.evalTypes}}
    const attachment=unwrap(await api.client.send(api.requests.attachToSessionRequest(room.owned.sessionId,runId)),'SessionAttached').attachment
    room.owned.attachmentId=attachment.id;await room.checkpoint(room.owned)
    await api.client.subscribeToKernelEvents(room.owned.sessionId,attachment.id)
    // Real passive time beyond the 60-second sweep; no provider prompt.
    for(let i=0;i<13;i++){await pause(5000);await guard()}
    await api.client.send({PumpTerminalOutput:{session_id:room.owned.sessionId,attachment_id:attachment.id}})
    report.preflight.attachmentSurvived65Seconds=true
  }
  if(!options.preflight||options.workerProbe) {
    stage='worker_provider_admission'
    await room.spawn({provider:'codex',model:report.model,effort:'high',accountProfile:runtime.profileId,sliceRef:report.slice.id})
    const state=unwrap(await api.client.send(api.requests.getSessionStateRequest(room.owned.sessionId)),'SessionState')
    const agent=state.session.agents.find(a=>a.id===room.owned.agentId)
    assert(report.slice.workerKernelId&&agent.remote_execution,'MP-10 kernel worker binding missing')
    assert.equal(agent.remote_execution?.worker_kernel_id,report.slice.workerKernelId,'MP-10 frozen explicit worker placement required')
    assert.equal(agent.effort,'high');assert.equal(agent.model,report.model)
    report.provider={model:agent.model,workerKernelId:agent.remote_execution.worker_kernel_id,effort:agent.effort}
    await checkpoint()
  }
  const episodes=options.preflight?[]:expected.filter(e=>!skipped.has(`${e.taskId}:${e.era}`))
  for(const episode of episodes) {
    await guard();stage='fixture_readiness'
    const row={mpItems:report.mpItems,...episode,seed:42,runId,effort:'high',harnessValid:false,startedAt:new Date().toISOString()}
    activeRow=row;report.attempts.push(row);await checkpoint()
    if(episode.evalTypes.includes('llm_judge')) {
      row.excluded=true;row.reason='Frozen official GPT-5.1 judge unavailable; no substitute or solver admission';await checkpoint();continue
    }
    try {
      await startSites(episode.era,episode.sites.includes('webshop'))
      stage='official_task_setup'
      const setup=await graderCall({op:'setup',taskId:episode.taskId,seed:42,urls:siteUrls})
      row.goal=setup.goal;row.setup={url:setup.url,viewport:setup.viewport,evalTypes:setup.evalTypes}
      stage='attachment_refresh'
      const attachment=unwrap(await api.client.send(api.requests.attachToSessionRequest(room.owned.sessionId,runId)),'SessionAttached').attachment
      room.owned.attachmentId=attachment.id;await room.checkpoint(room.owned)
      await api.client.subscribeToKernelEvents(room.owned.sessionId,attachment.id)
      const prompt=`MP-08 / MP-10 benchmark smoke. Solve this task in the Room browser already open: ${setup.goal}\nUse ONLY Chariox first-party slice_browser_* tools. Observe the page and use returned opaque field IDs for actions. Do not use shell, scripts, file edits, Computer tools, provider-native browser tools, or another browser. Do not inspect benchmark code or reward globals. Stop when the task is complete or unsupported. Maximum 60 mutating Browser actions; wall timeout 180 seconds. Return only your final answer to the task. Do not restate the task or narrate actions.`
      row.promptSha256=createHash('sha256').update(prompt).digest('hex')
      stage='prompt_submit';row.admission='reserved';await checkpoint()
      const started=Date.now(),identity=await room.submit(prompt)
      Object.assign(row,identity,{admission:'submitted',providerStartedAt:new Date(started).toISOString()});await checkpoint()
      stage='provider_settlement'
      const settled=await waitForSettlement(api,identity,{maxMs:180000,cancel:()=>room.cancel(),
        observe:()=>api.client.send({PumpTerminalOutput:{session_id:identity.sessionId,attachment_id:room.owned.attachmentId}}),guard:async()=>{
        try{await guard();return null}catch{return {cause:'resource_floor_or_interruption'}}
      }})
      row.turnLifecycle=settled.turn?.lifecycle??null;row.timedOut=Boolean(settled.cancellation);row.settlementMs=settled.elapsedMs
      stage='history_audit'
      const entries=settled.turn?await loadTurnHistory(api,{...identity,turn:settled.turn}):[]
      row.settlement=settlementRecord({...settled,...identity,entries,helpers})
      row.providerUnauthorized=entries.some(item=>item.entry.kind==='provider_error'&&/401|unauthorized|refresh_token_reused/i.test(item.entry.text))
      if(row.timedOut||row.turnLifecycle!=='completed'){stage='provider_settlement';throw Error('MP-10 provider settlement RED')}
      const audit=auditTimeWarpHistory(settled.turn,entries,helpers)
      row.answer=audit.answer;row.providerError=audit.providerError
      row.tools=audit.tools.map(record=>({tool:record.tool,status:record.status,input:sanitizeDrillMetadata(record.input)}))
      row.toolAuditComplete=true
      const history=unwrap(await api.client.send(api.requests.listRoomEnvironmentActionHistoryRequest(identity.sessionId,null,1000)),'RoomEnvironmentActionHistoryListed').page.actions
      row.actions=history.filter(a=>a.actor_id===`agent:${identity.agentId}`&&a.submitted_at_ms>=started)
      row.mutatingActions=row.actions.filter(a=>!new Set(['browser_status','browser_find','browser_text','browser_wait_for_text','browser_events','browser_downloads']).has(a.kind)).length
      assert(row.tools.length>0&&!row.providerError&&row.tools.every(t=>permittedBrowserTool(t.tool)), 'MP-10 frozen Browser-only tool audit RED')
      assert(row.mutatingActions<=60&&row.actions.every(a=>a.mode==='browser'),'MP-10 frozen action budget/track RED')
      stage='official_scoring'
      const verdict=await graderCall({op:'validate',answer:row.answer})
      row.reward=verdict.reward;row.done=verdict.done;row.officialInfo=verdict.info
      stage='evidence_capture'
      await graderCall({op:'screenshot',path:`/tmp/benchtw/screenshots/${episode.taskId}-v${episode.era}.png`})
      await docker(['cp',`${container}:/tmp/benchtw/screenshots/${episode.taskId}-v${episode.era}.png`,`${evidence}/${episode.taskId}-v${episode.era}.png`])
      row.harnessValid=true
    } catch(error) {
      row.firstFailingSeam=stage;row.error={errorClass:error.name,code:error.code??null,message:sanitizeDrillMetadata(error.message)}
      // No retry of reserved or submitted prompts, even when an RPC outcome is unknown.
      if(row.admission) {
        await room.cancel().catch(()=>{})
        const deadline=Date.now()+120000
        let settled=false
        while(Date.now()<deadline) {
          const state=unwrap(await api.client.send(api.requests.getSessionStateRequest(room.owned.sessionId)),'SessionState')
          if(!state.session.agents.find(a=>a.id===room.owned.agentId)?.is_processing){settled=true;break}
          await pause(500)
        }
        assert(settled,'MP-11 provider ownership did not settle')
      }
      if(!row.admission||stage==='prompt_submit'||stage==='official_scoring'||stage==='evidence_capture')interrupted=true
    }
    row.finishedAt=new Date().toISOString();await checkpoint()
    console.log(JSON.stringify({mpItems:report.mpItems,taskId:episode.taskId,era:episode.era,settled:report.attempts.length,
      wins:report.attempts.filter(r=>r.harnessValid&&r.reward===1).length,valid:report.attempts.filter(r=>r.harnessValid).length,
      invalid:report.attempts.filter(r=>!r.harnessValid&&!r.excluded).length}))
    if(interrupted||row.providerUnauthorized)throw Error('MP-10 fixture/resource/provider failure; retain attempts')
  }
  report.completed=report.attempts.length===episodes.length
} catch(error) {
  report.firstFailingSeam=stage;report.error={errorClass:error.name,message:sanitizeDrillMetadata(error.message)};process.exitCode=1
  console.log(`MP-08/MP-10 TimeWarp RED at ${stage}: ${error.name}`)
} finally {
  const cleanup={processes:[],rooms:null,shopGone:!shopContainer}
  try{await stopSites()}catch(error){cleanup.fixtureError=error.name}
  if(grader){grader.stdin.end();try{assert(await stopOwnedProcess(grader,{detached:true}));}catch(error){cleanup.graderError=error.name}}
  if(room)try{cleanup.rooms=await room.cleanup()}catch(error){cleanup.roomError=error.name}
  if(shopContainer) {
    const inspection=await docker(['inspect',shopContainer]).then(text=>JSON.parse(text)[0],()=>null)
    if(inspection){assert.equal(inspection.Config.Labels['io.chariox.r2next.timewarp'],runId);await docker(['rm','-f','-v',shopContainer])}
    cleanup.shopGone=await docker(['inspect',shopContainer]).then(()=>false,()=>true)
  }
  for(const child of children.toReversed()) {
    try{cleanup.processes.push({pid:child.pid,stopped:await stopOwnedProcess(child,{detached:true})})}
    catch(error){cleanup.processes.push({pid:child.pid,stopped:false,errorClass:error.name})}
  }
  if(api)await api.client.close()
  if(runtime)cleanup.runtime=await runtime.close({preserveState:cleanup.rooms?.complete!==true||cleanup.processes.some(p=>!p.stopped)})
  cleanup.complete=cleanup.rooms?.complete===true&&cleanup.shopGone&&cleanup.processes.every(p=>p.stopped)&&cleanup.runtime?.stateRemoved===true
  if(report.slice){
    cleanup.containerGone=await docker(['inspect',`chariox-slice-${report.slice.name}`]).then(()=>false,()=>true)
    cleanup.volumesGone=await Promise.all(report.slice.volumes.map(v=>docker(['volume','inspect',v]).then(()=>false,()=>true))).then(v=>v.every(Boolean))
    cleanup.complete&&=cleanup.containerGone&&cleanup.volumesGone
  }
  report.cleanup=cleanup;report.finishedAt=new Date().toISOString()
  if(!cleanup.complete)process.exitCode=1
  await checkpoint();process.off('SIGTERM',signal);process.off('SIGINT',signal)
}

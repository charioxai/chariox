// MP-08 / MP-09 / MP-10 / MP-11: real hosted new UI + kernel + official Codex,
// with an owned no-email verification page. Supplementary rehearsal, not AU-03 acceptance.
import assert from 'node:assert/strict'
import path from 'node:path'
import {pathToFileURL} from 'node:url'
import {mkdir,readFile,readlink,writeFile} from 'node:fs/promises'
import {createHash} from 'node:crypto'
import {startChallengeService} from './challenge-service.mjs'

const root=process.env.VFLAG_CLOUD_ROOT ?? '/root/work/agent-vflag-cloud'
const {chromium}=await import(pathToFileURL(path.join(root,'node_modules/playwright-core/index.mjs')))
const {startKernelFixture,waitFor}=await import(pathToFileURL(path.join(root,'apps/web/scripts/lib/external-access-kernel-fixture.mjs')))
const {enrollHostedKernel}=await import(pathToFileURL(path.join(root,'apps/web/scripts/lib/hosted-kernel-enrollment.mjs')))
const {signInIfRequired}=await import('/w/valenv/scripts/sign-in.mjs')
const origin=process.env.VFLAG_ORIGIN, evidence=process.env.VFLAG_EVIDENCE, binaryDir=process.env.VFLAG_BIN
assert.match(origin ?? '',/^https:\/\/pr[0-9]+-vflag[a-z0-9]*\.val\.51-255-87-178\.sslip\.io$/)
assert.ok(path.isAbsolute(evidence ?? '')&&path.isAbsolute(binaryDir ?? ''))
const expected=process.env.VFLAG_EXPECT ?? 'green'
const cells=[{id:'desktop-dpr1',width:1440,height:900,dpr:1},{id:'desktop-dpr2',width:1440,height:900,dpr:2},{id:'phone-dpr2',width:390,height:844,dpr:2}].filter(c=>(process.env.VFLAG_CELLS ?? 'desktop-dpr1,desktop-dpr2,phone-dpr2').split(',').includes(c.id))
await mkdir(evidence,{recursive:true})
const results=[],errors=[]
let fixture,browser,page,context,sessionId,agentId
const service=await startChallengeService()
const state=async()=>(await fixture.send({GetSessionState:{session_id:sessionId}})).SessionState.session
const save=async()=>writeFile(path.join(evidence,'results.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',expected,results,errors},null,2)+'\n')
async function closeFixture() {
  if(!fixture)return
  const closing=fixture.close()
  const watchdog=setTimeout(async()=>{
    for(const pid of fixture.pids) {
      if(!Number.isSafeInteger(pid)||pid<=1)continue
      try {if(await readlink(`/proc/${pid}/cwd`)===fixture.scratch)process.kill(pid,'SIGKILL')}catch(error){if(!['ENOENT','ESRCH'].includes(error.code))errors.push('cleanup:'+error.code)}
    }
  },15_000)
  try{await closing}finally{clearTimeout(watchdog)}
}
try {
  fixture=await startKernelFixture(binaryDir,'/usr/local/bin/codex',{kernelEnvironment:{IS_SANDBOX:'1',CHARIOX_BROWSER_DISPLAY_OPENH264:process.env.CHARIOX_BROWSER_DISPLAY_OPENH264}})
  await writeFile(path.join(evidence,'runtime-identity.json'),JSON.stringify({scratch:fixture.scratch,kernelPort:fixture.kernelPort,pids:fixture.pids},null,2)+'\n')
  browser=await chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage']})
  await enrollHostedKernel({browser,fixture,origin,evidence,relayPrefix:'wss://pr0-vflag'})
  await fixture.send({UnlockCredentialVault:{passphrase:fixture.passkey,duration:{kind:'until_kernel_stops'},surface:'terminal'}})
  const profilePath=process.env.VFLAG_CODEX_PROFILE
  assert.ok(path.isAbsolute(profilePath ?? ''),'Use a Chariox product-linked profile')
  const profile=(await fixture.send({LinkProviderAccountProfile:{provider:'codex',label:'Verification rehearsal',path:profilePath}})).ProviderAccountProfile.profile
  const worktrees=(await fixture.send({ListWorkspaceWorktrees:{workspace_id:fixture.workspace}})).WorkspaceWorktreesListed.worktrees
  for(const cell of cells) {
    console.log('Stage: '+cell.id)
    const challenge=service.create(), tag=cell.id+'-'+Date.now().toString(36)
    const created=(await fixture.send({CreateSession:{workspace_id:fixture.workspace,worktree_id:worktrees[0].path,alias:'Verification '+cell.id,agent_defaults:{provider:'codex',model:'gpt-6-luna',effort:'low',account_profile:profile.profile_id}}})).SessionCreated
    sessionId=created.session.id;agentId=created.agent.id
    const row={cell:cell.id,challenge_id:challenge.id,session_id:sessionId,agent_id:agentId,status:'RUNNING'};results.push(row);await save()
    context=await browser.newContext({viewport:{width:cell.width,height:cell.height},deviceScaleFactor:cell.dpr,hasTouch:cell.width===390})
    page=await context.newPage();page.setDefaultTimeout(60_000)
    page.on('pageerror',error=>errors.push('pageerror:'+error.message.slice(0,250)))
    await signInIfRequired(page,origin,'/terminal?ui=os')
    const bundle=page.waitForResponse(r=>/\/assets\/terminal-os\.[a-f0-9]+\.js$/.test(r.url()))
    await page.goto(origin+'/terminal?ui=os')
    row.client_sha256=createHash('sha256').update(await(await bundle).body()).digest('hex')
    if(cell.width===390)await page.locator('.mobile-sidebar-toggle').click()
    await page.locator(`[data-room-id='${sessionId}'] [data-action=open-room]`).click()
    await page.locator('[data-prompt-input]').waitFor()
    await fixture.send({KernelBrowser:{command:{op:'grant_room_computer',agent_id:agentId}}})
    const prompt=`MP-08 / MP-09 / MP-10 / MP-11. Owned local verification rehearsal ONLY. Use real Chariox tools. Load kernel browser if needed and continue on automatic reload. Open ${challenge.url}; take a fresh snapshot and obtain the ref for main#challenge, NOT an input or heading. Then request_popup wait:false, title "Verification ${tag}", message "Enter the test code in this page", choices ONLY [{"id":"cancel","label":"Cancel","reply":"cancel"}], view browser_region with that exact tab_id and main ref${expected==='red'?'':`, verification ${JSON.stringify({challenge_id:challenge.id,url:challenge.url+'/verify',accepted_json_pointer:'/accepted'})}`}. Do NOT click Request test code, enter any code, navigate, close, cancel, or steer while the person acts. The card MUST exist before a code is requested. Yield on the returned card registration, deadline 15 minutes from now. On wakes, read the inbox. Do not treat rejection, dialog disappearance, source loss or card answer as acceptance. Only after the card condition_met with a matching accepted verification_result receipt, take a fresh snapshot, capture the readonly Scoped API token field through chariox.vault.capture_browser read node, request_id "capture_${tag}", label "verification-${tag}", use_binding {target:{kind:"command_environment",name:"REHEARSAL_TOKEN",origin:"${new URL(challenge.url).origin}"},value_kind:"secret"}. Use the returned credential_id with chariox.vault.use_env, origin "${new URL(challenge.url).origin}", env {"REHEARSAL_TOKEN":<credential_id>}, argv ${JSON.stringify(['node','-e',"fetch(process.argv[1],{headers:{authorization:'Bearer '+process.env.REHEARSAL_TOKEN}}).then(async r=>{console.log(JSON.stringify({status:r.status,authenticated:(await r.json()).authenticated}));process.exitCode=r.status===200?0:1})",challenge.url+'/me'])}. Require exit_code0, HTTP200 and authenticated true. Never print code/token or use shell to read it. Remove your own capture with chariox.vault.remove_capture. Finish REHEARSAL_${tag}_PASS only on successful capture+authenticated call+removal; otherwise REHEARSAL_${tag}_FAIL plus a nonsecret reason.`
    await page.locator('[data-prompt-input]').fill(prompt);await page.locator('[data-action=send-prompt]').click()
    const card=page.locator('[data-interaction-id]').filter({hasText:'Enter the test code in this page'})
    await card.waitFor({timeout:300_000})
    const live=(await state()).active_interactions.find(c=>c.title==='Verification '+tag)
    assert.ok(live?.asking_task_id&&live.view?.browser_region)
    const activeRun=(await state()).active_provider_run_id;
    if(activeRun){const run=(await fixture.send({GetProviderRun:{provider_run_id:activeRun}})).ProviderRun.provider_run;row.model=run.model;row.effort=run.variant;assert.equal(run.model,'gpt-6-luna')}
    Object.assign(row,{task_id:live.asking_task_id,card_id:live.id,region:live.view.browser_region})
    assert.equal((await(await fetch(challenge.url+'/status')).json()).issued,false,'card precedes first code request')
    assert.equal(await card.locator('[data-choice-id]').count(),1,'no separate Confirm/Done')
    await card.locator('[data-action=read-interaction]').click()
    const popup=page.locator(`[data-interaction-detail='${live.id}']`)
    await popup.locator('.kernel-browser-page-surface[data-paint-mode=mirror]').waitFor({timeout:120_000})
    row.popup_geometry=await popup.evaluate(element=>{const r=element.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height}})
    assert.ok(row.popup_geometry.x>=0&&row.popup_geometry.x+row.popup_geometry.width<=cell.width,'the live popup fits the viewer width')
    const frame=page.frameLocator(`[data-interaction-detail='${live.id}'] .kernel-browser-mirror iframe:not([inert])`)
    await frame.locator('#issue').click()
    await frame.locator('#code').waitFor({state:'visible'})
    await frame.locator('#code').click();await frame.locator('#code').pressSequentially('000000',{delay:50});await frame.locator('#code').press('Enter')
    await waitFor(async()=>(await(await fetch(challenge.url+'/status')).json()).events.some(e=>e.result==='rejected'),'server rejection')
    await page.waitForTimeout(2000)
    row.ui_after_rejection=await frame.locator('body').evaluate(body=>({has_rejection:body.textContent.includes('That code was not accepted'),has_verify:body.textContent.includes('Verify'),alerts:body.querySelectorAll('[role=alert]').length,nodes:['challenge','verify','code'].map(id=>{const node=body.querySelector('#'+id);const box=node?.getBoundingClientRect();return{id,present:Boolean(node),hidden:node?.hidden,children:node?.childNodes.length,box:box?{x:box.x,y:box.y,width:box.width,height:box.height}:null}})}));
    await save()
    const afterRejected=await state()
    row.rejection_card_retained=(afterRejected.active_interactions??[]).some(c=>c.id===live.id)
    await page.screenshot({path:path.join(evidence,cell.id+'-rejected.png')})
    if(expected==='red') {
      assert.equal(row.rejection_card_retained,false,'base loses the card on SPA anchor replacement')
      row.status='EXPECTED RED';await fixture.send({EndSession:{session_id:sessionId}});await save();break
    }
    assert.equal(row.rejection_card_retained,true,'a site rejection keeps the same card')
    await popup.waitFor()
    await frame.getByRole('alert').filter({hasText:'That code was not accepted'}).waitFor()
    await frame.locator('#code').click();await frame.locator('#code').press('ControlOrMeta+A');await frame.locator('#code').pressSequentially('246810',{delay:50});await frame.locator('#code').press('Enter')
    await waitFor(async()=>(await(await fetch(challenge.url+'/status')).json()).accepted,'actual server acceptance')
    await popup.waitFor({state:'detached',timeout:30_000})
    await waitFor(async()=>{
      const status=await(await fetch(challenge.url+'/status')).json()
      const text=(await page.locator(`[data-room-pane='${agentId}'] .ap-final`).allTextContents()).join('\n')
      assert.ok(!text.includes('REHEARSAL_'+tag+'_FAIL'),'official provider reported failure')
      return status.authenticated_calls>0&&text.includes('REHEARSAL_'+tag+'_PASS')
    },'real provider Vault capture and authenticated HTTP call',300_000)
    const final=await(await fetch(challenge.url+'/status')).json()
    row.events=final.events;row.authenticated_calls=final.authenticated_calls;row.status='PASS'
    assert.equal(final.events.filter(e=>e.kind==='challenge_issued').length,1)
    const agent=(await state()).agents.find(a=>a.id===agentId)
    const runId=agent?.provider_run_id ?? agent?.providerRunId
    if(runId) {const run=(await fixture.send({GetProviderRun:{provider_run_id:runId}})).ProviderRun.provider_run;assert.equal(run.model,'gpt-6-luna');row.model=run.model;row.effort=run.variant}
    await page.screenshot({path:path.join(evidence,cell.id+'-accepted.png')})
    await fixture.send({EndSession:{session_id:sessionId}})
    await context.close();context=null;await save()
  }
} catch(error) {
  errors.push(error.message);process.exitCode=1
  if(results.length)results.at(-1).status='FAIL'
  if(fixture&&sessionId) {try {const snapshot=await state();await writeFile(path.join(evidence,'failure-status.json'),JSON.stringify({agents:snapshot.agents?.map(a=>({id:a.id,is_processing:a.is_processing,state:a.state})),cards:(snapshot.active_interactions??[]).map(c=>({id:c.id,title:c.title,task:c.asking_task_id,region:c.view?.browser_region})),tasks:snapshot.agent_tasks?.map(t=>({id:t.task_id,state:t.state})),wakes:snapshot.agent_wakes?.map(w=>({id:w.id,state:w.state,kind:w.kind}))},null,2)+'\n')}catch{}}
  if(page)await page.screenshot({path:path.join(evidence,'failure.png')}).catch(()=>{})
} finally {
  await save()
  await browser?.close().catch(()=>{})
  await closeFixture()
  await service.close()
}
if(process.exitCode)throw Error('Hosted verification rehearsal failed; see status-only results')

// MP-08 / MP-09 / MP-10 / MP-11: real hosted new UI + kernel + official Codex,
// with an owned no-email verification page. Supplementary rehearsal, not AU-03 acceptance.
import assert from 'node:assert/strict'
import path from 'node:path'
import {pathToFileURL} from 'node:url'
import {mkdir,readFile,readlink,writeFile} from 'node:fs/promises'
import {createHash} from 'node:crypto'
import {execFileSync} from 'node:child_process'
import {startChallengeService} from './challenge-service.mjs'

const root=process.env.VFLAG_CLOUD_ROOT ?? '/root/work/agent-vflag-cloud'
const {chromium}=await import(pathToFileURL(path.join(root,'node_modules/playwright-core/index.mjs')))
const {startKernelFixture,waitFor}=await import(pathToFileURL(path.join(root,'apps/web/scripts/lib/external-access-kernel-fixture.mjs')))
const {enrollHostedKernel}=await import(pathToFileURL(path.join(root,'apps/web/scripts/lib/hosted-kernel-enrollment.mjs')))
const {signInIfRequired}=await import('/w/valenv/scripts/sign-in.mjs')
const origin=process.env.VFLAG_ORIGIN, evidence=process.env.VFLAG_EVIDENCE, binaryDir=process.env.VFLAG_BIN
const existingRoot=process.env.VFLAG_EXISTING_KERNEL_ROOT
if(existingRoot)assert.equal(existingRoot,'/root/.chariox/dev/vault-drill-kernel','only the explicitly authorized real test stack')
assert.match(origin ?? '',/^https:\/\/pr[0-9]+-(?:vflag[a-z0-9]*|gmailprep)\.val\.51-255-87-178\.sslip\.io$/)
assert.ok(path.isAbsolute(evidence ?? '')&&path.isAbsolute(binaryDir ?? ''))
const expected=process.env.VFLAG_EXPECT ?? 'green'
assert.ok(['green','encoding-red','grant-red'].includes(expected))
const recoverBrowser=process.env.VFLAG_RECOVER_BROWSER==='1'||expected==='grant-red'
const requestEncoding=process.env.VFLAG_ENCODING ?? 'json'
const segmented=process.env.VFLAG_SEGMENTED==='1'
assert.ok(['json','urlencoded'].includes(requestEncoding))
const cells=[{id:'desktop-dpr1',width:1440,height:900,dpr:1},{id:'desktop-dpr2',width:1440,height:900,dpr:2}].filter(c=>(process.env.VFLAG_CELLS ?? 'desktop-dpr1,desktop-dpr2').split(',').includes(c.id))
await mkdir(evidence,{recursive:true})
const results=[],errors=[]
let fixture,browser,page,context,viewerContext,sessionId,agentId,challengeUrl
const service=await startChallengeService()
const state=async()=>(await fixture.send({ListSessions:{}})).SessionsListed.sessions.find(session=>session.id===sessionId)
const save=async()=>writeFile(path.join(evidence,'results.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',expected,requestEncoding,segmented,recoverBrowser,results,errors},null,2)+'\n')
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
  if(existingRoot) {
    process.env.ROOM_KERNEL_ROOT=existingRoot;process.env.ROOM_WS_FROM=path.join(root,'apps/api/package.json')
    const peer=await(await import('./room-driver/rpc.mjs')).connect()
    const workspace=path.join(process.env.TMPDIR,'real-stack-workspace-'+Date.now())
    await mkdir(workspace,{recursive:true});execFileSync('git',['init','-q','-b','main',workspace]);execFileSync('git',['-C',workspace,'-c','user.name=Fixture','-c','user.email=fixture@chariox.invalid','commit','--allow-empty','-qm','fixture [skip ci]'])
    assert.ok(workspace.startsWith(path.resolve(process.env.TMPDIR)+'/real-stack-workspace-'))
    execFileSync('chown',['-R','chariox-gmailprep:chariox-gmailprep',workspace])
    const registered=(await peer.send({CreateWorkspaceDirectory:{path:workspace}})).WorkspaceDirectoryCreated.directory
    fixture={send:peer.send,close:async()=>peer.close(),pids:[],scratch:existingRoot,workspace:registered}
    const vault=(await fixture.send({GetCredentialVaultStatus:{}})).CredentialVaultStatus.status
    assert.equal(vault.unlocked,true,'unlock the real test kernel Vault through the owner UI before running token verification')
  } else {
  fixture=await startKernelFixture(binaryDir,'/usr/local/bin/codex',{kernelEnvironment:{IS_SANDBOX:'1',CHARIOX_KERNEL_BROWSER_EXECUTABLE:process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE ?? '/opt/google/chrome/chrome',CHARIOX_BROWSER_DISPLAY_OPENH264:process.env.CHARIOX_BROWSER_DISPLAY_OPENH264,...(process.env.VFLAG_BROWSER_SCRIPT ? {CHARIOX_KERNEL_BROWSER_SCRIPT:process.env.VFLAG_BROWSER_SCRIPT} : {})}})
  }
  await writeFile(path.join(evidence,'runtime-identity.json'),JSON.stringify({scratch:fixture.scratch,kernelPort:fixture.kernelPort,pids:fixture.pids},null,2)+'\n')
  browser=await chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage']})
  if(!existingRoot)await enrollHostedKernel({browser,fixture,origin,evidence,relayPrefix:'wss://pr0-vflag'})
  if(!existingRoot)await fixture.send({UnlockCredentialVault:{passphrase:fixture.passkey,duration:{kind:'until_kernel_stops'},surface:'terminal'}})
  let profile
  if(existingRoot) {
    const profiles=(await fixture.send({ListProviderAccountProfiles:{provider:'codex'}})).ProviderAccountProfilesListed.profiles
    profile=profiles.find(item=>item.profile_id===(process.env.VFLAG_PROFILE_ID ?? 'vflag-r4-drill-owdpqv4h'))
    assert.ok(profile,'Use the existing product-linked rehearsal profile on the real kernel')
  } else {
    const profilePath=process.env.VFLAG_CODEX_PROFILE
    assert.ok(path.isAbsolute(profilePath ?? ''),'Use a Chariox product-linked profile')
    profile=(await fixture.send({LinkProviderAccountProfile:{provider:'codex',label:'Verification rehearsal',path:profilePath}})).ProviderAccountProfile.profile
  }
  const worktrees=(await fixture.send({ListWorkspaceWorktrees:{workspace_id:fixture.workspace}})).WorkspaceWorktreesListed.worktrees
  for(const cell of cells) {
    console.log('Stage: '+cell.id)
    const challenge=service.create({requestEncoding,segmented});challengeUrl=challenge.url;const tag=cell.id+'-'+Date.now().toString(36)
    const created=(await fixture.send({CreateSession:{workspace_id:fixture.workspace,worktree_id:worktrees[0].path,alias:'Verification '+cell.id,agent_defaults:{provider:'codex',model:'gpt-6-luna',effort:'low',account_profile:profile.profile_id}}})).SessionCreated
    sessionId=created.session.id;agentId=created.agent.id
    const row={cell:cell.id,challenge_id:challenge.id,session_id:sessionId,agent_id:agentId,status:'RUNNING'};results.push(row);await save()
    context=await browser.newContext({viewport:{width:cell.width,height:cell.height},deviceScaleFactor:cell.dpr,hasTouch:cell.width===390})
    page=await context.newPage();page.setDefaultTimeout(60_000)
    page.on('pageerror',error=>errors.push('pageerror:'+error.message.slice(0,250)))
    if(existingRoot) {
      await page.goto(origin+'/auth/login?returnTo=%2Fterminal%3Fui%3Dos')
      await page.getByLabel('Email',{exact:true}).fill((await readFile('/var/tmp/vflag-owner-email','utf8')).trim())
      try{await page.getByLabel('Access key',{exact:true}).fill((await readFile('/var/tmp/vflag-owner-key','utf8')).trim())}catch{throw Error('Owner access-key fill failed')}
      await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.waitForURL(url=>url.pathname!='/auth/login')
    } else await signInIfRequired(page,origin,'/terminal?ui=os')
    const bundle=page.waitForResponse(r=>/\/assets\/terminal-os\.[a-f0-9]+\.js$/.test(r.url())).then(async response=>{
      const asset=await context.request.get(response.url())
      assert.equal(asset.ok(),true,'the loaded client bundle remains available')
      return createHash('sha256').update(await asset.body()).digest('hex')
    })
    await page.goto(origin+'/terminal?ui=os')
    row.client_sha256=await bundle
    if(cell.width===390)await page.locator('.mobile-sidebar-toggle').click()
    await page.locator(`[data-room-id='${sessionId}'] [data-action=open-room]`).click()
    await page.locator('[data-prompt-input]').waitFor()
    await fixture.send({KernelBrowser:{command:{op:'grant_room_computer',agent_id:agentId}}})
    await fixture.send({FocusAgent:{session_id:sessionId,agent_id:agentId}})
    const prompt=`MP-08 / MP-09 / MP-10 / MP-11. Owned local verification rehearsal ONLY. Use real Chariox tools. Write visible updates and the card message directly to the person in plain words. Do not narrate tool names, refs, grants, provider reloads, internal receipts or task bookkeeping. Load kernel browser if needed and continue on automatic reload. Open ${challenge.url}; take a fresh snapshot and obtain the ref for main#challenge, NOT an input or heading. Then request_popup wait:false, title "Verification ${tag}", message "Press Send Code, enter the test code, then select Verify.", choices ONLY [{"id":"cancel","label":"Cancel","reply":"cancel"}], view browser_region with that exact tab_id and main ref, verification ${JSON.stringify({challenge_id:challenge.id,url:challenge.url+'/verify',accepted_json_pointer:'/accepted'})}. Do NOT click Send Code, enter any code, navigate, close, cancel, or steer while the person acts. The card MUST exist before a code is requested. Yield on the returned card registration, deadline 90 minutes from now. On wakes, read the inbox. Acknowledge a retired or unavailable card occurrence as handled before asking for browser approval. After successful token use and capture removal, acknowledge the accepted card completion occurrence as handled using the original asking task id and the inbox sequence before your final reply. After browser approval, read browser state first and find the original challenge tab in its tabs before taking a fresh snapshot. If browser access was revoked or the card became unavailable, keep the same tab, request browser access, wait for the owner decision, then take a fresh snapshot and raise a fresh verification card. This recovery is part of the current request; never treat an unavailable card as a completed task. Do not treat rejection, dialog disappearance, source loss or card answer as acceptance. Only after the card condition_met with a matching accepted verification_result receipt, take a fresh snapshot, capture the readonly Scoped API token field through chariox.vault.capture_browser read node, request_id "capture_${tag}", label "verification-${tag}", use_binding {target:{kind:"command_environment",name:"REHEARSAL_TOKEN",origin:"${new URL(challenge.url).origin}"},value_kind:"secret"}. Use the returned credential_id with chariox.vault.use_env, origin "${new URL(challenge.url).origin}", env {"REHEARSAL_TOKEN":<credential_id>}, argv ${JSON.stringify(['node','-e',"fetch(process.argv[1],{headers:{authorization:'Bearer '+process.env.REHEARSAL_TOKEN}}).then(async r=>{console.log(JSON.stringify({status:r.status,authenticated:(await r.json()).authenticated}));process.exitCode=r.status===200?0:1})",challenge.url+'/me'])}. Require exit_code0, HTTP200 and authenticated true. Never print code/token or use shell to read it. Remove your own capture with chariox.vault.remove_capture. Finish with the plain message Verification complete. The token works. only on successful capture, authenticated call and removal. Otherwise explain what still needs to be done in plain words.`
    await writeFile(path.join(fixture.workspace,'VERIFICATION_REHEARSAL.md'),prompt+'\n')
    await page.locator('[data-prompt-input]').fill(`Open ${challenge.url} and help me verify my identity. Follow VERIFICATION_REHEARSAL.md in this workspace: show the live card, wait while I finish, then check the temporary token works.`);await page.locator('[data-action=send-prompt]').click()
    const card=page.locator('[data-interaction-id]').filter({hasText:'Press Send Code, enter the test code, then select Verify.'})
    await waitFor(async()=>{
      if(await card.isVisible())return true
      const reply=(await page.locator(`[data-room-pane='${agentId}'] .ap-final`).allTextContents()).join('\n')
      assert.ok(!reply.includes('REHEARSAL_'+tag+'_FAIL'),'provider preparation failed before raising the card')
      return false
    },'the provider raises the live verification card',300_000)
    let live=((await state()).active_interactions??[]).find(c=>c.title==='Verification '+tag)
    assert.ok(live?.asking_task_id&&live.view?.browser_region)
    if(recoverBrowser) {
      const previous=live
      row.retired_card_id=previous.id
      await fixture.send({KernelBrowser:{command:{op:'revoke_grants',agent_id:agentId}}})
      await waitFor(async()=>!((await state()).active_interactions??[]).some(item=>item.id===previous.id),'the revoked card retires without pretending to verify')
      await waitFor(async()=>{
        const snapshot=await state()
        return (snapshot.active_interactions??[]).some(item=>item.choices?.some(choice=>choice.id==='allow_browser'))
          || snapshot.agent_tasks?.some(task=>task.agent_id===agentId&&['blocked','done'].includes(task.state))
      },'the agent requests browser approval after its card retires',180_000)
      if(expected==='grant-red') {
        const blocked=(await state()).active_interactions??[]
        assert.equal(blocked.some(item=>item.choices?.some(choice=>choice.id==='allow_browser')),false,'old path has no in-place browser approval')
        await page.screenshot({path:path.join(evidence,cell.id+'-grant-recovery-stuck.png')})
        row.status='EXPECTED RED';row.retired_card_recovery_stuck=true;await save();await fixture.send({EndSession:{session_id:sessionId}});break
      }
      const allow=page.getByRole('button',{name:'Allow browser',exact:true})
      await allow.waitFor({timeout:30_000})
      await page.screenshot({path:path.join(evidence,cell.id+'-allow-browser.png')})
      await allow.click()
      await waitFor(async()=>{
        live=((await state()).active_interactions??[]).find(item=>item.id!==previous.id&&item.title==='Verification '+tag&&item.view?.browser_region)
        return !!live
      },'approval continues the task and raises a fresh live card',300_000)
      assert.equal(live.view.browser_region.tab_id,previous.view.browser_region.tab_id,'recovery uses the same tab')
      row.browser_access_restored_in_place=true;row.fresh_card_after_retirement=true
      assert.equal(await allow.count(),0,'the resolved browser block disappears')
    }

    assert.equal(live.timeout_sec ?? null,null,'an owner verification card has no automatic expiry')
    row.card_has_no_deadline=true
    row.visible_card_message=live.message
    assert.equal(live.message,'Press Send Code, enter the test code, then select Verify.')
    assert.ok(!/chariox\.|condition_met|attributed result|retained obligations|user_domain_not_granted/.test(await card.innerText()),'the card speaks to the person')
    const askingTask=(await state()).agent_tasks?.find(task=>task.task_id===live.asking_task_id)
    if(askingTask?.wait?.deadline_ms) {
      row.wait_remaining_ms=askingTask.wait.deadline_ms-Date.now()
      assert.ok(row.wait_remaining_ms>=60*60*1000,'the person has at least an hour to finish')
    }

    const activeRun=(await state()).active_provider_run_id;
    if(activeRun){const run=(await fixture.send({GetProviderRun:{provider_run_id:activeRun}})).ProviderRun.provider_run;row.model=run.model;row.effort=run.variant;assert.equal(run.model,'gpt-6-luna')}
    Object.assign(row,{task_id:live.asking_task_id,card_id:live.id,region:live.view.browser_region})
    assert.equal((await(await fetch(challenge.url+'/status')).json()).issued,false,'card precedes first code request')
    assert.equal(await card.locator('[data-choice-id]').count(),1,'no separate Confirm/Done')
    if(expected==='green')assert.equal(await card.evaluate(element=>!!element.closest('.agent-terminal')),true,'card is visible in the transcript beside its work')
    await card.locator('[data-action=read-interaction]').click()
    const popup=page.locator(`[data-interaction-detail='${live.id}']`)
    await popup.locator('.kernel-browser-page-surface[data-paint-mode=mirror]').waitFor({timeout:120_000})
    row.popup_geometry=await popup.evaluate(element=>{const r=element.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height}})
    assert.ok(row.popup_geometry.x>=0&&row.popup_geometry.x+row.popup_geometry.width<=cell.width,'the live popup fits the viewer width')
    const frame=page.frameLocator(`[data-interaction-detail='${live.id}'] .kernel-browser-mirror iframe:not([inert])`)
    let viewerPopup
    if(expected==='green') {
      await frame.locator('#issue').waitFor()
      await page.screenshot({path:path.join(evidence,cell.id+'-card-ready.png')})
      viewerContext=await browser.newContext({viewport:{width:cell.width,height:cell.height},deviceScaleFactor:cell.dpr===1?2:1,storageState:await context.storageState()})
      const viewer=await viewerContext.newPage();await viewer.goto(origin+'/terminal?ui=os')
      await viewer.locator(`[data-room-id='${sessionId}'] [data-action=open-room]`).click()
      await viewer.locator(`[data-interaction-id='${live.id}'] [data-action=read-interaction]`).click()
      viewerPopup=viewer.locator(`[data-interaction-detail='${live.id}']`)
      await viewer.frameLocator(`[data-interaction-detail='${live.id}'] .kernel-browser-mirror iframe:not([inert])`).locator('#issue').waitFor()
      await viewer.screenshot({path:path.join(evidence,cell.id+'-second-viewer-ready.png')})
      row.concurrent_owner_viewer=true
    }
    await frame.locator('#issue').click()
    const enterCode=async (target,code)=>{
      if(segmented) {
        for(const i of [5,0,3,1,4,2]) {const field=target.locator('#code'+i);await field.click();await field.press('ControlOrMeta+A');await field.pressSequentially(code[i],{delay:50})}
        await target.locator('#code2').press('Enter')
      } else {const field=target.locator('#code');await field.click();await field.press('ControlOrMeta+A');await field.pressSequentially(code,{delay:50});await field.press('Enter')}
    }
    await enterCode(frame,'000000')
    await waitFor(async()=>(await(await fetch(challenge.url+'/status')).json()).events.some(e=>e.result==='rejected'),'server rejection')
    await page.waitForTimeout(2000)
    const afterRejected=await state()
    row.rejection_card_retained=(afterRejected.active_interactions??[]).some(c=>c.id===live.id)
    await page.screenshot({path:path.join(evidence,cell.id+'-rejected.png')})
    assert.equal(row.rejection_card_retained,true,'a site rejection keeps the same card')
    await popup.waitFor()
    await frame.getByRole('alert').filter({hasText:'That code was not accepted'}).waitFor()
    let retryFrame=frame
    if(expected==='green'&&!segmented) {
      await popup.getByRole('button',{name:'Open full tab',exact:true}).click()
      await popup.waitFor({state:'detached'})
      const sameTab=page.locator(`[data-room-browser] .kernel-browser-page-surface[data-browser-document='${live.view.browser_region.document_id}']`)
      await sameTab.waitFor()
      retryFrame=page.frameLocator(`[data-room-browser] .kernel-browser-page-surface[data-browser-document='${live.view.browser_region.document_id}'] .kernel-browser-mirror iframe:not([inert])`)
      await retryFrame.getByRole('alert').filter({hasText:'That code was not accepted'}).waitFor()
      await page.screenshot({path:path.join(evidence,cell.id+'-same-full-tab-rejected.png')})
      row.full_tab_same_document=true
    }
    await enterCode(retryFrame,'246810')
    assert.equal(await page.locator('[data-prompt-input]').inputValue(),'','typing stays in the browser instead of reaching the Room composer')
    row.typing_stayed_in_browser=true
    await waitFor(async()=>(await(await fetch(challenge.url+'/status')).json()).accepted,'actual server acceptance')
    if(expected==='encoding-red') {
      await page.waitForTimeout(2000)
      row.acceptance_card_retained=((await state()).active_interactions??[]).some(c=>c.id===live.id)
      assert.equal(row.acceptance_card_retained,true,'base cannot correlate the successful submitted code')
      await page.screenshot({path:path.join(evidence,cell.id+'-accepted-stuck.png')})
      row.status='EXPECTED RED';await fixture.send({EndSession:{session_id:sessionId}});await save();break
    }
    await viewerPopup.waitFor({state:'detached',timeout:30_000})
    await waitFor(async()=>!((await state()).active_interactions??[]).some(c=>c.id===live.id),'acceptance closes the same card on every owner client')
    row.self_closed_on_acceptance=true;row.resume_clicked=false
    await waitFor(async()=>{
      const status=await(await fetch(challenge.url+'/status')).json()
      const snapshot=await state()
      const task=snapshot.agent_tasks?.find(task=>task.task_id===live.asking_task_id)
      if(status.authenticated_calls===0||task?.state!=='done')return false
      const entries=(await fixture.send({ListCredentials:{}})).CredentialsListed.credentials
      const captures=entries.filter(entry=>entry.metadata?.created_by_id===agentId&&entry.metadata?.session_id===sessionId)
      row.capture_removed=captures.length===0;row.agent_continued_without_resume=task.state==='done'
      return captures.length===0
    },'real provider Vault capture, authenticated HTTP call and cleanup',300_000)
    const final=await(await fetch(challenge.url+'/status')).json()
    row.events=final.events;row.authenticated_calls=final.authenticated_calls;row.status='PASS'
    assert.equal(final.events.filter(e=>e.kind==='challenge_issued').length,1)
    const agent=(await state()).agents.find(a=>a.id===agentId)
    const runId=agent?.provider_run_id ?? agent?.providerRunId
    if(runId) {const run=(await fixture.send({GetProviderRun:{provider_run_id:runId}})).ProviderRun.provider_run;assert.equal(run.model,'gpt-6-luna');row.model=run.model;row.effort=run.variant}
    const replies=await page.locator('.pane-transcript-reply').allTextContents()
    await writeFile(path.join(evidence,cell.id+'-visible-replies.json'),JSON.stringify(replies,null,2)+'\n')
    assert.ok(!replies.some(text=>/chariox\.|condition_met|user_domain_not_granted|asking_task_id|inbox sequence|provider reload|retained obligations|\bsnapshot\b|\btools\b.*\b(refresh|loading|load)\b/i.test(text)),'visible replies use plain words')
    row.plain_visible_replies=true
    await page.screenshot({path:path.join(evidence,cell.id+'-accepted.png')})
    await fixture.send({EndSession:{session_id:sessionId}})
    await viewerContext?.close();viewerContext=null
    await context.close();context=null;await save()
  }
} catch(error) {
  errors.push(error.message);process.exitCode=1
  if(challengeUrl)await writeFile(path.join(evidence,'failure-service-status.json'),JSON.stringify(await(await fetch(challengeUrl+'/status')).json(),null,2)+'\n').catch(()=>{})
  if(results.length)results.at(-1).status='FAIL'
  if(fixture&&sessionId) {try {const snapshot=await state();await writeFile(path.join(evidence,'failure-status.json'),JSON.stringify({agents:snapshot.agents?.map(a=>({id:a.id,is_processing:a.is_processing,state:a.state})),cards:(snapshot.active_interactions??[]).map(c=>({id:c.id,title:c.title,task:c.asking_task_id,region:c.view?.browser_region})),tasks:snapshot.agent_tasks?.map(t=>({id:t.task_id,state:t.state})),wakes:snapshot.agent_wakes?.map(w=>({id:w.id,state:w.state,kind:w.kind}))},null,2)+'\n')}catch{}}
  if(fixture&&sessionId&&agentId)try {
    const history=(await fixture.send({GetSessionHistoryOutline:{session_id:sessionId,agent_ids:[agentId],latest_prompt_count:1,include_prefetched_content:false}})).SessionHistoryOutline;
    await writeFile(path.join(evidence,'failure-history.json'),JSON.stringify(history.agents.map(agent=>({agent_id:agent.agent_id,turns:agent.turns.map(turn=>({state:turn.lifecycle,summary:turn.summary?.entry?.text,tools:turn.blobs.filter(blob=>blob.kind==='provider_tool').map(blob=>({id:blob.blob_id,title:blob.title}))}))})),null,2)+'\n')
  }catch{}
  if(page)await page.screenshot({path:path.join(evidence,'failure.png')}).catch(()=>{})
} finally {
  await save()
  await browser?.close().catch(()=>{})
  await closeFixture()
  await service.close()
}
if(process.exitCode)throw Error('Hosted verification rehearsal failed; see status-only results')

// MP-08 / MP-09 / MP-10 / MP-11: owner-window once unlock, real provider, hosted WSS.
import assert from 'node:assert/strict';
import {mkdir,writeFile,readdir,readFile} from 'node:fs/promises';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const cloudRoot=process.env.VFLAG_CLOUD_ROOT??'/root/work/agent-vflag-cloud';
const {chromium}=await import(pathToFileURL(path.join(cloudRoot,'node_modules/playwright-core/index.mjs')));
const {startKernelFixture,waitFor}=await import(pathToFileURL(path.join(cloudRoot,'apps/web/scripts/lib/external-access-kernel-fixture.mjs')));
const {enrollHostedKernel}=await import(pathToFileURL(path.join(cloudRoot,'apps/web/scripts/lib/hosted-kernel-enrollment.mjs')));
import {signInIfRequired} from '/w/valenv/scripts/sign-in.mjs';
const evidence=process.env.VFLAG_EVIDENCE,expected=process.env.VFLAG_EXPECT??'green';
const origin=process.env.VFLAG_ORIGIN;
assert.match(origin??'',/^https:\/\/pr[0-9]+-vflag[a-z0-9]*\.val\.51-255-87-178\.sslip\.io$/);
assert.ok(path.isAbsolute(evidence??'')&&path.isAbsolute(process.env.VFLAG_BIN??''));
assert.ok(['red','green'].includes(expected));
await mkdir(evidence,{recursive:true});
let fixture,browser,sessionId,phase='start';
const report={MP:'MP-08 / MP-09 / MP-10 / MP-11',expected,cells:[]};
const save=()=>writeFile(evidence+'/results.json',JSON.stringify(report,null,2)+'\n');
try {
 fixture=await startKernelFixture(process.env.VFLAG_BIN,'/usr/local/bin/codex',{kernelEnvironment:{IS_SANDBOX:'1',CHARIOX_KERNEL_BROWSER_EXECUTABLE:process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE??'/opt/google/chrome/chrome'}});
 report.pids=fixture.pids;report.kernelRoot=fixture.scratch;await save();
 browser=await chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage']});
 phase='enroll';await enrollHostedKernel({browser,fixture,origin,evidence,relayPrefix:new URL(origin).origin.replace('https:','wss:').replace('.val.','relay.val.')});
 await fixture.send({UnlockCredentialVault:{passphrase:fixture.passkey,duration:{kind:'until_kernel_stops'},surface:'terminal'}});
 const profile=(await fixture.send({LinkProviderAccountProfile:{provider:'codex',label:'Once unlock drill',path:process.env.VFLAG_CODEX_PROFILE}})).ProviderAccountProfile.profile;
 const worktrees=(await fixture.send({ListWorkspaceWorktrees:{workspace_id:fixture.workspace}})).WorkspaceWorktreesListed.worktrees;
 const created=(await fixture.send({CreateSession:{workspace_id:fixture.workspace,worktree_id:worktrees[0].path,alias:'Once unlock '+expected,agent_defaults:{provider:'codex',model:'gpt-6-luna',effort:'low',account_profile:profile.profile_id}}})).SessionCreated;
 sessionId=created.session.id;const agentId=created.agent.id;
 await fixture.send({AliasAgent:{session_id:sessionId,agent_id:agentId,alias:'Once'}});
 report.sessionId=sessionId;report.agentId=agentId;await save();
 const state=async()=>(await fixture.send({ListSessions:{}})).SessionsListed.sessions.find(s=>s.id===sessionId);
 const cards=async()=>(await state()).active_interactions??[];
 const vaultStatus=async()=>(await fixture.send({GetCredentialVaultStatus:null})).CredentialVaultStatus.status;
 const observedFill=async(tool)=>{
  const outline=(await fixture.send({GetSessionHistoryOutline:{session_id:sessionId,agent_ids:[agentId],latest_prompt_count:1,include_prefetched_content:false}})).SessionHistoryOutline;
  const contents=[];
  for(const agent of outline.agents)for(const turn of agent.turns)for(const blob of turn.blobs.filter(b=>b.kind==='provider_tool')) {
   contents.push(JSON.stringify(await fixture.send({GetSessionHistoryBlobContent:{session_id:sessionId,agent_id:agentId,blob_id:blob.blob_id}})));
  }
  const text=contents.join('\n').replaceAll('\\"','"');
  const name=tool==='chariox.vault.fill_field'?/vault[._]fill_field/:/kernel_browser_paste_secret/;
  assert.ok(name.test(text),'history records the real requested fill tool');
  assert.ok(/"inserted"\s*:\s*true/.test(text),'the real fill tool returned inserted true');
  return {toolObserved:true,resultInserted:true};
 };
 for(const dpr of [1,2]) {
  const context=await browser.newContext({viewport:{width:1440,height:900},deviceScaleFactor:dpr});
  const page=await context.newPage();page.setDefaultTimeout(30000);
  const errors=[];page.on('pageerror',()=>errors.push('pageerror'));
  await signInIfRequired(page,origin,'/terminal?ui=os');await page.goto(origin+'/terminal?ui=os');
  await page.locator(`[data-room-id='${sessionId}'] [data-action=open-room]`).click();
  const prompt=async(text)=>{
   const before=(await state()).agents.find(a=>a.id===agentId)?.last_prompt_sent_at_ms??0;
   await page.locator(`[data-room-pane='${agentId}'] .agent-head strong`).first().click();
   const input=page.locator('[data-prompt-input]');
   await waitFor(async()=>(await input.getAttribute('placeholder'))==='Message Once…','composer targets agent',10000);
   await input.fill(text);await page.locator('[data-action=send-prompt]').click();
   await waitFor(async()=>((await state()).agents.find(a=>a.id===agentId)?.last_prompt_sent_at_ms??0)>before,'prompt persisted',10000);
  };
  const settle=async()=>waitFor(async()=>!(await state()).agents.find(a=>a.id===agentId)?.is_processing,'provider turn settles',180000);
  const unlockOnce=async(waiting=false)=>{
   await page.locator('[data-feature=vault]').click();
   const form=page.locator('.work-editor-popup [data-vault-unlock]');
   await page.locator('.work-editor-popup [data-vault-unlock], [data-vault-window][role=button]').first().waitFor();
   if(!await form.isVisible()) {
    try {await page.locator('[data-vault-window][role=button]').click({timeout:3000});}
    catch(error) {if(!await form.isVisible())throw error;}
   }
   await form.waitFor();await form.locator('input[type=password]').fill(fixture.passkey);
   await form.locator('[data-vault-unlock-duration]').selectOption('once');
   await form.locator('button[type=submit]').click();await form.waitFor({state:'detached'});
   if(!waiting)assert.equal((await vaultStatus()).unlocked,true);
   await page.screenshot({path:evidence+`/once-unlocked-dpr${dpr}.png`});
   const window=page.locator('[data-window-id]').filter({has:page.locator('[data-vault-window]')});
   await window.locator('[data-action=close-window]').click();
  };
  const generate=id=>`Call chariox.vault.generate exactly once with request_id "${id}", origin "https://github.com", description "Single-use owner unlock regression". Wait for its result. Do not call other tools or retry. Report whether it succeeded, without any secrets.`;
  const count=async()=>(await fixture.send({ListCredentials:null})).CredentialsListed.credentials.length;
  if(dpr===1) {
   phase='warmup';await prompt('Call chariox.request_popup exactly once, wait true, title "Once warmup", message "Cancel warmup.", choices [{"id":"cancel","label":"Cancel","reply":"cancel"}], timeout_sec 60. Then finish with Ready. Do not call other tools.');
   const warm=await waitFor(async()=>(await cards()).find(c=>c.title==='Once warmup'),'provider warmup card',180000);
   await page.locator(`[data-interaction-id='${warm.id}'] [data-action=read-interaction]`).click();
   await page.locator(`[data-interaction-detail='${warm.id}'] [data-choice-id=cancel]`).click();await settle();
  }
  phase=`unlock-dpr${dpr}`;await fixture.send({LockCredentialVault:null});
  await unlockOnce();
  const initial=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials;
  const before=initial.length;
  phase=`first-generation-dpr${dpr}`;await prompt(generate(`once-${dpr}-first`));
  await waitFor(async()=>(await count())===before+1,'first credential created',180000);await settle();
  const firstRun=(await fixture.send({GetProviderRun:{provider_run_id:(await state()).active_provider_run_id}})).ProviderRun.provider_run;
  assert.equal(firstRun.model,'gpt-6-luna');
  const cell={dpr,resolvedModel:firstRun.model,resolvedEffort:firstRun.variant,firstCreated:true,lockedAfterFirst:!(await vaultStatus()).unlocked};
  report.cells.push(cell);await save();
  assert.equal(cell.lockedAfterFirst,expected==='green','once closes after one real agent operation');
  await page.screenshot({path:evidence+`/after-first-dpr${dpr}.png`});
  phase=`second-generation-dpr${dpr}`;await prompt(generate(`once-${dpr}-second`));
  if(expected==='green') {
   const card=await waitFor(async()=>(await cards()).find(c=>c.id.startsWith('vault-unlock-')),'next agent call requests a fresh unlock',180000);
   cell.nextAsked=true;await save();assert.equal(await count(),before+1);
   await page.locator(`[data-interaction-id='${card.id}'] [data-action=read-interaction]`).click();
   await page.locator(`[data-interaction-detail='${card.id}'] [data-vault-unlock]`).waitFor();
   await page.screenshot({path:evidence+`/next-needs-unlock-dpr${dpr}.png`});
   // Owner unlocks from the Vault window while the agent card is pending.
   await page.keyboard.press('Escape');await unlockOnce(true);
   await waitFor(async()=>(await count())===before+2,'notified generation succeeds',180000);await settle();
   cell.notificationConsumed=!(await vaultStatus()).unlocked;await save();assert.equal(cell.notificationConsumed,true);
   assert.equal((await cards()).length,0);
   await page.screenshot({path:evidence+`/notification-consumed-dpr${dpr}.png`});

   phase=`browser-fill-dpr${dpr}`;
   const handle=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials.find(c=>!initial.some(old=>old.id===c.id)).id;
   await fixture.send({KernelBrowser:{command:{op:'grant_room_computer',agent_id:agentId}}});
   const title=`GitHub fill ready ${dpr}`;
   await prompt(`Load the kernel browser tools if needed and continue after their automatic reload. Open https://github.com/login in a new tab and take a fresh snapshot. Identify the username text input and the password input, retaining their observed refs and tab/document identity for the next turn. Do not type or submit anything. Then call chariox.request_popup once, wait true, title "${title}", message "The observed GitHub fields are ready.", choices [{"id":"done","label":"Done","reply":"done"}], timeout_sec 60. After Done, finish with Ready.`);
   const ready=await waitFor(async()=>(await cards()).find(c=>c.title===title),'real GitHub fill target observed',180000);
   await page.locator(`[data-interaction-id='${ready.id}'] [data-action=read-interaction]`).click();
   await page.locator(`[data-interaction-detail='${ready.id}'] [data-choice-id=done]`).click();await settle();
   cell.browserFills=[];
   for(const [tool,field]of [['chariox.kernel_browser_paste_secret','password input'],['chariox.vault.fill_field','username text input']]) {
    await unlockOnce();
    const marker=`FILLED_${dpr}_${cell.browserFills.length}`;
    await prompt(`Take a fresh snapshot of the GitHub login tab and identify its ${field}. Then call ${tool} exactly once with credential_id "${handle}", with submit false, using tab_id, generation, document_id and node_ref from that fresh snapshot. Do not submit, navigate, read any secret, or call tools other than the snapshot and this fill. Only if the fill returns inserted true, reply exactly ${marker}. Otherwise report the error.`);
    await waitFor(async()=>(await page.locator(`[data-room-pane='${agentId}'] .pane-transcript-reply`).allTextContents()).some(text=>text.replace(/^[^A-Za-z0-9]+/,'').startsWith(marker)),'real browser fill succeeds',180000);
    await settle();assert.equal((await vaultStatus()).unlocked,false,'browser fill consumes the owner unlock');
    cell.browserFills.push({tool,...await observedFill(tool),vaultLocked:true});await save();
    await page.screenshot({path:evidence+`/fill-${cell.browserFills.length}-consumed-dpr${dpr}.png`});
   }
  } else {
   await waitFor(async()=>(await count())===before+2,'base wrongly permits second credential without unlocking',180000);await settle();
   cell.nextAsked=(await cards()).some(c=>c.id.startsWith('vault-unlock-'));assert.equal(cell.nextAsked,false);
   cell.secondCreatedWithoutUnlock=true;
   await page.screenshot({path:evidence+`/second-bypassed-unlock-dpr${dpr}.png`});
  }
  cell.pageErrors=errors;cell.status=expected==='green'?'PASS':'EXPECTED RED';await save();await context.close();
 }
 report.status=expected==='green'?'PASS':'EXPECTED RED';
} catch(error) {
 report.status='FAIL';report.phase=phase;
 const message=String(error.message);
 report.error=(fixture?message.split(fixture.passkey).join('[redacted]'):message).slice(0,400);
 if(fixture&&sessionId) {
  try {
   const outline=(await fixture.send({GetSessionHistoryOutline:{session_id:sessionId,latest_prompt_count:1,include_prefetched_content:false}})).SessionHistoryOutline;
   report.lastTurns=outline.agents.flatMap(a=>a.turns.map(t=>{
    const text=t.summary?.entry?.text??'';
    return {lifecycle:t.lifecycle,tools:t.blobs.filter(b=>b.kind==='provider_tool').map(b=>b.title),hasSummaryText:Boolean(text),unauthorized:/unauthorized|\b401\b/i.test(text),rateLimited:/\b429\b|response_too_many_failed_attempts/i.test(text)};
   }));
  }catch{report.historyDiagnosticUnavailable=true;}
 }
 process.exitCode=1;
}
finally {
 if(fixture) {
  const parents=new Map();
  for(const name of await readdir('/proc'))if(/^\d+$/.test(name)) {
   try{const stat=await readFile(`/proc/${name}/stat`,'utf8');parents.set(Number(name),Number(stat.slice(stat.lastIndexOf(')')+2).split(' ')[1]));}catch{}
  }
  const owned=new Set(fixture.pids);let added=true;
  while(added){added=false;for(const [pid,parent]of parents)if(owned.has(parent)&&!owned.has(pid)){owned.add(pid);added=true;}}
  report.runtimePids=[...owned];
 }
 if(fixture&&sessionId)await fixture.send({EndSession:{session_id:sessionId}}).catch(()=>{});
 await browser?.close();await fixture?.close();report.closed=true;await save();
}

// MP-08 / MP-09 / MP-10 / MP-11: real owner creation -> official Codex use over hosted WSS.
// Run as a non-root user under boundrun; the coordinator-approved drill login enters only on fd 3.
import assert from 'node:assert/strict'
import { randomBytes, createHash } from 'node:crypto'
import { mkdir, readFile, readdir, writeFile, open as openFile } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import {spawn} from 'node:child_process'
import { chromium } from '/root/work/agent-vflag-cloud/node_modules/playwright-core/index.mjs'
import { startKernelFixture, waitFor } from '/root/work/agent-vflag-cloud/apps/web/scripts/lib/external-access-kernel-fixture.mjs'

const origin = process.env.VFLAG_ORIGIN, evidence = process.env.VFLAG_EVIDENCE, binaryDir = process.env.VFLAG_BIN
assert.ok(/^https:\/\/pr[0-9]+-vflag[a-z0-9]*\.val\.51-255-87-178\.sslip\.io$/.test(origin ?? ''), 'Use an owned hosted vflag stack')
assert.ok(evidence && binaryDir && path.isAbsolute(evidence) && path.isAbsolute(binaryDir))
assert.ok(!path.resolve(evidence).startsWith(path.resolve('../../../') + path.sep), 'Keep evidence outside the repo')
const expected = process.env.VFLAG_EXPECT ?? 'allowed', dpr = Number(process.env.VFLAG_DPR ?? 1)
assert.ok(['allowed','unregistered'].includes(expected))
await mkdir(evidence, { recursive:true })
const marker = `VFLAG_${Date.now()}_DPR${dpr}`, secret = randomBytes(24).toString('hex'), replacement = randomBytes(24).toString('hex')
const privateValues = [secret, replacement], checks = [], errors = []
let baselineToolRefusal = false, mailAccount, mailAccess, capturedSecret
const serviceSecrets=[]
let fixture, browser, context, page, agentId, sessionId, observedRunId
const check = (name, ok) => { checks.push({ name, ok:Boolean(ok) }); assert.ok(ok, name) }
const safe = value => { let text = String(value); for (const secret of privateValues) if (secret) text = text.split(secret).join('[redacted]'); return text }
const shot = async name => {
 if (await page.locator('[data-vault-window]').count() && !/failure/.test(name)) {
  const geometry = await page.locator('[data-vault-lock]').evaluate(el => {
   const box=el.getBoundingClientRect(),scroll=el.closest('[data-vault-credentials]'),clip=scroll.getBoundingClientRect()
   return {top:box.top,bottom:box.bottom,clipTop:clip.top,clipBottom:clip.bottom,scrollTop:scroll.scrollTop}
  })
  check('Vault toolbar visible at rest: '+name,geometry.top>=geometry.clipTop&&geometry.bottom<=geometry.clipBottom)
  checks.push({name:'Vault toolbar geometry: '+name,...geometry})
 }
 return page.screenshot({ path:path.join(evidence, name+'.png') })
}
async function scanArtifacts(directory) {
 let files=0
 for (const item of await readdir(directory,{withFileTypes:true})) {
  if (/kernel-local-auth|vault|config|auth\.json|credentials|\.token$|\.key$|\.pem$/i.test(item.name) || item.isSymbolicLink()) continue
  const filename=path.join(directory,item.name)
  if(item.isDirectory()) files+=await scanArtifacts(filename)
  else if(/\.(json|jsonl|ndjson|log|txt|md|sqlite|sqlite3|db)$/.test(item.name)) {
   const bytes=await readFile(filename); files++
   if(expected==='unregistered') for(const line of bytes.toString('utf8').split('\n')) {
    if(/function_call_output|custom_tool_call_output/.test(line)&&/credential|Vault/i.test(line)&&/unknown|not found|unregistered|not registered/i.test(line)) {
     baselineToolRefusal=true
     await writeFile(path.join(evidence,'baseline-tool-refusal.json'),JSON.stringify({file:path.relative(fixture.scratch,filename),nativeToolOutput:safe(line)},null,2)+'\n')
    }
   }
   const matches=[...privateValues.slice(0,3),...serviceSecrets].filter(Boolean).flatMap((value,index)=>bytes.includes(Buffer.from(value))?[["owner-secret","replacement","vault-passphrase","fixture-relay-token","device-code"][index] ?? 'other-private-field']:[])
   if(matches.length)await writeFile(path.join(evidence,'artifact-match.json'),JSON.stringify({file:path.relative(fixture.scratch,filename),matches},null,2)+'\n')
   check('no Vault value in runtime artifact',matches.length===0)
  }
 }
 return files
}
async function descendants(roots) {
 const records=[]
 for(const name of await readdir('/proc')) {
  if(!/^\d+$/.test(name))continue
  try {const fields=(await readFile(`/proc/${name}/stat`,'utf8')).split(') ').at(-1).split(' ');records.push({pid:Number(name),parent:Number(fields[1]),started:fields[19]})}catch{}
 }
 const owned=new Set(roots),found=[]
 for(let added=true;added;) {added=false;for(const record of records)if(!owned.has(record.pid)&&owned.has(record.parent)){owned.add(record.pid);found.push(record);added=true}}
 return found
}
async function stopSurvivors(records) {
 let stopped=0
 for(const record of [...records].reverse()) {
  if(!Number.isInteger(record.pid)||record.pid<=1)throw Error('Refusing an invalid cleanup PID')
  try {const fields=(await readFile(`/proc/${record.pid}/stat`,'utf8')).split(') ').at(-1).split(' ');if(fields[19]!==record.started||fields[0]==='Z')continue;process.kill(record.pid,'SIGTERM');stopped++}catch(error){if(!['ENOENT','ESRCH'].includes(error.code))throw error}
 }
 return stopped
}
async function agentTurn(prompt, success, failure) {
 await page.evaluate(() => {
  const selector='[data-passkey-prompt],[data-interaction-id],section[data-kernel-interaction-id]'
  const probe={seen:document.querySelectorAll(selector).length}
  probe.observer=new MutationObserver(()=>{probe.seen+=document.querySelectorAll(selector).length})
  probe.observer.observe(document.body,{childList:true,subtree:true});window.__vflagProbe=probe
 })
 const field=page.locator('[data-prompt-input]');await field.fill(prompt);await field.press('Enter')
 await waitFor(async()=>{const state=(await fixture.send({GetSessionState:{session_id:sessionId}})).SessionState;observedRunId=state.agent_activity[agentId]?.active_turn?.provider_run_id ?? state.session.active_provider_run_id;return Boolean(observedRunId)},'native provider run identity',60000)
 await page.waitForFunction(({success,failure})=>[...document.querySelectorAll('.pane-transcript-reply')].some(e=>e.innerText.includes(success)||e.innerText.includes(failure)),{success,failure},{timeout:300000})
 const replies=await page.locator('.pane-transcript-reply').allInnerTexts()
 check(success,replies.some(text=>text.includes(success))&&!replies.some(text=>text.includes(failure)))
 const seen=await page.evaluate(()=>{window.__vflagProbe.observer.disconnect();return window.__vflagProbe.seen})
 check('no approval or sudo interaction during ordinary Vault use',seen===0)
 await shot(success)
}
async function openRoom(sessionId) {
 const open=page.locator(`[data-room-id='${sessionId}'] [data-action=open-room]`)
 await open.waitFor({timeout:60000});await open.click()
}
async function navigate(url) {
 const field=page.locator('.kernel-browser-omnibox').first();await field.waitFor({timeout:60000})
 // Wait for the initial home Tab before typing; otherwise opening it can replace
 // the address entered into the still-empty browser view.
 await page.waitForFunction(()=>[...document.querySelectorAll('.kernel-browser-page-surface')].some(surface=>surface.getBoundingClientRect().width>0&&['mirror','video'].includes(surface.getAttribute('data-paint-mode'))),{},{timeout:60000})
 for(let attempt=0;attempt<3;attempt++) {
  await field.fill(url);await field.press('Enter')
  try {
   await waitFor(async()=>{try{return new URL(await field.inputValue()).hostname===new URL(url).hostname}catch{return false}},'displayed browser URL committed',20000)
   break
  } catch(error) {if(attempt===2)throw error;checks.push({name:'initial navigation retried',attempt:attempt+1})}
 }
 await page.waitForFunction(()=>[...document.querySelectorAll('.kernel-browser-page-surface')].some(surface=>surface.getBoundingClientRect().width>0&&['mirror','video'].includes(surface.getAttribute('data-paint-mode'))),{},{timeout:60000})
 await shot('navigation-'+new URL(url).hostname.replaceAll('.','-'))
}

try {
 fixture=await startKernelFixture(binaryDir,'/usr/local/bin/codex',{kernelEnvironment:{DISPLAY:process.env.DISPLAY,CHARIOX_ROOM_AGENT_TOOLS:'1',CHARIOX_KERNEL_BROWSER_PORT:'0',CHARIOX_KERNEL_BROWSER_EXECUTABLE:process.env.VFLAG_CHROME ?? '/opt/google/chrome/chrome'}})
 privateValues.push(fixture.passkey,fixture.bootstrap.relayToken)
 // The approved lane drill transfer is neither printed nor persisted in evidence.
 const profileHome='/root/.chariox/dev/lanes-miguel-r28/executor/chariox/state/provider-accounts/cmsq37g92003e1z9xz4ekzhzr/codex/codex-1-tzpopphy/codex'
 const profile=(await fixture.send({LinkProviderAccountProfile:{provider:'codex',label:'vflag drill',path:profileHome}})).ProviderAccountProfile.profile
 const worktrees=(await fixture.send({ListWorkspaceWorktrees:{workspace_id:fixture.workspace}})).WorkspaceWorktreesListed.worktrees
 const created=(await fixture.send({CreateSession:{workspace_id:fixture.workspace,worktree_id:worktrees[0].path,alias:'vflag bound Vault',agent_defaults:{provider:'codex',model:process.env.VFLAG_MODEL ?? 'gpt-6-luna',effort:'low',account_profile:profile.profile_id}}})).SessionCreated
 const session=created.session,agent=created.agent
 sessionId=session.id;agentId=agent.id
 check('real Codex Room configured',Boolean(agent?.id))
 browser=await chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage'],...(process.env.SHAPE_PROXY?{proxy:{server:process.env.SHAPE_PROXY,bypass:''}}:{})})
 context=await browser.newContext({viewport:{width:1440,height:1000},deviceScaleFactor:dpr})
 page=await context.newPage();page.setDefaultNavigationTimeout(120000);page.setDefaultTimeout(120000);page.on('pageerror',error=>errors.push(safe(error.message)))
 await page.goto(origin+'/validation/ready')
 const status=(await fixture.send({RelayStatus:null})).RelayStatus.status
 const localProfile=createHash('sha256').update('local').digest('hex')
 const hostProfile=path.join(fixture.scratch,'home','state','kernel-browser',localProfile)
 const rootBrowser=path.join('/var/lib/chariox-browser',createHash('sha256').update(hostProfile).digest('hex'))
 await writeFile(path.join(evidence,'runtime-identity.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',scratch:fixture.scratch,daemon_id:status.daemon_id,host_profile:hostProfile,root_browser:rootBrowser},null,2)+'\n')
 const login=(await fixture.send({StartCloudRelayLogin:{api_url:origin,client_id:'vflag-drill',client_alias:'vflag drill',machine_id:status.machine_id,machine_alias:'vflag kernel'}})).CloudRelayLoginStarted.login
 privateValues.push(login.device_code)
 check('normal hosted device enrollment',new URL(login.verification_url).origin===origin)
 await page.goto(login.verification_url)
 if(await page.getByLabel('Access key',{exact:true}).count()) {
  const access=readFileSync(3,'utf8').trim();privateValues.push(access)
  await page.getByLabel('Email',{exact:true}).fill(process.env.VFLAG_OWNER_EMAIL)
  await page.getByLabel('Access key',{exact:true}).fill(access)
  await page.getByRole('button',{name:'Sign in',exact:true}).click()
 }
 await page.getByRole('button',{name:/approve|allow|connect/i}).click()
 await waitFor(async()=>{
  const result=(await fixture.send({PollCloudRelayLogin:{api_url:origin,device_code:login.device_code}})).CloudRelayLoginPolled.result
  assert.notEqual(result.status,'denied');return result.status==='approved'
 },'normal device enrollment',120000)
 await fixture.send({ConnectCloudRelay:null})
 await waitFor(async()=>{const status=(await fixture.send({RelayStatus:null})).RelayStatus.status;return status.connected&&status.relay_url.startsWith('wss://pr0-vflag')},'hosted WSS')
 const served=page.waitForResponse(response=>/\/assets\/terminal-os\.[a-f0-9]+\.js$/.test(response.url()),{timeout:120000})
 await page.goto(origin+'/terminal?ui=os',{waitUntil:'domcontentloaded'})
 if(process.env.VFLAG_SKIN) {
  await page.locator('[data-action=account-menu]').click()
  await page.locator('[data-action=open-settings]').click()
  await page.locator('[data-settings-section=appearance]').click()
  await page.locator(`[data-theme-card=${process.env.VFLAG_SKIN}]`).click()
  await page.locator(`[data-theme-card=${process.env.VFLAG_SKIN}][aria-checked=true]`).waitFor()
  check('requested skin selected',true)
  await page.locator('.window-frame').filter({has:page.locator('[data-settings-window]')}).locator('[data-action=close-window]').click()
 }
 const bundle=await served
 checks.push({name:'served bundle identity',sha256:createHash('sha256').update(await bundle.body()).digest('hex')})
 await openRoom(session.id)
 await page.keyboard.press('F8');await page.locator('[data-link-developer-tool]').waitFor()
 const modes=page.locator('[data-link-developer-tool] [data-carrier-mode=relay]')
 for(let index=0;index<await modes.count();index++)await modes.nth(index).click()
 await page.waitForTimeout(2500)
 check('encrypted hosted relay selected',await page.locator('[data-link-developer-tool] [data-carrier-mode=relay][aria-pressed=true]').count()>0)
 await page.getByRole('button',{name:'Close inspector',exact:true}).click()
 await page.locator('[data-feature=vault]').click()
 await page.locator('[data-vault-unlock] input').fill(fixture.passkey)
 await page.locator('[data-vault-unlock] button[type=submit]').click()
 await page.locator('[data-vault-unlock]').waitFor({state:'detached'})
 const vaultFrame=page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')})
 if(await vaultFrame.locator('[data-action=maximize-window]').getAttribute('aria-label')==='Maximize window')await vaultFrame.locator('[data-action=maximize-window]').click()
 await page.locator('[data-vault-new]').click()
 const form=page.locator('[data-vault-new-form]')
 await form.getByLabel('Name',{exact:true}).fill(marker)
 await form.getByLabel('Kind',{exact:true}).selectOption('Password')
 await form.getByLabel('Use',{exact:true}).fill('https://github.com')
 await form.getByLabel('Value',{exact:true}).fill(secret)
 await shot('1-owner-create-masked');await form.getByLabel('Value',{exact:true}).press('Enter')
 await form.waitFor({state:'detached'})
 const row=page.locator('[data-vault-entry]').filter({hasText:marker});await row.waitFor()
 const id=await row.getAttribute('data-vault-entry');check('owner-created row has handle',Boolean(id))
 await row.click();await shot('2-owner-saved')
 const saved=page.locator('[data-vault-saved-value]'),reveal=page.locator('[data-vault-reveal]')
 await reveal.hover();await page.mouse.down()
 try { await page.waitForFunction(()=>document.querySelector('[data-vault-saved-value]')?.type==='text');check('owner hold reveals exact value',(await saved.inputValue())===secret) } finally {await page.mouse.up()}
 check('release clears the owner value',(await saved.inputValue())==='')
 const external=await fixture.external()
 try {await assert.rejects(external.send({OwnerVault:{action:'reveal',id}}));check('agent connection cannot reveal',true)} finally {external.close()}
 const registered=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials.find(entry=>entry.id===id)
 check('one registered store for owner-created value',expected==='unregistered'?!registered:registered?.metadata?.use_binding?.target?.origin==='https://github.com')
 if(process.env.VFLAG_SCAN_ONLY!=='1') {
 await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click()
 await openRoom(session.id)
 if(!await page.locator('.kernel-browser-window').count()) {await page.locator('[data-action=open-room-view]').last().click();await page.locator('[data-open-room-view=browser]').click()}
 await navigate('https://github.com/login','Sign in to GitHub')
 const ready=marker+'_READY',failed=marker+'_SETUP_FAILED'
 await page.locator('[data-prompt-input]').fill(`MP-08 / MP-09 / MP-10 / MP-11 setup: I want you to use my displayed kernel browser. Call chariox.load_kernel_browser once using necessary tool discovery wrappers, then get its state. I will approve ordinary browser access. Do not use sudo or Vault. If load_kernel_browser requests a provider reload, follow that instruction and continue after the automatic refresh; that is normal setup, not a failure. Finish exactly ${ready} only after a successful browser state call; use ${failed} only for an actual tool error after the tools have loaded.`)
 await page.locator('[data-prompt-input]').press('Enter')
 await waitFor(async()=>{
  const cards=page.locator('section[data-kernel-interaction-id]')
  for(let index=0;index<await cards.count();index++) {
   const card=cards.nth(index),text=(await card.getAttribute('aria-label') ?? '')+' '+await card.innerText()
   check('setup approves only browser resource access',/Chariox resource access/i.test(text)&&/browser/i.test(text)&&!/(sudo|payment card)/i.test(text))
   await card.locator('button[data-kernel-interaction-choice=allow]').click()
  }
  const replies=await page.locator('.pane-transcript-reply').allInnerTexts();assert.ok(!replies.some(text=>text.includes(failed)))
  return replies.some(text=>text.includes(ready))
 },'real Codex browser admission',300000)
 await page.waitForFunction(()=>!document.querySelector('section[data-kernel-interaction-id]'))
 if(process.env.VFLAG_DNS_ONLY!=='1') {
 if(expected==='unregistered') {
  await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 baseline regression: I created the Vault entry ${id} in New secret for https://github.com. Actually call chariox.kernel_browser_paste_secret once for that handle and the observed GitHub Password field, submit=false. Do not generate a replacement, read a value, use sudo or ask approval. Finish ${marker}_UNREGISTERED only if the actual tool refuses it as unknown/unregistered; otherwise ${marker}_UNEXPECTED.`,marker+'_UNREGISTERED',marker+'_UNEXPECTED')
 } else {
  const fill=async(label)=>agentTurn(`MP-08 / MP-09 / MP-10 / MP-11: I created the Vault entry ${id} for https://github.com. Use the existing kernel browser tab: get a fresh snapshot, then chariox.kernel_browser_paste_secret with this handle into its Password field, submit=false. Do not sign in, submit, request sudo or approval, read/copy/re-display the value, or generate another entry. Get a model snapshot and screenshot afterward; the field must stay masked. Actually perform the tool calls. Finish exactly ${marker}_${label} on success or ${marker}_FAILED on failure.`,marker+'_'+label,marker+'_FAILED')
  await fill('FILLED')
  let activity=(await fixture.send({OwnerVault:{action:'activity',id}})).OwnerVault.result.events
  check('actual successful agent fill recorded',activity.some(event=>event.event.startsWith('Used in browser by ')))
  await navigate('https://www.djangoproject.com/accounts/login/','Log in')
  await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 origin refusal: get a fresh snapshot of the displayed Django login. Actually try chariox.kernel_browser_paste_secret with ${id} into its password input, submit=false. The entry belongs to https://github.com. Do not change or regenerate it, submit, request sudo/approval or read its value. Finish exactly ${marker}_REFUSED only if the tool refuses this different destination; otherwise ${marker}_UNEXPECTED.`,marker+'_REFUSED',marker+'_UNEXPECTED')
  activity=(await fixture.send({OwnerVault:{action:'activity',id}})).OwnerVault.result.events
  check('actual wrong-origin refusal recorded',activity.some(event=>event.event.startsWith('Use refused')))
  check('Activity preserves owner views',activity.some(event=>event.event==='Viewed by owner'))
  await page.locator('[data-feature=vault]').click();await page.locator(`[data-vault-entry='${id}']`).click()
  await page.getByText(/Used in browser by /).first().waitFor();await page.getByText(/Use refused/).first().waitFor();await shot('5-owner-activity')
  await page.locator('[data-vault-replace]').click();await page.locator('[data-vault-replacement]').fill(replacement);await page.locator('[data-vault-replacement]').press('Enter')
  await page.locator('[data-vault-replacement]').waitFor({state:'detached'});await page.getByText('Replaced by owner',{exact:true}).waitFor()
  const after=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials.find(entry=>entry.id===id)
  check('Replace preserves the original binding',JSON.stringify(after.metadata.use_binding)===JSON.stringify(registered.metadata.use_binding))
  await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click()
  await openRoom(session.id);await navigate('https://github.com/login','Sign in to GitHub');await fill('REPLACED_FILLED')
  const projection=(await fixture.send({GetSessionState:{session_id:session.id}})).SessionState
  const state=JSON.stringify(projection)
  check('session projection contains no Vault values',privateValues.every(value=>!state.includes(value)))
  const runId=observedRunId
  check('provider run observed',Boolean(runId))
  const run=(await fixture.send({GetProviderRun:{provider_run_id:runId}})).ProviderRun.provider_run
  check('resolved Codex model and effort',run.model===(process.env.VFLAG_MODEL ?? 'gpt-6-luna')&&run.variant==='low')
  checks.push({name:'provider identity',model:run.model,effort:run.variant})
  await writeFile(path.join(evidence,'activity.json'),JSON.stringify({id,events:activity},null,2)+'\n')
 }
 }

 }
 if(process.env.VFLAG_MAIL==='1') {
  // Public real account. The credential request stays in private runtime state;
  // capture_command consumes stdout and the model receives only a Vault handle.
  const domain=(await (await fetch('https://api.mail.tm/domains')).json())['hydra:member'].find(item=>item.isActive).domain
  const address=`chariox-vflag-${Date.now()}@${domain}`, password=randomBytes(32).toString('hex')
  privateValues.push(password)
  const created=await fetch('https://api.mail.tm/accounts',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({address,password})})
  check('real Mail.tm disposable account created',created.status===201)
  mailAccount=await created.json()
  const accessReply=await fetch('https://api.mail.tm/token',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({address,password})})
  check('Mail.tm account supports API login',accessReply.status===200)
  mailAccess=(await accessReply.json()).token;privateValues.push(mailAccess)
  const red=await fetch('https://api.mail.tm/me')
  check('AU-02 red: service refuses absent credential',red.status===401)
  await writeFile(path.join(evidence,'au02-red.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',service:'https://api.mail.tm/me',status:red.status,authenticated:false})+'\n')
  await writeFile(path.join(fixture.workspace,'mail-auth.json'),JSON.stringify({address,password}),{mode:0o600})
  await writeFile(path.join(fixture.workspace,'mail-capture.py'),`import json,urllib.request\np=json.load(open('mail-auth.json'))\nr=urllib.request.Request('https://api.mail.tm/token',data=json.dumps(p).encode(),headers={'Content-Type':'application/json'})\nx=json.load(urllib.request.urlopen(r,timeout=25))\nprint(x['token'])\n`)
  await writeFile(path.join(fixture.workspace,'mail-use.py'),`import os,json,urllib.request\nr=urllib.request.Request('https://api.mail.tm/me',headers={'Authorization':'Bearer '+os.environ['MAILTM_TOKEN']})\nx=json.load(urllib.request.urlopen(r,timeout=25))\nassert x['id']==${JSON.stringify(mailAccount.id)}\nproof={'authorized':True,'account_matches':True,'status':200}\nopen('mail-proof.json','w').write(json.dumps(proof))\nprint(json.dumps(proof))\n`)
  if(process.env.VFLAG_DNS_ONLY!=='1') {
  await openRoom(session.id)
  await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-02: use chariox.vault.capture_command (discover the existing runtime tool) with request_id ${marker}-mail, argv ["/usr/bin/python3","mail-capture.py"], host api.mail.tm, label ${marker}-Mailtm, extract {"kind":"stdout"}${process.env.VFLAG_BINDING==='bound'?', use_binding {"target":{"kind":"command_environment","name":"MAILTM_TOKEN","origin":"https://api.mail.tm"}}':''}. This authorized command prints a freshly minted real-service API credential directly into the Vault. Do not run that script in any other tool, read mail-auth.json, reveal the secret, or use sudo. Then use chariox.vault.use_env with argv ["/usr/bin/python3","mail-use.py"], origin https://api.mail.tm and env {"MAILTM_TOKEN":the returned credential_id}. The command returns authorized true only when the real service accepts it for this account. Finish exactly ${marker}_MAIL_AUTHORIZED only after actual tool success with authorized true; otherwise ${marker}_MAIL_FAILED.`,marker+'_MAIL_AUTHORIZED',marker+'_MAIL_FAILED')
  const proof=JSON.parse(await readFile(path.join(fixture.workspace,'mail-proof.json'),'utf8'));check('real command received accepted service response',proof.authorized&&proof.account_matches&&proof.status===200)
  const creds=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials
  const captured=creds.find(entry=>entry.description===marker+'-Mailtm')
  check('AU-02 captured credential registered',Boolean(captured))
  const revealed=(await fixture.send({OwnerVault:{action:'reveal',id:captured.id}})).OwnerVault.result
  capturedSecret=revealed.value
  check('AU-02 owner-only private absence scanner received captured value',typeof capturedSecret==='string'&&capturedSecret.length>20)
  privateValues.push(capturedSecret);serviceSecrets.push(capturedSecret)
  const bindingExpected=process.env.VFLAG_BINDING==='bound'
  for(const [name,destination,variable] of [['origin','https://example.com','MAILTM_TOKEN'],['name','https://api.mail.tm','OTHER_TOKEN']]) {
   const suffix=`${marker}_GUARD_${name.toUpperCase()}`
   await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 one-use boundary probe: actually call chariox.vault.use_env with origin ${destination}, argv ["/usr/bin/touch","wrong-${name}.ran"], env {"${variable}":"${captured.id}"}. This command does no network I/O and prints no value; its marker records whether the kernel executes this different declared use. Do not use any other command. Finish exactly ${suffix}_REFUSED if the actual tool refuses this destination, ${suffix}_EXECUTED if it executes, or ${suffix}_FAILED on another error.`,suffix+(bindingExpected?'_REFUSED':'_EXECUTED'),suffix+'_FAILED')
   const ran=await readFile(path.join(fixture.workspace,`wrong-${name}.ran`)).then(()=>true).catch(error=>{if(error.code==='ENOENT')return false;throw error})
   check(`AU-02 ${bindingExpected?'green':'red'}: wrong ${name} ${bindingExpected?'refused':'executed'}`,ran===!bindingExpected)
  }
  await writeFile(path.join(evidence,'capture-binding.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',expected:process.env.VFLAG_BINDING,credential_id:captured.id,binding:captured.metadata?.use_binding??null},null,2)+'\n')
  await page.locator('[data-feature=vault]').click()
  const capturedRow=page.locator(`[data-vault-entry='${captured.id}']`);await capturedRow.waitFor();await shot('au02-captured-row-masked')
  checks.push({name:'AU-02 binding metadata',binding:captured.metadata?.use_binding??null})
  await writeFile(path.join(evidence,'au02-green.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',service:'https://api.mail.tm/me',authorized:true,credential_id:captured.id,binding:captured.metadata?.use_binding??null},null,2)+'\n')
  if(process.env.VFLAG_EXTRA==='1') {
   await capturedRow.click()
   const rotated=(await (await fetch('https://api.mail.tm/token',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({address,password})})).json()).token
   check('real service issues replacement token',typeof rotated==='string'&&rotated.length>20&&rotated!==capturedSecret)
   privateValues.push(rotated);serviceSecrets.push(rotated)
   await page.locator('[data-vault-replace]').click();await page.locator('[data-vault-replacement]').fill(rotated);await page.locator('[data-vault-replacement]').press('Enter')
   await page.locator('[data-vault-replacement]').waitFor({state:'detached'});await shot('au11-owner-rotated-masked')
   const changed=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials.find(entry=>entry.id===captured.id)
   check('AU-11 replacement preserves capture binding',JSON.stringify(changed.metadata.use_binding)===JSON.stringify(captured.metadata.use_binding))
   await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click();await openRoom(session.id)
   await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-11: the owner replaced Vault entry ${captured.id} using its existing Replace control. Actually call chariox.vault.use_env again with argv ["/usr/bin/python3","mail-use.py"], origin https://api.mail.tm, env {"MAILTM_TOKEN":"${captured.id}"}. Then call chariox.mandate.list_resources (this legacy namespace is the existing resource register, no mandate approval) and verify the captured credential is listed with kind/service/creator attribution. Finish exactly ${marker}_ROTATED if the API succeeds and register contains the entry, otherwise ${marker}_ROTATE_FAILED. Never reveal a value or use sudo.`,marker+'_ROTATED',marker+'_ROTATE_FAILED')
   const revokeScript=`import os,json,urllib.request
r=urllib.request.Request('https://api.mail.tm/accounts/${mailAccount.id}',method='DELETE',headers={'Authorization':'Bearer '+os.environ['MAILTM_TOKEN']})
x=urllib.request.urlopen(r,timeout=25)
assert x.status==204
open('revoke-proof.json','w').write(json.dumps({'revoked':True,'status':204}))
print('revoked')
`
   await writeFile(path.join(fixture.workspace,'mail-revoke.py'),revokeScript)
   await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-11 owner revocation request: delete this disposable Mail.tm account using chariox.vault.use_env, argv ["/usr/bin/python3","mail-revoke.py"], origin https://api.mail.tm, env {"MAILTM_TOKEN":"${captured.id}"}. I authorize deleting only this drill-created account. Do not use another tool for the script or reveal the token. Finish exactly ${marker}_REVOKED after tool success, otherwise ${marker}_REVOKE_FAILED.`,marker+'_REVOKED',marker+'_REVOKE_FAILED')
   const revoked=JSON.parse(await readFile(path.join(fixture.workspace,'revoke-proof.json'),'utf8'))
   check('AU-11 actual service revocation completed',revoked.revoked&&revoked.status===204)
   const denied=await fetch('https://api.mail.tm/me',{headers:{Authorization:'Bearer '+rotated}})
   check('AU-11 revoked credential refused by real service',denied.status===401)
   await writeFile(path.join(evidence,'au11-rotation-revocation.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',rotated:true,preserved_binding:true,revoked:true,status_after_revocation:denied.status,inventory:'real provider list_resources checked',limitation:'Mail.tm account revocation; no dedicated provider key-revocation endpoint or owner Created tab'},null,2)+'\n')
  }

  }
  if(process.env.VFLAG_DNS_ONLY==='1') {

   const dnsPassword=randomBytes(24).toString('hex');privateValues.push(dnsPassword);serviceSecrets.push(dnsPassword)
   await page.locator('[data-feature=vault]').click()
   await page.locator('[data-vault-new]').click();const form=page.locator('[data-vault-new-form]')
   await form.getByLabel('Name',{exact:true}).fill(marker+'-Dynv6-password');await form.getByLabel('Kind',{exact:true}).selectOption('Password')
   await form.getByLabel('Use',{exact:true}).fill('https://dynv6.com');await form.getByLabel('Value',{exact:true}).fill(dnsPassword);await form.getByLabel('Value',{exact:true}).press('Enter');await form.waitFor({state:'detached'})
   const dnsCred=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials.find(c=>c.description===marker+'-Dynv6-password');check('DNS password saved for exact signup origin',Boolean(dnsCred))
   await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click();await openRoom(session.id)
   await navigate('https://dynv6.com/users/sign_up')
   await writeFile(path.join(fixture.workspace,'dynv6-client.py'),await readFile('/root/.chariox/dev/lanes-miguel-r28/agents/vflag/tmp/dynv6-client.py'))
   const signup=await new Promise((resolve,reject)=>{
    const child=spawn('/usr/bin/python3',['dynv6-client.py','signup'],{cwd:fixture.workspace,stdio:['pipe','pipe','pipe']});let out='',err=''
    child.stdout.on('data',data=>out+=data);child.stderr.on('data',data=>err+=data);child.on('error',reject)
    child.on('close',code=>{if(code!==0)reject(Error('normal owner signup client failed'));else{try{resolve(JSON.parse(out))}catch{reject(Error('normal owner signup response invalid'))}}})
    child.stdin.end(JSON.stringify({email:address,password:dnsPassword}))
   })
   const registered=signup.registered===true
   await writeFile(path.join(evidence,'dns-owner-signup.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',method:'normal observed Rails signup form; private owner setup, no CAPTCHA bypass',...signup})+'\n')
   if(registered) {
    await writeFile(path.join(fixture.workspace,'dynv6-auth.json'),JSON.stringify({email:address,password:dnsPassword}),{mode:0o600})
    let verify
    await waitFor(async()=>{
     const list=await (await fetch('https://api.mail.tm/messages',{headers:{Authorization:'Bearer '+mailAccess}})).json()
     for(const message of list['hydra:member']??[]) {
      const content=await (await fetch('https://api.mail.tm/messages/'+message.id,{headers:{Authorization:'Bearer '+mailAccess}})).json()
      const body=[content.text,...(Array.isArray(content.html)?content.html:[content.html??''])].join('\n').replaceAll('&amp;','&')
      const found=body.match(/https:\/\/dynv6\.com\/users\/confirmation\?confirmation_token=[A-Za-z0-9_-]+/)
      if(found){verify=found[0];return true}
     }
     return false
    },'actual dynv6 confirmation email',180000)
    privateValues.push(verify)
    const omnibox=page.locator('.kernel-browser-omnibox').first();await omnibox.fill(verify);await omnibox.press('Enter')
    await page.waitForTimeout(3000)
    await omnibox.fill('https://dynv6.com/users/sign_in');await omnibox.press('Enter');await page.waitForTimeout(2000)
    await shot('dns-email-confirmed')
    const ctx={fixture,page,agentTurn,navigate,shot,check,evidence,marker,address,dnsCred,privateValues,serviceSecrets}
    await writeFile(path.join(evidence,'dns-ready.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',registered:true,confirmation_requested:true,workspace:fixture.workspace,password_handle:dnsCred.id})+'\n')
    const follow='/root/.chariox/dev/lanes-miguel-r28/agents/vflag/tmp/dns-followup.mjs'
    await waitFor(async()=>await readFile(follow).then(()=>true).catch(()=>false),'owned DNS continuation prepared',240000)
    await (await import(follow+'?run='+Date.now())).run(ctx)
   }
  }

 }

 if(process.env.VFLAG_EXTRA==='1') {
  await openRoom(session.id)
  await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-15: actually call chariox.agent_auth.register with request_id ${marker}-firecrawl, resource_url https://api.firecrawl.dev/, resource_metadata_url https://www.firecrawl.dev/.well-known/oauth-protected-resource, flow anonymous. Firecrawl currently supports only identity assertions, so record the kernel's actual refusal. Do not fabricate an assertion or use another account. Finish exactly ${marker}_FIRECRAWL_BLOCKED if the tool explicitly refuses the unavailable issuer-free flow, ${marker}_FIRECRAWL_UNEXPECTED otherwise.`,marker+'_FIRECRAWL_BLOCKED',marker+'_FIRECRAWL_UNEXPECTED')
  await writeFile(path.join(evidence,'au15-blocker.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',service:'https://www.firecrawl.dev',flow:'anonymous',result:'actual real-provider kernel registration refused',green:false})+'\n')
  await navigate('https://codeberg.org/user/sign_up')
  await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-10 signup preflight: inspect the current public Codeberg registration page using the kernel browser snapshot and screenshot. Do not submit a form, solve or avoid any bot check. If an anti-bot challenge blocks registration, finish exactly ${marker}_TOTP_SIGNUP_BLOCKED and identify that challenge; otherwise finish ${marker}_TOTP_SIGNUP_AVAILABLE.`,marker+'_TOTP_SIGNUP_BLOCKED',marker+'_TOTP_SIGNUP_AVAILABLE')
  await writeFile(path.join(evidence,'au10-signup-blocker.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',site:'https://codeberg.org/user/sign_up',result:'real browser anti-bot challenge observed; not solved',totp_enrollment:false})+'\n')

 }
 check('no browser page errors',errors.length===0)
 const day=new Date().toISOString().slice(0,10).replaceAll('-',path.sep)
 const nativeRoot=path.join(profileHome,'sessions',day), nativeFiles=[], refusals=[]
 for(const item of await readdir(nativeRoot,{withFileTypes:true})) {
  if(!item.isFile()||!item.name.endsWith('.jsonl'))continue
  const filename=path.join(nativeRoot,item.name),fd=await openFile(filename,'r'),head=Buffer.alloc(16384)
  let first
  try{const {bytesRead}=await fd.read(head,0,head.length,0);first={payload:{cwd:head.subarray(0,bytesRead).toString('utf8').match(/"cwd"\s*:\s*"([^"]+)"/)?.[1]}}}finally{await fd.close()}
  if(first?.payload?.cwd!==fixture.workspace)continue
  const bytes=await readFile(filename)
  check('no Vault value in own official provider transcript',[...privateValues.slice(0,3),...serviceSecrets].filter(Boolean).every(secret=>!bytes.includes(Buffer.from(secret))))
  nativeFiles.push(filename)
  for(const line of bytes.toString('utf8').split('\n'))if(/function_call_output|custom_tool_call_output/.test(line)&&/different use|issuer-free flow|Anonymous or Service Auth/i.test(line))refusals.push(JSON.parse(safe(line)))
 }
 check('official native provider history checked',nativeFiles.length>0)
 await writeFile(path.join(evidence,'native-history-scan.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',files:nativeFiles,clean:true,refusals},null,2)+'\n')
 const scanned=await scanArtifacts(fixture.scratch)
 checks.push({name:'artifact scan',files:scanned})
 if(expected==='unregistered')check('native provider tool output records the baseline refusal',baselineToolRefusal)
 await writeFile(path.join(evidence,'result.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',expected,dpr,scanOnly:process.env.VFLAG_SCAN_ONLY==='1',checks},null,2)+'\n')
 console.log(JSON.stringify({result:'PASS',expected,dpr,checks:checks.length}))
} catch(error) {
 if(page) await shot('failure').catch(()=>{})
 const surfaces=page?await page.locator('[data-paint-mode]').evaluateAll(elements=>elements.map(element=>({mode:element.getAttribute('data-paint-mode'),visible:element.getBoundingClientRect().width>0,frames:element.querySelectorAll('iframe').length}))).catch(()=>[]):[]
 await writeFile(path.join(evidence,'failure.json'),JSON.stringify({error:safe(error.stack ?? error),checks,errors,surfaces},null,2)+'\n')
 if(fixture)await writeFile(path.join(evidence,'kernel.log'),safe(fixture.diagnostics()))
 console.error(safe(error.message ?? error));process.exitCode=1
} finally {
 if(mailAccount&&mailAccess) {const removed=await fetch('https://api.mail.tm/accounts/'+mailAccount.id,{method:'DELETE',headers:{Authorization:'Bearer '+mailAccess}}).catch(()=>null);await writeFile(path.join(evidence,'mailtm-cleanup.json'),JSON.stringify({status:removed?.status??null,deleted:[204,404].includes(removed?.status)})+'\n')}
 if(context) await context.close().catch(()=>{})
 if(browser) await browser.close().catch(()=>{})
 if(fixture) {
  const owned=await descendants(fixture.pids)
  await fixture.close()
  const stopped=await stopSurvivors(owned)
  await writeFile(path.join(evidence,'cleanup.json'),JSON.stringify({fixturePids:fixture.pids,descendants:owned,stopped},null,2)+'\n')
 }
}

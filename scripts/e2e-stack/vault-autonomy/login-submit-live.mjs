// MP-08 / MP-09 / MP-10 / MP-11: an official agent signs in to a real login form with one
// chariox.kernel_browser_paste_secret submit=true call. The form's submit navigates; the call must
// report the landing page (submitted.navigated) and the agent must continue without pasting again.
// Sites: LSUB_SITE=gitea (self-hosted Gitea) or public (the-internet.herokuapp.com, a public demo
// login that publishes its credentials for exactly this).
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdir, readFile, readdir, writeFile, unlink, open as openFile } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import {spawn} from 'node:child_process'
import {pathToFileURL} from 'node:url'
import {startGitea} from './gitea-service.mjs'
import {signInIfRequired} from '/w/valenv/scripts/sign-in.mjs'

const cloudRepo=process.env.LSUB_CLOUD_REPO, profileHome=process.env.LSUB_PROFILE_HOME, giteaBinary=process.env.LSUB_GITEA
const site=process.env.LSUB_SITE??'gitea', providerName=process.env.LSUB_PROVIDER??'codex'
assert.ok(['gitea','public'].includes(site)&&['codex','claude'].includes(providerName))
assert.ok([cloudRepo,process.env.LSUB_BUILD_MANIFEST,...(site==='gitea'?[giteaBinary]:[]),...(providerName==='codex'?[profileHome]:[])].every(value=>value&&path.isAbsolute(value)), 'Set absolute Cloud (playwright), Gitea binary, codex profile and build-manifest paths')
const {chromium}=await import(pathToFileURL(path.join(cloudRepo,'node_modules/playwright-core/index.mjs')).href)
const {startKernelFixture,waitFor}=await import(pathToFileURL(path.join(cloudRepo,'apps/web/scripts/lib/external-access-kernel-fixture.mjs')).href)

const origin = process.env.LSUB_ORIGIN, evidence = process.env.LSUB_EVIDENCE, binaryDir = process.env.LSUB_BIN
assert.ok(/^https:\/\/pr[0-9]+-vpaste[a-z0-9]*\.val\.51-255-87-178\.sslip\.io$/.test(origin ?? ''), 'Use an owned hosted vflag stack')
assert.ok(evidence && binaryDir && path.isAbsolute(evidence) && path.isAbsolute(binaryDir))
assert.ok(!path.resolve(evidence).startsWith(path.resolve('.')+path.sep), 'Keep evidence outside the repo')
const dpr = Number(process.env.LSUB_DPR ?? 1)
assert.ok([1,2].includes(dpr))
assert.match(process.env.LSUB_CLOUD_HEAD??'',/^[a-f0-9]{40}$/, 'Pin the exact Cloud head')
await mkdir(evidence, { recursive:true })
await writeFile(path.join(evidence,'source.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',kernel_manifest:JSON.parse(await readFile(process.env.LSUB_BUILD_MANIFEST,'utf8')),cloud_head:process.env.LSUB_CLOUD_HEAD,origin,dpr,provider:providerName,site,model:process.env.LSUB_MODEL??(providerName==='claude'?'claude-haiku-5-5':'gpt-6-luna'),harness_sha256:createHash('sha256').update(await readFile(new URL(import.meta.url))).digest('hex'),client_shaper:process.env.SHAPE_PROXY??null,performance_claims:false},null,2)+'\n')

const marker = `LSUB${site.toUpperCase()}${providerName.toUpperCase()}_${Date.now()}_DPR${dpr}`
const privateValues = [], checks = [], errors = []
const serviceSecrets=[], codeValues=[]
// Which kind of Vault-derived value a runtime artifact holds (never the value itself). A code is six digits,
// so it only counts as a whole number, not inside a longer one such as a timestamp.
const leaks = bytes => {
 const found=[]
 if(serviceSecrets.filter(Boolean).some(value=>bytes.includes(Buffer.from(value))))found.push('password-or-seed')
 if(codeValues.length&&new RegExp('(?<![0-9])(?:'+codeValues.join('|')+')(?![0-9])').test(bytes.toString('latin1')))found.push('one-time-code')
 return found
}
let fixture, browser, context, page, agentId, sessionId, observedRunId, gitea, captured, lastReplies=[]
const check = (name, ok) => { checks.push({ name, ok:Boolean(ok) }); assert.ok(ok, name) }
const safe = value => { let text = String(value); for (const secret of [...privateValues,...serviceSecrets]) if (secret) text = text.split(secret).join('[redacted]'); return text }
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
   const matches=leaks(bytes)
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
  probe.observer.observe(document.body,{childList:true,subtree:true});window.__lsubProbe=probe
 })
 const field=page.locator('[data-prompt-input]');await field.fill(prompt);await field.press('Enter')
 await waitFor(async()=>{const state=(await fixture.send({GetSessionState:{session_id:sessionId}})).SessionState;observedRunId=state.agent_activity[agentId]?.active_turn?.provider_run_id ?? state.session.active_provider_run_id;return Boolean(observedRunId)},'native provider run identity',60000)
 await page.waitForFunction(({success,failure})=>[...document.querySelectorAll('.pane-transcript-reply')].some(e=>{const text=e.innerText.replace(/^[^A-Za-z0-9]+/,'');return text.startsWith(success)||text.startsWith(failure)}),{success,failure},{timeout:600000})
 const replies=await page.locator('.pane-transcript-reply').allInnerTexts()
 // The user's own prompt text is a transcript entry too: only an answer that starts with the marker counts.
 lastReplies=replies.map(text=>safe(text).slice(0,400))
 const answer=text=>text.replace(/^[^A-Za-z0-9]+/,'')
 check(success,replies.some(text=>answer(text).startsWith(success))&&!replies.some(text=>answer(text).startsWith(failure)))
 const seen=await page.evaluate(()=>{window.__lsubProbe.observer.disconnect();return window.__lsubProbe.seen})
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
   await waitFor(async()=>{try{return new URL(await page.locator('.kernel-browser-page-surface').last().getAttribute('data-browser-url')).hostname===new URL(url).hostname}catch{return false}},'actual browser surface URL committed',20000)
   break
  } catch(error) {if(attempt===2)throw error;checks.push({name:'initial navigation retried',attempt:attempt+1})}
 }
 await page.waitForFunction(()=>[...document.querySelectorAll('.kernel-browser-page-surface')].some(surface=>surface.getBoundingClientRect().width>0&&['mirror','video'].includes(surface.getAttribute('data-paint-mode'))),{},{timeout:60000})
 await shot('navigation-'+new URL(url).hostname.replaceAll('.','-'))
}

const publicSite={origin:'https://the-internet.herokuapp.com',user:'tomsmith',password:'SuperSecretPassword!',loginPath:'/login'}
const pastes=[]
// Every paste_secret call of the run, with its outcome, from the official provider transcript.
async function transcriptPastes(files) {
 const calls=new Map()
 for(const filename of files) {
  for(const line of (await readFile(filename,'utf8')).split('\n').filter(Boolean)) {
   let entry;try{entry=JSON.parse(line)}catch{continue}
   const text=JSON.stringify(entry)
   if(!/kernel_browser_paste_secret/.test(text))continue
   const payload=entry.payload??entry.message??entry
   // Codex: function_call / function_call_output pairs. Claude: tool_use / tool_result content blocks.
   const blocks=[payload,...(Array.isArray(payload.content)?payload.content:[])]
   for(const block of blocks) {
    const id=block.call_id??block.id??block.tool_use_id
    if(!id)continue
    if(['function_call','tool_use'].includes(block.type)&&/kernel_browser_paste_secret/.test(block.name??''))calls.set(id,{id,output:null})
    if(['function_call_output','tool_result'].includes(block.type)&&calls.has(id))calls.get(id).output=safe(JSON.stringify(block.output??block.content)).slice(0,600)
   }
  }
 }
 return [...calls.values()]
}
try {
 fixture=await startKernelFixture(binaryDir,providerName==='claude'?'/root/.local/bin/claude':'/usr/local/bin/codex',{kernelEnvironment:{DISPLAY:process.env.DISPLAY,CHARIOX_ROOM_AGENT_TOOLS:'1',CHARIOX_KERNEL_BROWSER_PORT:'0',CHARIOX_KERNEL_BROWSER_EXECUTABLE:process.env.LSUB_CHROME ?? '/opt/google/chrome/chrome',...(providerName==='claude'?{IS_SANDBOX:'1'}:{})}})
 privateValues.push(fixture.passkey,fixture.bootstrap.relayToken)
 let profile
 if(providerName==='codex') {
  profile=(await fixture.send({LinkProviderAccountProfile:{provider:'codex',label:'vpaste drill',path:profileHome}})).ProviderAccountProfile.profile
 } else {
  // The #917 setup-token enrolment: the lane account's token goes to the kernel's Vault, never to a file or log.
  const token=readFileSync(process.env.LSUB_CLAUDE_TOKEN_FILE,'utf8').trim();privateValues.push(token)
  profile=(await fixture.send({CreateProviderAccountProfile:{provider:'claude',label:'vpaste drill'}})).ProviderAccountProfile.profile
  await fixture.send({SetProviderAccountCredential:{session_id:null,agent_id:null,provider:'claude',account_profile:profile.profile_id,value:token,run:false,overwrite:false}})
  const verified=(await fixture.send({RefreshProviderAccountProfile:{provider:'claude',account_profile:profile.profile_id}})).ProviderAccountProfile.profile
  check('Claude drill profile authenticated',verified.auth_state!=='Unauthorized')
 }
 const worktrees=(await fixture.send({ListWorkspaceWorktrees:{workspace_id:fixture.workspace}})).WorkspaceWorktreesListed.worktrees
 const model=process.env.LSUB_MODEL??(providerName==='claude'?'claude-haiku-5-5':'gpt-6-luna')
 const created=(await fixture.send({CreateSession:{workspace_id:fixture.workspace,worktree_id:worktrees[0].path,alias:'vpaste login submit',agent_defaults:{provider:providerName,model,effort:'low',account_profile:profile.profile_id}}})).SessionCreated
 const session=created.session,agent=created.agent
 sessionId=session.id;agentId=agent.id
 check('real '+providerName+' Room configured',Boolean(agent?.id))
 browser=await chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage'],...(process.env.SHAPE_PROXY?{proxy:{server:process.env.SHAPE_PROXY,bypass:''}}:{})})
 context=await browser.newContext({viewport:{width:1440,height:1000},deviceScaleFactor:dpr})
 page=await context.newPage();page.setDefaultNavigationTimeout(120000);page.setDefaultTimeout(120000);page.on('pageerror',error=>errors.push(safe(error.message)))
 await page.goto(origin+'/validation/ready')
 const status=(await fixture.send({RelayStatus:null})).RelayStatus.status
 const login=(await fixture.send({StartCloudRelayLogin:{api_url:origin,client_id:'vpaste-drill',client_alias:'vpaste drill',machine_id:status.machine_id,machine_alias:'vpaste kernel'}})).CloudRelayLoginStarted.login
 privateValues.push(login.device_code)
 check('normal hosted device enrollment',new URL(login.verification_url).origin===origin)
 await signInIfRequired(page,origin,'/terminal?ui=os');await page.goto(login.verification_url)
 await page.getByRole('button',{name:/approve|allow|connect/i}).click()
 await waitFor(async()=>{
  const result=(await fixture.send({PollCloudRelayLogin:{api_url:origin,device_code:login.device_code}})).CloudRelayLoginPolled.result
  assert.notEqual(result.status,'denied');return result.status==='approved'
 },'normal device enrollment',120000)
 await fixture.send({ConnectCloudRelay:null})
 await waitFor(async()=>{const status=(await fixture.send({RelayStatus:null})).RelayStatus.status;return status.connected&&status.relay_url.startsWith('wss://pr0-vpaste')},'hosted WSS')
 const served=page.waitForResponse(response=>/\/assets\/terminal-os\.[a-f0-9]+\.js$/.test(response.url()),{timeout:120000})
 await page.goto(origin+'/terminal?ui=os',{waitUntil:'domcontentloaded'})
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
 await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click()
 await openRoom(session.id)
 if(!await page.locator('.kernel-browser-window').count()) {await page.locator('[data-action=open-room-view]').last().click();await page.locator('[data-open-room-view=browser]').click()}
 let target
 if(site==='gitea') {
  const giteaPassword='Gp-'+createHash('sha256').update(String(Date.now())+Math.random()).digest('hex').slice(0,28)
  serviceSecrets.push(giteaPassword)
  gitea=await startGitea({binary:giteaBinary,home:'/home/ubuntu/vpaste-gitea-'+process.pid,user:'drillagent',password:giteaPassword})
  target={origin:gitea.origin,user:'drillagent',password:giteaPassword,loginPath:'/user/login'}
 } else target=publicSite
 serviceSecrets.push(target.password)
 const created_password=await fixture.send({OwnerVault:{action:'create',kind:'Password',name:marker+' site password',value:target.password,use_binding:{target:{kind:'site',origin:target.origin},value_kind:'secret'}}})
 const passwordId=created_password.OwnerVault.result.id
 check('owner stored the site password in the Vault for the one site',Boolean(passwordId))
 await navigate(target.origin+target.loginPath)
 const ready=marker+'_READY',failed=marker+'_SETUP_FAILED'
 await page.locator('[data-prompt-input]').fill(`vpaste setup: I want you to use my displayed kernel browser. Call chariox.load_kernel_browser once using necessary tool discovery wrappers, then get its state. I will approve ordinary browser access. Do not use sudo or Vault. If load_kernel_browser requests a provider reload, follow that instruction and continue after the automatic refresh; that is normal setup, not a failure. Finish exactly ${ready} only after a successful browser state call; use ${failed} only for an actual tool error after the tools have loaded.`)
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
 },'real '+providerName+' browser admission',300000)
 await page.waitForFunction(()=>!document.querySelector('section[data-kernel-interaction-id]'))
 await page.waitForFunction(ready=>[...document.querySelectorAll('.pane-transcript-reply')].filter(e=>e.innerText.replace(/^[^A-Za-z0-9]+/,'').startsWith(ready)).length>=2,ready,{timeout:30000}).catch(()=>checks.push({name:'no second setup run observed'}))
 const done=marker+'_LOGGED_IN',stuck=marker+'_LOGIN_FAILED'
 // The task never mentions the MD-2 error or a retry: how the agent reads the paste result is what is measured.
 await agentTurn(`vpaste login ${marker}: The kernel browser is already loaded. Use chariox.kernel_browser_* tools on my displayed browser; take a fresh snapshot before every action (tab_id, generation, document_id and node_ref come from it). The page ${target.origin}${target.loginPath} is open. Sign in as the user ${target.user}: type the user name into its field with the browser input tool, then fill the password field with Vault credential ${passwordId} using chariox.kernel_browser_paste_secret with submit=true (the field belongs to a native form, so no button click is needed). A call rejected for its argument shape is normal: read the error, correct the arguments and retry. Never type or print the password yourself. When you are signed in, finish with the word ${done} followed by a space and the title of the page you landed on; if you could not sign in, finish with ${stuck}.`,done,stuck)
 if(site==='gitea') check('Gitea session shows the signed-in dashboard',(await fetch(target.origin+'/api/healthz')).ok)
 const vaultWindow=async name=>{
  await page.locator('[data-feature=vault]').click();await page.locator('[data-vault-window]').waitFor();await page.waitForTimeout(800)
  await shot(name)
  await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click()
 }
 await vaultWindow('vault-window-after-login')
 check('no browser page errors',errors.length===0)
 // The official provider's own transcript: how many pastes the agent made and what each returned.
 const transcriptRoot=providerName==='codex'?path.join(profileHome,'sessions'):path.join(fixture.scratch,'home','state','provider-accounts','local','claude')
 const files=[]
 const walk=async directory=>{for(const item of await readdir(directory,{withFileTypes:true}).catch(()=>[])){const filename=path.join(directory,item.name);if(item.isDirectory())await walk(filename);else if(item.name.endsWith('.jsonl'))files.push(filename)}}
 await walk(transcriptRoot)
 const own=[];for(const filename of files){if((await readFile(filename,'utf8')).includes(marker))own.push(filename)}
 check('official native provider history found',own.length>0)
 const calls=await transcriptPastes(own);pastes.push(...calls)
 checks.push({name:'paste_secret calls in the provider transcript',count:calls.length,outputs:calls.map(call=>call.output)})
 check('agent pasted exactly once',calls.length===1)
 check('the paste result reports the submit',calls.length>=1&&/submitted/.test(calls[0].output??'')&&/navigated/.test(calls[0].output??''))
 check('no host-browser error in any paste result',calls.every(call=>!/MD-2|operation failed|browser_controller_internal|error/i.test(call.output??'')))
 for(const filename of own){const bytes=await readFile(filename);check('no Vault value in own official provider transcript',leaks(bytes).length===0&&!serviceSecrets.some(value=>value&&bytes.includes(Buffer.from(value))&&site==='gitea'))}
 const scanned=await scanArtifacts(fixture.scratch)
 checks.push({name:'artifact scan',files:scanned})
 await writeFile(path.join(evidence,'result.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',dpr,provider:providerName,site,checks},null,2)+'\n')
 console.log(JSON.stringify({result:'PASS',dpr,provider:providerName,site,checks:checks.length}))
} catch(error) {
 if(page) await shot('failure').catch(()=>{})
 await writeFile(path.join(evidence,'failure.json'),JSON.stringify({error:safe(error.stack ?? error),checks,errors,pastes,lastReplies},null,2)+'\n')
 if(fixture)await writeFile(path.join(evidence,'kernel.log'),safe(fixture.diagnostics()))
 console.error(safe(error.message ?? error));process.exitCode=1
} finally {
 await gitea?.stop().catch(()=>{})
 if(context) await context.close().catch(()=>{})
 if(browser) await browser.close().catch(()=>{})
 if(fixture) {
  const owned=await descendants(fixture.pids)
  await fixture.close()
  const stopped=await stopSurvivors(owned)
  await writeFile(path.join(evidence,'cleanup.json'),JSON.stringify({fixturePids:fixture.pids,descendants:owned,stopped},null,2)+'\n')
 }
}

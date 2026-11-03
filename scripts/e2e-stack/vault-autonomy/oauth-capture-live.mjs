// MP-08 / MP-09 / MP-10 / MP-11: an official agent enrols a real Gitea authenticator by capturing
// the setup key or QR code from the page, then signs in twice in different TOTP periods.
// Sign-in uses the validation helper, which fills the generated key privately.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdir, readFile, readdir, writeFile, unlink, open as openFile } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import {spawn} from 'node:child_process'
import {pathToFileURL} from 'node:url'
import {createHmac} from 'node:crypto'
import {startGitea} from './gitea-service.mjs'
import {startRelyingParty} from './oauth-relying-party.mjs'
import {signInIfRequired} from '/w/valenv/scripts/sign-in.mjs'

const cloudRepo=process.env.TCAP_CLOUD_REPO, profileHome=process.env.TCAP_PROFILE_HOME, giteaBinary=process.env.TCAP_GITEA
assert.ok([cloudRepo,profileHome,giteaBinary,process.env.TCAP_BUILD_MANIFEST].every(value=>value&&path.isAbsolute(value)), 'Set absolute Cloud (playwright), provider profile, Gitea binary and build-manifest paths')
const mode=process.env.TCAP_MODE??'key'
assert.ok(['key','qr'].includes(mode))
const {chromium}=await import(pathToFileURL(path.join(cloudRepo,'node_modules/playwright-core/index.mjs')).href)
const {startKernelFixture,waitFor}=await import(pathToFileURL(path.join(cloudRepo,'apps/web/scripts/lib/external-access-kernel-fixture.mjs')).href)

const origin = process.env.TCAP_ORIGIN, evidence = process.env.TCAP_EVIDENCE, binaryDir = process.env.TCAP_BIN
assert.ok(/^https:\/\/pr[0-9]+-v(gaps|flag)[a-z0-9]*\.val\.51-255-87-178\.sslip\.io$/.test(origin ?? ''), 'Use an owned hosted vflag stack')
assert.ok(evidence && binaryDir && path.isAbsolute(evidence) && path.isAbsolute(binaryDir))
assert.ok(!path.resolve(evidence).startsWith(path.resolve('.')+path.sep), 'Keep evidence outside the repo')
const dpr = Number(process.env.TCAP_DPR ?? 1)
assert.ok([1,2].includes(dpr))
assert.match(process.env.TCAP_CLOUD_HEAD??'',/^[a-f0-9]{40}$/, 'Pin the exact Cloud head')
await mkdir(evidence, { recursive:true })
await writeFile(path.join(evidence,'source.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',kernel_manifest:JSON.parse(await readFile(process.env.TCAP_BUILD_MANIFEST,'utf8')),cloud_head:process.env.TCAP_CLOUD_HEAD,origin,dpr,model:process.env.TCAP_MODEL??'gpt-6-luna',harness_sha256:createHash('sha256').update(await readFile(new URL(import.meta.url))).digest('hex'),client_shaper:process.env.SHAPE_PROXY??null,performance_claims:false},null,2)+'\n')

const BASE32='ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
// RFC 6238 SHA-1/30 s/6 digits, as Gitea checks: every code near the run, for the absence scans only.
function otpCodes(seed) {
 let bits='';for(const char of seed.replace(/=+$/,''))bits+=BASE32.indexOf(char).toString(2).padStart(5,'0')
 const key=Buffer.from(bits.match(/.{8}/g).map(byte=>parseInt(byte,2)))
 const first=Math.floor(Date.now()/30000)-4,codes=[]
 for(let counter=first;counter<first+60;counter++) {
  const message=Buffer.alloc(8);message.writeBigUInt64BE(BigInt(counter))
  const digest=createHmac('sha1',key).update(message).digest(),offset=digest[19]&15
  codes.push(String((digest.readUInt32BE(offset)&0x7fffffff)%1000000).padStart(6,'0'))
 }
 return codes
}
const marker = `TCAP${mode.toUpperCase()}_${Date.now()}_DPR${dpr}`
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
  // The kernel's bundled prompt defaults are static text, not runtime output: a six-digit code can match them by chance.
  if (/kernel-local-auth|vault|config|auth\.json|credentials|\.token$|\.key$|\.pem$|^\.bundled-defaults\.json$/i.test(item.name) || item.isSymbolicLink()) continue
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
  probe.observer.observe(document.body,{childList:true,subtree:true});window.__vgapsProbe=probe
 })
 const field=page.locator('[data-prompt-input]');await field.fill(prompt);await field.press('Enter')
 await waitFor(async()=>{const state=(await fixture.send({GetSessionState:{session_id:sessionId}})).SessionState;observedRunId=state.agent_activity[agentId]?.active_turn?.provider_run_id ?? state.session.active_provider_run_id;return Boolean(observedRunId)},'native provider run identity',60000)
 await page.waitForFunction(({success,failure})=>[...document.querySelectorAll('.pane-transcript-reply')].some(e=>{const text=e.innerText.replace(/^[^A-Za-z0-9]+/,'');return text.startsWith(success)||text.startsWith(failure)}),{success,failure},{timeout:600000})
 const replies=await page.locator('.pane-transcript-reply').allInnerTexts()
 // The user's own prompt text is a transcript entry too: only an answer that starts with the marker counts.
 lastReplies=replies.map(text=>safe(text).slice(0,400))
 const answer=text=>text.replace(/^[^A-Za-z0-9]+/,'')
 check(success,replies.some(text=>answer(text).startsWith(success))&&!replies.some(text=>answer(text).startsWith(failure)))
 const seen=await page.evaluate(()=>{window.__vgapsProbe.observer.disconnect();return window.__vgapsProbe.seen})
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

try {
 fixture=await startKernelFixture(binaryDir,'/usr/local/bin/codex',{kernelEnvironment:{DISPLAY:process.env.DISPLAY,CHARIOX_ROOM_AGENT_TOOLS:'1',CHARIOX_KERNEL_BROWSER_PORT:'0',CHARIOX_KERNEL_BROWSER_EXECUTABLE:process.env.TCAP_CHROME ?? '/opt/google/chrome/chrome'}})
 privateValues.push(fixture.passkey,fixture.bootstrap.relayToken)
 // The approved lane drill transfer is neither printed nor persisted in evidence.
 const profile=(await fixture.send({LinkProviderAccountProfile:{provider:'codex',label:'vgaps drill',path:profileHome}})).ProviderAccountProfile.profile
 const worktrees=(await fixture.send({ListWorkspaceWorktrees:{workspace_id:fixture.workspace}})).WorkspaceWorktreesListed.worktrees
 const created=(await fixture.send({CreateSession:{workspace_id:fixture.workspace,worktree_id:worktrees[0].path,alias:'vgaps TOTP capture',agent_defaults:{provider:'codex',model:process.env.TCAP_MODEL ?? 'gpt-6-luna',effort:'low',account_profile:profile.profile_id}}})).SessionCreated
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
 const login=(await fixture.send({StartCloudRelayLogin:{api_url:origin,client_id:'vgaps-drill',client_alias:'vgaps drill',machine_id:status.machine_id,machine_alias:'vgaps kernel'}})).CloudRelayLoginStarted.login
 privateValues.push(login.device_code)
 check('normal hosted device enrollment',new URL(login.verification_url).origin===origin)
 await signInIfRequired(page,origin,'/terminal?ui=os');await page.goto(login.verification_url)
 await page.getByRole('button',{name:/approve|allow|connect/i}).click()
 await waitFor(async()=>{
  const result=(await fixture.send({PollCloudRelayLogin:{api_url:origin,device_code:login.device_code}})).CloudRelayLoginPolled.result
  assert.notEqual(result.status,'denied');return result.status==='approved'
 },'normal device enrollment',120000)
 await fixture.send({ConnectCloudRelay:null})
 await waitFor(async()=>{const status=(await fixture.send({RelayStatus:null})).RelayStatus.status;return status.connected&&status.relay_url.startsWith(`wss://${new URL(origin).hostname.split(".")[0]}`)},'hosted WSS')
 const served=page.waitForResponse(response=>/\/assets\/terminal-os\.[a-f0-9]+\.js$/.test(response.url()),{timeout:120000})
 await page.goto(origin+'/terminal?ui=os',{waitUntil:'domcontentloaded'})
 if(process.env.TCAP_SKIN) {
  await page.locator('[data-action=account-menu]').click()
  await page.locator('[data-action=open-settings]').click()
  await page.locator('[data-settings-section=appearance]').click()
  await page.locator(`[data-theme-card=${process.env.TCAP_SKIN}]`).click()
  await page.locator(`[data-theme-card=${process.env.TCAP_SKIN}][aria-checked=true]`).waitFor()
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
 await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click()
 await openRoom(session.id)
 if(!await page.locator('.kernel-browser-window').count()) {await page.locator('[data-action=open-room-view]').last().click();await page.locator('[data-open-room-view=browser]').click()}
 const giteaPassword='Gp-'+createHash('sha256').update(String(Date.now())+Math.random()).digest('hex').slice(0,28)
 serviceSecrets.push(giteaPassword)
 gitea=await startGitea({binary:giteaBinary,home:'/home/ubuntu/vgaps-gitea-oauth-'+process.pid,user:'drillagent',password:giteaPassword})
 check('real Gitea service running without two-factor',!await gitea.twoFactorEnabled())
 const created_password=await fixture.send({OwnerVault:{action:'create',kind:'Password',name:marker+' Gitea password',value:giteaPassword,use_binding:{target:{kind:'site',origin:gitea.origin},value_kind:'secret'}}})
 const passwordId=created_password.OwnerVault.result.id
 check('owner stored the service password in the Vault for the one site',Boolean(passwordId))
 await navigate(gitea.origin+'/user/login')
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
 // Loading the browser reloads the provider; the kernel then continues the setup prompt once more. Let that
 // second run end so it cannot queue behind (or replace the tools of) the real turns.
 await page.waitForFunction(ready=>[...document.querySelectorAll('.pane-transcript-reply')].filter(e=>e.innerText.replace(/^[^A-Za-z0-9]+/,'').startsWith(ready)).length>=2,ready,{timeout:30000}).catch(()=>checks.push({name:'no second setup run observed'}))
 const appName=marker+' relying app',envPath=path.join(fixture.workspace,'.env')
 const common=`The kernel browser is already loaded. Use chariox.kernel_browser_* tools on my displayed browser; take a fresh snapshot before every action (tab_id, generation, document_id and node_ref come from it). Sign in to Gitea at ${gitea.origin} as the user drillagent: type the user name drillagent, and fill the password field with Vault credential ${passwordId} using chariox.kernel_browser_paste_secret with submit=false, then click the Sign In button (the browser pane is small; buttons can be below the visible area). A call rejected for its argument shape is normal: read the error, correct the arguments and retry (up to five attempts at each step) before treating the step as failed. Never type or print a password or client secret yourself.`
 const rp=await startRelyingParty(gitea.origin,path.join(fixture.workspace,'.env'))
 try {
  await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-07 app ${marker}: ${common} Then open ${gitea.origin}/user/settings/applications . Under "Manage OAuth2 Applications" create a new application: Application Name "${appName}", Redirect URI ${rp.redirectUri} (one line), keep Confidential Client checked, click Create Application. The next page shows the Client ID and the Client Secret once. Read the Client ID from the page yourself (it is public). Call chariox.vault.capture_browser (read="node") with node_ref set to the element that shows the client secret, request_id ${marker}-secret, label "${appName} client secret" and use_binding {target:{kind:"file",path:"${envPath}"},value_kind:"secret"} (use the exact field shape from the tool schema); you receive only a credential_id. Then call chariox.vault.write_file with path ".env", template "CLIENT_ID=<the client id>\nCLIENT_SECRET={{CLIENT_SECRET}}\n" and values {CLIENT_SECRET:<that credential_id>}. Never print the client secret or read .env back. Finish exactly ${marker}_APP_CONFIGURED once .env was written, otherwise ${marker}_APP_FAILED naming the first failing tool and its error text.`,marker+'_APP_CONFIGURED',marker+'_APP_FAILED')
  captured=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials.find(c=>c.description===appName+' client secret')
  check('agent captured the OAuth client secret into the Vault',Boolean(captured))
  checks.push({name:'captured entry shape',binding:captured.metadata?.use_binding,created_by_kind:captured.metadata?.created_by_kind})
  check('entry is a secret bound to the workspace .env file only',captured.metadata?.use_binding?.target?.kind==='file'&&captured.metadata.use_binding.target.path===envPath)
  const envText=await readFile(envPath,'utf8').catch(()=> '')
  const clientSecret=envText.match(/^CLIENT_SECRET=(.+)$/m)?.[1],clientId=envText.match(/^CLIENT_ID=(.+)$/m)?.[1]
  check('.env holds the client id and a real client secret written from the handle',Boolean(clientId)&&Boolean(clientSecret)&&clientSecret.length>=20)
  serviceSecrets.push(clientSecret)
  check('the application exists in Gitea with the exact redirect URI',gitea.oauthApplications().some(app=>app.client_id===clientId&&app.redirect_uris.includes(rp.redirectUri)))
  await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-07 login ${marker}: ${common} The relying party app is already running with the configuration you wrote. In the kernel browser open ${rp.origin}/login . It sends you to Gitea's authorize page for "${appName}": click Authorize Application (a consent step; do nothing else). You then land back on the app's callback page. Finish exactly ${marker}_OAUTH_OK if that page says "OAuth login OK: drillagent", otherwise ${marker}_OAUTH_FAILED with the page text.`,marker+'_OAUTH_OK',marker+'_OAUTH_FAILED')
  check('the relying party exchanged the code with the captured secret and read the user',rp.log.some(entry=>entry.exchange===200&&entry.login==='drillagent'))
  check('Gitea recorded the owner consent grant',gitea.oauthGrants()>=1)
  await writeFile(path.join(evidence,'oauth-roundtrip.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',relying_party_log:rp.log,gitea_grants:gitea.oauthGrants(),redirect_uri:rp.redirectUri},null,2)+'\n')
  const vaultWindow=async name=>{
   await page.locator('[data-feature=vault]').click();await page.locator('[data-vault-window]').waitFor();await page.waitForTimeout(800)
   await shot(name)
   await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click()
  }
  await vaultWindow('vault-window-after-oauth')
 } finally {
  await rp.stop()
  await gitea?.stop().catch(()=>{})
  gitea=null
 }
 check('no browser page errors',errors.length===0)
 const day=new Date().toISOString().slice(0,10).replaceAll('-',path.sep)
 const nativeRoot=path.join(profileHome,'sessions',day), nativeFiles=[]
 for(const item of await readdir(nativeRoot,{withFileTypes:true})) {
  if(!item.isFile()||!item.name.endsWith('.jsonl'))continue
  const filename=path.join(nativeRoot,item.name),fd=await openFile(filename,'r'),head=Buffer.alloc(16384)
  let first
  try{const {bytesRead}=await fd.read(head,0,head.length,0);first={payload:{cwd:head.subarray(0,bytesRead).toString('utf8').match(/"cwd"\s*:\s*"([^"]+)"/)?.[1]}}}finally{await fd.close()}
  if(first?.payload?.cwd!==fixture.workspace)continue
  const bytes=await readFile(filename)
  // The service's own page shows the setup key, so a snapshot taken before the capture holds it: that is page
  // content, not a Vault value. After the capture the entry is protected, and the agent's own words never hold it.
  const lines=bytes.toString('utf8').split('\n').filter(Boolean)
  const afterCapture=lines.slice(Math.max(0,lines.findIndex(line=>captured&&line.includes(captured.id))+1))
  const authored=lines.filter(line=>{try{const payload=JSON.parse(line).payload;return payload?.type==='function_call'||(payload?.type==='message'&&payload.role==='assistant')}catch{return false}})
  const report={password_anywhere:bytes.includes(Buffer.from(serviceSecrets[0])),
   seed_after_capture:serviceSecrets.slice(1).some(value=>Buffer.from(afterCapture.join('\n')).includes(Buffer.from(value))),
   seed_in_agent_words:serviceSecrets.slice(1).some(value=>Buffer.from(authored.join('\n')).includes(Buffer.from(value))),
   code_anywhere:leaks(Buffer.from(lines.join('\n'))).includes('one-time-code'),
   seed_in_page_snapshot_before_capture:serviceSecrets.slice(1).some(value=>bytes.includes(Buffer.from(value)))}
  // Which transcript entries hold a computed code, with the digits masked: this tells a page snapshot from a tool result.
  const codePattern=codeValues.length?new RegExp('(?<![0-9])(?:'+codeValues.join('|')+')(?![0-9])','g'):null
  report.code_entries=codePattern?lines.flatMap(line=>{
   codePattern.lastIndex=0;const match=codePattern.exec(line);if(!match)return []
   let payload;try{payload=JSON.parse(line).payload}catch{payload={}}
   return [{type:payload?.type,name:payload?.name,call_id:payload?.call_id,context:line.slice(Math.max(0,match.index-160),match.index).replace(/[0-9]{6}/g,'<six-digits>')+'<CODE>'+line.slice(match.index+6,match.index+120).replace(/[0-9]{6}/g,'<six-digits>')}]
  }).slice(0,6):[]
  checks.push({name:'provider transcript classes',report})
  check('no Vault value in own official provider transcript',!report.password_anywhere&&!report.seed_after_capture&&!report.seed_in_agent_words&&!report.code_anywhere)
  nativeFiles.push(filename)
 }
 check('official native provider history checked',nativeFiles.length>0)
 await writeFile(path.join(evidence,'native-history-scan.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',files:nativeFiles,clean:true},null,2)+'\n')
 const scanned=await scanArtifacts(fixture.scratch)
 checks.push({name:'artifact scan',files:scanned})
 await writeFile(path.join(evidence,'result.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',dpr,checks},null,2)+'\n')
 console.log(JSON.stringify({result:'PASS',dpr,checks:checks.length}))
} catch(error) {
 if(page) await shot('failure').catch(()=>{})
 const surfaces=page?await page.locator('[data-paint-mode]').evaluateAll(elements=>elements.map(element=>({mode:element.getAttribute('data-paint-mode'),visible:element.getBoundingClientRect().width>0,frames:element.querySelectorAll('iframe').length}))).catch(()=>[]):[]
 await writeFile(path.join(evidence,'failure.json'),JSON.stringify({error:safe(error.stack ?? error),checks,errors,surfaces,lastReplies},null,2)+'\n')
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

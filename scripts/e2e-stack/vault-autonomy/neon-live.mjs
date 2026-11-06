// MP-08 / MP-09 / MP-10 / MP-11: real owner creation -> official Codex use over hosted WSS.
// Run as a non-root user under boundrun; the coordinator-approved drill login enters only on fd 3.
import assert from 'node:assert/strict'
import { randomBytes, createHash } from 'node:crypto'
import { mkdir, readFile, readdir, writeFile, open as openFile } from 'node:fs/promises'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import {pathToFileURL} from 'node:url'
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
 await page.waitForFunction(({success,failure})=>[...document.querySelectorAll('.pane-transcript-reply')].some(e=>e.innerText.includes(success)||e.innerText.includes(failure)),{success,failure},{timeout:600000})
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

 await page.locator('.window-frame').filter({has:page.locator('[data-vault-window]')}).locator('[data-action=close-window]').click()
 await openRoom(session.id)
 const neonOrigin='https://claimable.neon.tech'
 await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-15: actually call the Chariox kernel tool chariox.agent_auth.register (discover the runtime tool; it may be exposed as agent_auth_register) with request_id ${marker}-neon, resource_url ${neonOrigin}/, resource_metadata_url ${neonOrigin}/.well-known/oauth-protected-resource, flow anonymous, label "neon ${marker}" and return_fields ["/project/id","/project/expires_at"]. Do not supply an email, do not claim, do not use sudo or another account. The tool returns the credential handle id and those two non-secret fields. Then call chariox.http_request_with_credential with the handle for GET ${neonOrigin}/v1/projects/<project id from fields> (the response holds no secret) and report the HTTP status, the project id and the expiry. Remember the handle and project id for later. Finish exactly ${marker}_NEON_REGISTERED if registration and that call returned 200, ${marker}_NEON_FAILED otherwise.`,marker+'_NEON_REGISTERED',marker+'_NEON_FAILED')
 const credentials=(await fixture.send({ListCredentials:null})).CredentialsListed.credentials
 const neon=credentials.find(entry=>entry.metadata?.use_binding?.target?.origin===neonOrigin)
 check('auth.md registration landed in the Vault bound to the Neon origin only',Boolean(neon))
 await writeFile(path.join(evidence,'au15-neon-registration.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',service:neonOrigin,flow:'anonymous',entry:{id:neon?.id,metadata:neon?.metadata}},null,2)+'\n')
 const workspaceEnv=path.join(fixture.workspace,'.env')
 await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-06: get a Neon project's database URL into ${workspaceEnv} WITHOUT the URL, token or password ever appearing in any tool result or message. Work in ${fixture.workspace}. (1) Write neon_register.py: it POSTs {"type":"anonymous"} to ${neonOrigin}/v1/agent/identity, exchanges the returned identity_assertion at ${neonOrigin}/v1/oauth2/token (form fields grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer, assertion, resource=${neonOrigin}/), writes the new project's id to project.txt and prints ONLY the access_token on stdout. (2) Call chariox.vault.capture_command with a unique request_id, argv ["python3","neon_register.py"], cwd ${fixture.workspace}, host ${new URL(neonOrigin).host} and use_binding target {kind:"command_environment", name:"NEON_TOKEN", origin:"${neonOrigin}"}. (3) Write neon_fetch.py: it GETs ${neonOrigin}/v1/projects/<id from project.txt>/credentials with header Authorization: Bearer <environment variable NEON_TOKEN>, writes the JSON body to creds.json and prints only the HTTP status. Run it with chariox.vault.use_env (origin ${neonOrigin}, env {NEON_TOKEN: <the captured handle>}). (4) Call chariox.vault.capture_file on ${fixture.workspace}/creds.json with extract {kind:"json",pointer:"/database_url"}, host ${new URL(neonOrigin).host}, use_binding target {kind:"file",path:"${workspaceEnv}"}, replace_with_handle true and a unique request_id. (5) Call chariox.vault.write_file for path .env with template "DATABASE_URL={{DATABASE_URL}}\n" and values {DATABASE_URL: <the file handle from step 4>}. Never cat, print, grep or echo creds.json, project.txt output of secrets or .env. Finish exactly ${marker}_NEON_ENV_WRITTEN when .env exists, ${marker}_NEON_ENV_FAILED otherwise.`,marker+'_NEON_ENV_WRITTEN',marker+'_NEON_ENV_FAILED')
 const envText=await readFile(workspaceEnv,'utf8').catch(()=> '')
 const databaseUrl=envText.match(/^DATABASE_URL=(.+)$/m)?.[1]
 check('.env holds a real Neon connection string written from the Vault handle',/^postgres(ql)?:\/\/[^:]+:[^@]+@[^/]+\/.+/.test(databaseUrl ?? ''))
 if(databaseUrl) {serviceSecrets.push(databaseUrl);try{serviceSecrets.push(new URL(databaseUrl).password)}catch{}}
 await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 AU-06: write a small app ${fixture.workspace}/app.mjs that reads DATABASE_URL from ${workspaceEnv}, connects to Postgres, runs "select current_database() as db, now() as server_time, version() as v" plus creates and fills a temporary table with three rows and counts them, and prints only db, server_time, the first 20 characters of v and the count (never the URL). Install the needed client (npm install pg in ${fixture.workspace}), run the app and report its output. Never print .env. Finish exactly ${marker}_NEON_QUERIED if the query succeeded, ${marker}_NEON_QUERY_FAILED otherwise.`,marker+'_NEON_QUERIED',marker+'_NEON_QUERY_FAILED')
 const probeLines=(await page.locator('.pane-transcript-reply').allInnerTexts()).filter(text=>text.includes(marker+'_NEON_QUERIED'))
 await writeFile(path.join(evidence,'au06-neon-app-output.json'),JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',reply:safe(probeLines.join('\n'))},null,2)+'\n')
 try {
  const {default:pg}=await import(pathToFileURL(path.join('/root/work/agent-vflag-cloud/node_modules/.pnpm/pg@8.22.0/node_modules/pg/lib/index.js')).href)
  const client=new pg.Client({connectionString:databaseUrl,ssl:{rejectUnauthorized:true},connectionTimeoutMillis:30000});await client.connect()
  const answer=await client.query('select current_database() as db');await client.end()
  check('the Neon service itself accepts the captured connection string (independent harness query)',Boolean(answer.rows[0].db))
 } catch(error) {check('the Neon service itself accepts the captured connection string (independent harness query): '+safe(error.message),false)}
 await agentTurn(`MP-08 / MP-09 / MP-10 / MP-11 cleanup of two disposable Neon projects: (1) The first project (auth.md handle): record it in the resource register with chariox.mandate.record_resource (discover the runtime tool; request_id unique, service claimable.neon.tech, kind neon-project, name "neon ${marker}", url ${neonOrigin}/v1/projects/<project id>, revoke {kind:"http", request:{credential_id:<the auth.md handle>, url:"${neonOrigin}/v1/projects/<project id>", method:"DELETE"}}), call chariox.mandate.list_resources and report the entry's kind, service and who created it, then call chariox.mandate.revoke_resource for it (Neon answers 204 and revokes the token), then GET ${neonOrigin}/v1/projects/<project id> with the handle once more (expect 401). (2) For the project created by neon_register.py, write neon_delete.py that sends DELETE ${neonOrigin}/v1/projects/<id from project.txt> with header Authorization: Bearer <environment variable NEON_TOKEN> and prints only the HTTP status; run it with chariox.vault.use_env (origin ${neonOrigin}, env {NEON_TOKEN: <the captured command handle>}). Report only HTTP statuses. Finish exactly ${marker}_NEON_CLEANUP_DONE whatever the statuses are.`,marker+'_NEON_CLEANUP_DONE',marker+'_NEON_CLEANUP_UNEXPECTED')
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

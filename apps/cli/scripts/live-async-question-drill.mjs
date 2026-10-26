#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11 AM-ASK: real kernels, relay, TUI and Codex.
// Run under systemd-run --scope -p MemoryMax=4G -p MemorySwapMax=0 -- timeout 900.
import assert from 'node:assert/strict'
import {randomBytes, createHash} from 'node:crypto'
import {mkdir, mkdtemp, readFile, writeFile, rm} from 'node:fs/promises'
import {createWriteStream} from 'node:fs'
import {execFileSync} from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import {fileURLToPath, pathToFileURL} from 'node:url'
import {LocalIpcClient} from '../../../packages/kernel-client/dist/ipc.js'
import {openTui, publicSnapshot} from '../../../scripts/e2e-stack/context-switch/tui.mjs'
import {spawnOwned, signalOwnedProcess, ownedProcessHandles} from '../../kernel/slice-linux-docker/owned-process-signals.mjs'

const tree = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const variant = (r, name) => { assert(r[name], `Expected ${name}, got ${Object.keys(r)}`); return r[name] }
const assistantText = snapshot => (snapshot.transcript?.entries??[])
  .filter(entry=>entry.role==='assistant').map(entry=>entry.text).join('\n')
const required = key => { assert(process.env[key], `Set ${key}`); return process.env[key] }
async function until(label, read, predicate = Boolean, timeout = 90_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) { const value = await read(); if (predicate(value)) return value; await sleep(250) }
  throw Error(`Timed out: ${label}`)
}

// This runs from an official provider tool on the outside kernel. It knows no
// owner's token or profile: only the private socket and public Room id.
if (process.argv[2] === '--follower') {
  const [socket, room, output] = process.argv.slice(3)
  const client = new LocalIpcClient(`ws+unix://${socket}`)
  try {
    let refused = false
    try { await client.send({GetSessionState:{session_id:room}}) } catch { refused = true }
    assert(refused, 'An ungranted outside process must be refused')
    const granted = variant(await client.send({RequestKernelAccess:{holder_pid:process.pid,lifetime_minutes:5}}), 'KernelAccessGranted').grant
    const session = variant(await client.send({GetSessionState:{session_id:room}}), 'SessionState').session
    const question = session.active_interactions.find(q => q.title === 'AMASK primary')
    assert(question?.asking_task_id)
    const request = {RespondToInteraction:{session_id:room,interaction_id:question.id,choice_id:'continue'}}
    await client.send(request)
    let secondRefused = false
    try { await client.send(request) } catch { secondRefused = true }
    assert(secondRefused, 'A second answer must be refused')
    await writeFile(output, JSON.stringify({grant_id:granted.grant_id,interaction_id:question.id,ungranted_refused:refused,second_answer_refused:secondRefused})+'\n')
    console.log('AMASK_FOLLOWER_ANSWERED')
  } finally { await client.close() }
  process.exit(0)
}

const evidence = required('AMASK_EVIDENCE'), state = required('AMASK_STATE_ROOT')
for (const dir of [evidence,state]) { assert(path.isAbsolute(dir) && !dir.startsWith(tree+'/')); await mkdir(dir,{recursive:true,mode:0o700}) }
const root = await mkdtemp(path.join(state,'run-')), bin = required('AMASK_BIN')
const profilePath = required('AMASK_CODEX_PROFILE')
const workspace = await mkdtemp('/root/work/agent-amask-drill-')
execFileSync('git',['init','-q',workspace])
await writeFile(path.join(workspace,'README.md'),'Disposable MP-08 / MP-09 / MP-10 / MP-11 async-question drill.\n')
const children = [], kernels = [], clients = [], tuis = [], secrets = []
const report = {MP:['MP-08','MP-09','MP-10','MP-11'],head:execFileSync('git',['rev-parse','HEAD'],{cwd:tree,encoding:'utf8'}).trim(),started:new Date().toISOString(),model:'gpt-6-luna',effort:'low',cells:[],processes:[]}
let browser
function safe(text) { for(const secret of secrets) assert(!text.includes(secret),'Private value in evidence'); return text }
async function record(name, data) { await writeFile(path.join(evidence,name+'.json'),safe(JSON.stringify(data,null,2))+'\n',{mode:0o600}) }
async function port() { return new Promise((resolve,reject)=>{const server=net.createServer();server.once('error',reject);server.listen(0,'127.0.0.1',()=>{const p=server.address().port;server.close(()=>resolve(p))})}) }
function start(binary, env, name) {
  const child=spawnOwned(binary,[],{cwd:tree,env,stdio:['ignore','pipe','pipe'],detached:true})
  const log=createWriteStream(path.join(root,name+'.private.log'),{mode:0o600})
  child.stdout.pipe(log);child.stderr.pipe(log);children.push(child)
  report.processes.push({name,pid:child.pid})
  return child
}
async function stop(child) {
  if(!child || child.exitCode!==null || child.signalCode!==null)return
  signalOwnedProcess(child,'SIGINT')
  try { await until('owned process exit',async()=>child.exitCode!==null||child.signalCode!==null,Boolean,20_000) }
  catch {
    signalOwnedProcess(child,'SIGTERM')
    try { await until('owned process termination',async()=>child.exitCode!==null||child.signalCode!==null,Boolean,5000) }
    catch { signalOwnedProcess(child,'SIGKILL'); await until('owned process killed',async()=>child.exitCode!==null||child.signalCode!==null,Boolean,5000) }
  }
}
async function capture(tui,name) {
  await record(name,publicSnapshot(await tui.send('snapshot')))
  await writeFile(path.join(evidence,name+'.ansi'),safe(tui.output()),{mode:0o600})
  execFileSync('python3',[path.join(tree,'scripts/e2e-stack/context-switch/render-pty.py'),path.join(evidence,name+'.ansi')],{stdio:'ignore'})
}
async function ensureVault(k) {
  let status=variant(await k.client.send({GetCredentialVaultStatus:null}),'CredentialVaultStatus').status
  if(status.unlocked)return
  await k.tui.type('\x15/credential vault manage\r')
  await until('disposable Vault unlocked',async()=>{
    const s=await k.tui.send('snapshot')
    const secret=(s.interactions??[]).find(i=>i.customChoice?.inputKind==='secret'||i.customChoice?.input_kind==='secret')
    if(secret){await k.tui.send('interaction_custom_reply',{interactionId:secret.id,reply:k.passphrase,editing:true});await k.tui.send('interaction_submit',{choiceIndex:secret.choices.length})}
    status=variant(await k.client.send({GetCredentialVaultStatus:null}),'CredentialVaultStatus').status
    return status.unlocked
  })
}
async function setup(name,relayUrl,relaySecret) {
  const home=path.join(root,name)
  for(const sub of ['home','os-home','tmp','config','data','cache','state'])await mkdir(path.join(home,sub),{recursive:true,mode:0o700})
  const passphrase=randomBytes(24).toString('base64url');secrets.push(passphrase)
  await writeFile(path.join(home,'home/config.toml'),`version = 1\n[history.archive]\nmode = "disabled"\n[credential_vault]\nbackend = "chariox_encrypted"\npath = "${home}/home/private-vault.json"\nunlock_policy = "kernel_init"\nagent_management = "allow"\n`,{mode:0o600})
  const ports=await Promise.all(Array.from({length:4},port))
  const env={PATH:process.env.PATH,LANG:'C.UTF-8',HOME:home+'/os-home',CHARIOX_HOME:home+'/home',TMPDIR:home+'/tmp',XDG_CONFIG_HOME:home+'/config',XDG_STATE_HOME:home+'/state',XDG_DATA_HOME:home+'/data',XDG_CACHE_HOME:home+'/cache',CHARIOX_KERNEL_HOST:'127.0.0.1',CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_CODEX_PORT:String(ports[2]),CHARIOX_OPENCODE_PORT:String(ports[3]),CHARIOX_DAEMON_SOCKET:home+'/kernel.sock',CHARIOX_ROOM_AGENT_TOOLS:'1',CHARIOX_RELAY_URL:relayUrl,CHARIOX_RELAY_TOKEN:relaySecret,NODE_OPTIONS:'--max-old-space-size=4096',RUST_MIN_STACK:'16777216'}
  const k={home,env,passphrase,url:`ws://127.0.0.1:${ports[0]}/kernel`,name}
  k.child=start(bin+'/chariox-kernel',env,name)
  k.client=new LocalIpcClient(k.url,{localAuthEnvironment:env});clients.push(k.client)
  await until(name+' ready',()=>k.client.send({GetDaemonHealth:null}).catch(()=>null))
  k.passkeys=[];k.client.onKernelEvent(e=>{if(e.event==='passkey_prompts_changed')k.passkeys=e.prompts})
  await k.client.subscribeToWaitingRoomInventory()
  const linked=variant(await k.client.send({LinkProviderAccountProfile:{provider:'codex',label:'amask-'+name,path:profilePath}}),'ProviderAccountProfile').profile
  k.profile=linked.profile_id
  await until(name+' relay registered',()=>k.client.send({RelayStatus:null}),r=>r.RelayStatus?.status?.connected)
  const pairing=variant(await k.client.send({CreateTerminalPairingLink:{terminal_type:'cli',alias:'amask-'+name,expires_in_ms:60_000}}),'TerminalPairingLinkCreated').pairing
  secrets.push(pairing.pairing_link)
  const hostedOwner=name==='owner'&&process.env.AMASK_WEB_ORIGIN
  const tui=await openTui({tree:workspace,env:{...env,AMASK_RELAY_TOKEN:relaySecret},url:k.url,profile:k.profile,root:home,provider:'codex',model:'gpt-6-luna',binary:bin+'/chariox-cli',clientId:hostedOwner?'amask-owner-tui':pairing.terminal_id,
    connectionArgs:hostedOwner?undefined:['--terminal-pairing-link',pairing.pairing_link,'--relay-token-env','AMASK_RELAY_TOKEN']})
  tuis.push(tui);k.tui=tui
  await until(name+' attached',async()=>tui.send('snapshot'),s=>s.session?.agentCount>0&&s.attachmentId)
  await tui.send('switch_screen',{screen:'agents'})
  await tui.type('\x15/permissions yolo\r')
  await sleep(500)
  let s=await tui.send('snapshot');k.room=s.session.id;k.agent=s.session.focusedAgentId
  await tui.type('\x15/agent variant '+k.agent+' low\r')
  await ensureVault(k)
  k.session=async()=>variant(await k.client.send({GetSessionState:{session_id:k.room}}),'SessionState').session
  kernels.push(k)
  return k
}

try {
  const relayPort=await port(),relaySecret=randomBytes(32).toString('base64url');secrets.push(relaySecret)
  const relayUrl=`ws://127.0.0.1:${relayPort}`
  start(bin+'/chariox-relay',{PATH:process.env.PATH,CHARIOX_RELAY_HOST:'127.0.0.1',CHARIOX_RELAY_PORT:String(relayPort),CHARIOX_RELAY_TOKEN:relaySecret},'relay')
  const owner=await setup('owner',relayUrl,relaySecret)
  const follower=await setup('follower',relayUrl,relaySecret)
  for(const name of ['chariox-kernel','chariox-cli','chariox-relay'])report[name+'_sha256']=createHash('sha256').update(await readFile(bin+'/'+name)).digest('hex')
  await owner.tui.send('submit_prompt',{prompt:'Use chariox.request_popup with wait:false, title "AMASK primary", message "Choose the next step", one choice {id:"continue",label:"Continue",reply:"AMASK exact outside answer"}, timeout_sec:600. Then call chariox.events.inbox with this turn task_id and origin_prompt_id as a second tool while the question is pending. Finish this turn with AMASK_KEPT_WORKING. Do not block or yield. In a subsequent inbox turn, handle the answer, print AMASK_ANSWER_RECEIVED with the exact reply and attributed answerer, then acknowledge it as handled.'})
  const pending=await until('primary async question',owner.session,s=>(s.active_interactions??[]).some(q=>q.title==='AMASK primary'))
  const question=pending.active_interactions.find(q=>q.title==='AMASK primary')
  assert(question.asking_task_id)
  await until('asker finishes while question pending',owner.session,s=>s.agent_tasks?.some(t=>t.task_id===question.asking_task_id&&t.state==='waiting'))
  await until('continued work final answer',async()=>owner.tui.send('snapshot'),s=>assistantText(s).includes('AMASK_KEPT_WORKING'))
  await capture(owner.tui,'01-asking-turn-complete')
  report.cells.push({id:'AM-ASK-01',result:'CORE PASS',interaction_id:question.id,asking_task_id:question.asking_task_id})
  if(process.env.AMASK_WEB_CAPTURE_HELPER){
    // The helper drives the production hosted entry and captures both DPRs.
    execFileSync(process.env.AMASK_WEB_CAPTURE_HELPER,[owner.room,question.id,evidence],{env:{...process.env,AMASK_OWNER_ENDPOINT:owner.url,AMASK_OWNER_CHARIOX_HOME:owner.env.CHARIOX_HOME},stdio:'inherit',timeout:180_000})
  }
  if(process.env.AMASK_WEB_ORIGIN){delete owner.env.CHARIOX_RELAY_URL;delete owner.env.CHARIOX_RELAY_TOKEN}
  await stop(owner.child)
  owner.child=start(bin+'/chariox-kernel',owner.env,'owner-restarted')
  const restarted=await until('restarted question',owner.session,s=>s.active_interactions?.some(q=>q.id===question.id))
  const restored=restarted.active_interactions.find(q=>q.id===question.id)
  assert.equal(restored.requested_at_ms,question.requested_at_ms)
  assert.equal(restored.timeout_sec,question.timeout_sec)
  await ensureVault(owner)
  await capture(owner.tui,'02-restarted-question')
  const followerResult=path.join(root,'follower-result.json')
  const followerCommand=['node',path.join(tree,'apps/cli/scripts/live-async-question-drill.mjs'),'--follower',owner.env.CHARIOX_DAEMON_SOCKET,owner.room,followerResult].map(s=>"'"+s.replaceAll("'","'\\''")+"'").join(' ')
  await follower.tui.send('submit_prompt',{prompt:`Run this exact command using your shell tool and wait for it: ${followerCommand}. It requests access through the normal owner approval; do not read the owner token or state. When it succeeds finish with AMASK_FOLLOWER_DONE.`})
  const access=await until('outside grant decision',async()=>owner.passkeys.find(p=>p.kind==='access_grant'))
  assert(access.approve_choice_id)
  await owner.client.send({RespondToInteraction:{session_id:access.session_id,interaction_id:access.interaction_id,choice_id:access.approve_choice_id,passkey:owner.passphrase}})
  const result=await until('outside answer receipt',()=>readFile(followerResult,'utf8').then(JSON.parse).catch(()=>null))
  await record('03-outside-follower',result)
  await until('answer turn',async()=>owner.tui.send('snapshot'),s=>assistantText(s).includes('AMASK_ANSWER_RECEIVED')&&assistantText(s).includes('AMASK exact outside answer'),120_000)
  const answered=await owner.session()
  const wake=answered.agent_wakes.find(w=>w.id===question.id)
  assert.equal(wake.fire_count,1)
  assert(!answered.active_interactions.some(q=>q.id===question.id))
  await capture(owner.tui,'04-answer-wake')
  report.cells.push({id:'AM-ASK-02',result:'CORE PASS',grant_id:result.grant_id,duplicate_refused:true,ungranted_refused:true})
  report.cells.push({id:'AM-ASK-03',result:'CORE PASS',fire_count:wake.fire_count,restarted:true})
  await owner.tui.send('submit_prompt',{prompt:'Call chariox.request_popup twice, both wait:false and timeout_sec:3. First title "AMASK deadline default", message "Default deadline", choices [{id:"continue",label:"Continue",reply:"AMASK timeout default"}], default_on_timeout:"continue". Second title "AMASK deadline empty", message "No default deadline", same choices but omit default_on_timeout. Call events.inbox as another tool, then finish with AMASK_DEADLINES_ARMED. When woken, read events.inbox and handle both timed_out answers, report AMASK_DEADLINES_RECEIVED with their status/choice/reply, and acknowledge the handled events.'})
  await until('deadline receipts',owner.session,s=>s.agent_wakes?.filter(w=>w.kind==='interaction'&&w.id!==question.id&&w.fire_count===1).length>=2,120_000)
  await until('deadline answer turn',async()=>owner.tui.send('snapshot'),s=>assistantText(s).includes('AMASK_DEADLINES_RECEIVED'),120_000)
  await capture(owner.tui,'05-deadline-wake')
  report.cells.push({id:'AM-ASK-04',result:'CORE PASS'})
  await owner.tui.send('submit_prompt',{prompt:'Call chariox.request_popup with wait:false, title "AMASK stop", message "Stop withdraws this question", choices [{id:"continue",label:"Continue",reply:"AMASK must not default"}], timeout_sec:300, default_on_timeout:"continue". Then run sleep 60 with your shell tool and wait for it. This is a stop drill; do not answer the question yourself.'})
  await until('stop question pending',owner.session,s=>s.active_interactions?.some(q=>q.title==='AMASK stop'))
  await capture(owner.tui,'06-before-stop')
  await owner.tui.type('\x03')
  await until('stop withdraws card',owner.session,s=>!s.active_interactions?.some(q=>q.title==='AMASK stop'))
  await capture(owner.tui,'07-after-stop')
  report.cells.push({id:'AM-ASK-05',result:'CORE PASS',stopped_via:'real TUI Ctrl+C'})
  report.web=process.env.AMASK_WEB_CAPTURE_HELPER?'captured by hosted helper':'NOT RUN (run the hosted web helper at DPR1 and DPR2)'
} catch(error) {
  report.failure=String(error.message)
  process.exitCode=1
} finally {
  for(const tui of tuis.reverse())await tui.close().catch(()=>{})
  for(const client of clients.reverse())await client.close().catch(()=>{})
  for(const child of children.reverse())await stop(child).catch(()=>{})
  const remaining=children.flatMap(child=>ownedProcessHandles(child)).filter(h=>h.pid>1)
  for(const handle of remaining){try{signalOwnedProcess(handle,'SIGTERM')}catch{}}
  await browser?.close().catch(()=>{})
  await record('report',report)
  await rm(root,{recursive:true,force:true})
  await rm(workspace,{recursive:true,force:true})
  console.log(JSON.stringify({MP:report.MP,cells:report.cells,failure:report.failure??null,web:report.web,cleanup:'owned runtime removed'}))
}

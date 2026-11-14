#!/usr/bin/env node
// Two real owner rooms, official Codex turns, and one shared kernel decision.
import assert from 'node:assert/strict'
import {randomBytes, createHash} from 'node:crypto'
import {mkdir, readFile, writeFile} from 'node:fs/promises'
import {createWriteStream} from 'node:fs'
import {execFileSync} from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import {fileURLToPath} from 'node:url'
import {LocalIpcClient} from '../../../packages/kernel-client/dist/ipc.js'
import {openTui, publicSnapshot} from '../../../scripts/e2e-stack/context-switch/tui.mjs'
import {spawnOwned, signalOwnedProcess} from '../../kernel/slice-linux-docker/owned-process-signals.mjs'

const tree = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const required = key => {assert(process.env[key], `Set ${key}`); return process.env[key]}
const root = required('XROOM_STATE'), evidence = required('XROOM_EVIDENCE'), bin = required('XROOM_BIN')
for (const dir of [root, evidence]) {assert(path.isAbsolute(dir) && !dir.startsWith(tree+'/')); await mkdir(dir, {recursive:true, mode:0o700})}
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const report = {head:execFileSync('git',['rev-parse','HEAD'],{cwd:tree,encoding:'utf8'}).trim(), started:new Date().toISOString(), cells:[], processes:[]}
const children = [], tuis = [], clients = [], privateValues = []
let interrupted = false
process.once('SIGINT',()=>{interrupted=true}); process.once('SIGTERM',()=>{interrupted=true})
const variant = (r,k) => {assert(r[k], `Expected ${k}, got ${Object.keys(r)}`); return r[k]}
const text = s => (s.transcript?.entries??[]).filter(e=>e.role==='assistant').map(e=>e.text).join('\n')
async function until(label, read, predicate=Boolean, timeout=180_000) {
  const end = Date.now()+timeout
  while(Date.now()<end) {assert(!interrupted, 'Drill interrupted'); const value=await read(); if(predicate(value)) return value; await sleep(300)}
  throw Error('Timed out: '+label)
}
async function record(name, value) {
  const body=JSON.stringify(value,null,2)
  for(const secret of privateValues) assert(!body.includes(secret), 'Private value in evidence')
  await writeFile(path.join(evidence,name+'.json'),body+'\n',{mode:0o600})
}
async function port() {return new Promise((resolve,reject)=>{const s=net.createServer();s.once('error',reject);s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>resolve(p))})})}
function start(binary, env, name, args=[]) {
  const child=spawnOwned(binary,args,{cwd:tree,env,stdio:['ignore','pipe','pipe'],detached:true})
  const log=createWriteStream(path.join(root,name+'.private.log'),{mode:0o600})
  child.stdout.pipe(log);child.stderr.pipe(log);children.push(child);report.processes.push({name,pid:child.pid});return child
}
async function stop(child) {
  for(const [signal,ms] of [['SIGINT',15_000],['SIGTERM',5000],['SIGKILL',5000]]) {
    if(child.exitCode!==null||child.signalCode!==null)return
    signalOwnedProcess(child,signal)
    const end=Date.now()+ms;while(child.exitCode===null&&child.signalCode===null&&Date.now()<end)await sleep(100)
  }
  assert(child.exitCode!==null||child.signalCode!==null,'Owned process did not stop')
}
try {
  for(const name of ['chariox-kernel','chariox-relay'])report[name+'_sha256']=createHash('sha256').update(await readFile(path.join(bin,name))).digest('hex')
  const home=path.join(root,'home'), workspace=required('XROOM_WORKSPACE')
  assert(path.isAbsolute(workspace) && !workspace.startsWith('/root/.chariox/'))
  await mkdir(workspace,{mode:0o700})
  for(const dir of ['home','os-home','tmp','config','data','cache','state'])await mkdir(path.join(root,dir),{recursive:true,mode:0o700})
  execFileSync('git',['init','-q',workspace]);await writeFile(path.join(workspace,'README.md'),'Disposable cross-room decision drill.\n')
  const passphrase=randomBytes(24).toString('base64url');privateValues.push(passphrase)
  await writeFile(path.join(home,'config.toml'),`version = 1\n[history.archive]\nmode = "disabled"\n[credential_vault]\nbackend = "chariox_encrypted"\npath = "${home}/private-vault.json"\nunlock_policy = "kernel_init"\n`,{mode:0o600})
  const ports=await Promise.all(Array.from({length:5},port)), relaySecret=randomBytes(32).toString('base64url');privateValues.push(relaySecret)
  const relayUrl=`ws://127.0.0.1:${ports[4]}`
  start(path.join(bin,'chariox-relay'),{PATH:process.env.PATH,CHARIOX_RELAY_HOST:'127.0.0.1',CHARIOX_RELAY_PORT:String(ports[4]),CHARIOX_RELAY_TOKEN:relaySecret},'relay')
  const env={PATH:process.env.PATH,LANG:'C.UTF-8',HOME:path.join(root,'os-home'),CHARIOX_HOME:home,TMPDIR:process.env.TMPDIR??path.join(root,'tmp'),XDG_CONFIG_HOME:path.join(root,'config'),XDG_DATA_HOME:path.join(root,'data'),XDG_CACHE_HOME:path.join(root,'cache'),XDG_STATE_HOME:path.join(root,'state'),CHARIOX_KERNEL_HOST:'127.0.0.1',CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_CODEX_PORT:String(ports[2]),CHARIOX_OPENCODE_PORT:String(ports[3]),CHARIOX_DAEMON_SOCKET:path.join(root,'kernel.sock'),CHARIOX_ROOM_AGENT_TOOLS:'1',CHARIOX_RELAY_URL:relayUrl,CHARIOX_RELAY_TOKEN:relaySecret,RUST_MIN_STACK:'16777216',NODE_OPTIONS:'--max-old-space-size=4096'}
  if(process.env.XROOM_ENROLL_HELPER) {
    delete env.CHARIOX_RELAY_URL;delete env.CHARIOX_RELAY_TOKEN
    execFileSync(process.env.XROOM_ENROLL_HELPER,['enroll'],{env:{...process.env,...env,XROOM_KERNEL_BINARY:path.join(bin,'chariox-kernel')},stdio:['ignore','pipe','pipe'],timeout:180_000})
  }
  start(path.join(bin,'chariox-kernel'),env,'kernel')
  const url=`ws://127.0.0.1:${ports[0]}/kernel`, client=new LocalIpcClient(url,{localAuthEnvironment:env});clients.push(client)
  await until('kernel ready',()=>client.send({GetDaemonHealth:null}).catch(()=>null))
  await until('relay connected',()=>client.send({RelayStatus:null}),r=>r.RelayStatus?.status?.connected)
  await client.send({CreateCredentialVault:{origin:{kind:'passphrase',passphrase}}})
  await client.send({UnlockCredentialVault:{passphrase,duration:{kind:'until_kernel_stops'},surface:'settings'}})
  const profile=variant(await client.send({LinkProviderAccountProfile:{provider:'codex',label:'xroom-drill',path:required('XROOM_CODEX_PROFILE')}}),'ProviderAccountProfile').profile.profile_id
  await client.send({RefreshProviderAccountProfile:{provider:'codex',account_profile:profile}})
  const rooms=[]
  for(const name of ['a','b']) {
    const dir=path.join(root,name);await mkdir(dir,{recursive:true,mode:0o700})
    const pairing=variant(await client.send({CreateTerminalPairingLink:{terminal_type:'cli',alias:'xroom-'+name,expires_in_ms:60_000}}),'TerminalPairingLinkCreated').pairing
    privateValues.push(pairing.pairing_link)
    const tui=await openTui({tree:workspace,env:{...env,XROOM_RELAY_TOKEN:relaySecret},url,profile,root:dir,provider:'codex',model:'gpt-6-luna',binary:path.join(tree,'apps/cli/dist/index.js'),prefix:[required('XROOM_BUN')],clientId:pairing.terminal_id,connectionArgs:process.env.XROOM_ENROLL_HELPER?undefined:['--terminal-pairing-link',pairing.pairing_link,'--relay-token-env','XROOM_RELAY_TOKEN']})
    tuis.push(tui)
    const s=await until('room '+name,()=>tui.send('snapshot'),s=>s.session?.agentCount>0&&s.attachmentId)
    const room={tui,id:s.session.id,agent:s.session.focusedAgentId}
    await tui.type('\x15/permissions yolo\r');await tui.type('\x15/agent variant '+room.agent+' low\r')
    const resolved=(await tui.send('snapshot')).session.agents.find(a=>a.id===room.agent)
    assert.equal(resolved.model,'gpt-6-luna');assert.equal(resolved.effort,'low')
    room.session=async()=>variant(await client.send({GetSessionState:{session_id:room.id}}),'SessionState').session
    rooms.push(room)
  }
  const [a,b]=rooms;assert.notEqual(a.id,b.id);report.rooms=rooms.map(r=>({room:r.id,agent:r.agent,provider:'codex',model:'gpt-6-luna',effort:'low'}))
  await b.tui.send('submit_prompt',{prompt:'Call chariox.request_popup with wait:false, title "XROOM grant", message "Allow the disposable cross-room drill to proceed?", choices [{id:"allow",label:"Allow",reply:"XROOM_GRANTED"},{id:"deny",label:"Deny",reply:"XROOM_DENIED"}], timeout_sec:600. Finish with XROOM_CARD_ARMED. When its answer arrives in a later inbox turn, read the event, print XROOM_ANSWER_RECEIVED and its exact reply, and acknowledge it handled. Do not answer your own card.'})
  const pending=await until('grant card',b.session,s=>s.active_interactions?.some(i=>i.title==='XROOM grant'))
  const card=pending.active_interactions.find(i=>i.title==='XROOM grant');report.interaction_id=card.id
  await until('asking task waiting',b.session,s=>s.agent_tasks?.some(t=>t.task_id===card.asking_task_id&&t.state==='waiting'))
  await record('01-card-visible-on-asking-client',publicSnapshot(await b.tui.send('snapshot')))
  if(process.env.XROOM_ENROLL_HELPER) {
    start(process.env.XROOM_ENROLL_HELPER,{...process.env,...env,XROOM_KERNEL_URL:url},'web',['observe',b.id,card.id])
    await until('web clients see pending card',()=>readFile(path.join(evidence,'web-ready.json'),'utf8').then(JSON.parse).catch(()=>null))
  }
  await a.tui.send('submit_prompt',{prompt:`Use chariox.room.run_command for these commands in order: "room agents ${b.id}", "room cards ${b.id}", "agent answer ${card.id} allow --room ${b.id}". Print each complete public result as JSON. Then repeat the answer and verify it is refused because the card is settled. Finish with XROOM_COORDINATOR_DONE. Use the room tools directly; do not use shell, sudo or kernel_request.`})
  const done=await until('coordinator answers',()=>a.tui.send('snapshot'),s=>text(s).includes('XROOM_COORDINATOR_DONE'),300_000)
  await record('02-coordinator',publicSnapshot(done))
  assert(text(done).includes(b.agent),'Coordinator did not report other agent')
  assert(text(done).includes('answered'),'Coordinator did not report a successful answer')
  await until('shared card disappears',b.session,s=>!s.active_interactions?.some(i=>i.id===card.id))
  const awakened=await until('asking agent wakes',()=>b.tui.send('snapshot'),s=>text(s).includes('XROOM_ANSWER_RECEIVED')&&text(s).includes('XROOM_GRANTED'),300_000)
  await record('03-asking-agent-awake',publicSnapshot(awakened))
  const ledgerScript = `import sqlite3,json,sys
c=sqlite3.connect("file:"+sys.argv[1]+"?mode=ro",uri=True)
print(json.dumps({"answers":[json.loads(r[0])["payload"]["public_answer"] for r in c.execute("SELECT payload FROM agent_inbox WHERE source_id=? ORDER BY sequence",(sys.argv[2],))],"audit":[json.loads(r[0]) for r in c.execute("SELECT payload_json FROM durable_state_events WHERE kind=? AND subject_id=?",("interaction.answered",sys.argv[2]))]}))`
  const receipts=JSON.parse(execFileSync('python3',['-c',ledgerScript,path.join(home,'state/kernel.db'),card.id],{encoding:'utf8'}))
  assert.equal(receipts.answers.length,1);assert.equal(receipts.answers[0].reply,'XROOM_GRANTED');assert.equal(receipts.answers[0].agent_id,a.agent)
  assert.equal(receipts.audit.length,1);assert.equal(receipts.audit[0].answered_by_agent,a.agent);assert.equal(receipts.audit[0].answering_room_id,a.id)
  await record('03-authoritative-receipts',receipts)
  if(process.env.XROOM_ENROLL_HELPER) {
    const web=await until('web clients see resolution',()=>readFile(path.join(evidence,'web-report.json'),'utf8').then(JSON.parse).catch(()=>null))
    assert.equal(web.result,'PASS');report.web=web
  }
  report.cells.push({id:'XROOM-01',status:'PASS'},{id:'XROOM-02',status:'PASS'},{id:'XROOM-03',status:'PASS',first_answer_wins:true,asking_agent_woken:true})
  await client.send({LockCredentialVault:null})
  await b.tui.send('submit_prompt',{prompt:'Call chariox.manage_credential_vault with action popup exactly once to request the disposable Vault unlock card. Wait for its owner to cancel it. Do not enter a passphrase or answer the card. After cancellation, finish with XROOM_VAULT_CANCELLED.'})
  const secretState=await until('Vault secret card',b.session,s=>s.active_interactions?.some(i=>i.id.startsWith('vault-unlock-')))
  const secret=secretState.active_interactions.find(i=>i.id.startsWith('vault-unlock-'))
  assert.equal(secret.custom_choice?.input_kind,'secret')
  await a.tui.send('submit_prompt',{prompt:`Run "room cards ${b.id}" through chariox.room.run_command. Then run "agent answer ${secret.id} cancel --room ${b.id}" through the same tool. Print both public results in full and verify the answer is refused with owner-only. Do not enter a secret or unlock the Vault. Finish with XROOM_SECRET_REFUSED.`})
  const refused=await until('secret refused',()=>a.tui.send('snapshot'),s=>text(s).includes('XROOM_SECRET_REFUSED'),300_000)
  assert(text(refused).includes('owner-only'));assert((await b.session()).active_interactions.some(i=>i.id===secret.id))
  await record('04-secret-owner-only',publicSnapshot(refused))
  const cancelIndex=secret.choices.findIndex(choice=>choice.id==='cancel');assert(cancelIndex>=0)
  await b.tui.send('interaction_submit',{choiceIndex:cancelIndex})
  await until('owner terminal cancels the secret card',b.session,s=>!s.active_interactions?.some(i=>i.id===secret.id))
  await until('Vault requester returns after cancellation',()=>b.tui.send('snapshot'),s=>text(s).includes('XROOM_VAULT_CANCELLED'))
  report.cells.push({id:'XROOM-04',status:'PASS'},{id:'XROOM-05',status:'NOT RUN',note:'No second authenticated account in this isolated stack; guest refusal covered through the authenticated room-command regression test.'})
  report.result='PASS';report.finished=new Date().toISOString()
} catch(error) {report.result='FAIL';report.error=String(error?.stack??error);process.exitCode=1}
finally {
  for(const tui of tuis.reverse())await tui.close().catch(()=>{})
  for(const client of clients)await client.close().catch(()=>{})
  for(const child of children.reverse())await stop(child)
  report.cleanup='owned clients and processes stopped'
  await record('report',report)
  console.log(JSON.stringify({result:report.result,cells:report.cells,error:report.error}))
}

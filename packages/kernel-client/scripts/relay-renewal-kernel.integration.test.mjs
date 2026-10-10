import assert from 'node:assert/strict'
import {createHash,createHmac} from 'node:crypto'
import {createServer} from 'node:http'
import net from 'node:net'
import {spawn} from 'node:child_process'
import {mkdtemp,mkdir,open,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import path from 'node:path'
import {setTimeout as sleep} from 'node:timers/promises'
import test from 'node:test'
import {WebSocketServer} from 'ws'
import {LocalIpcClient} from '../dist/ipc.js'
import {RelayClientIdentity,createRelayKeypair} from '../dist/relay-crypto.js'

// Opt-in native regression: never use an operator profile or provider login.
// Cloud/relay are fixtures; the initial and renewal issuers are real kernels.
const binary=process.env.CHARIOX_RENEWAL_TEST_KERNEL_BINARY
const hash=s=>createHash('sha256').update(s).digest('hex')
const freePort=async()=>{const s=net.createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p}
const listen=async server=>{await new Promise(r=>server.listen(0,'127.0.0.1',r));return `http://127.0.0.1:${server.address().port}`}
const wait=async fn=>{const until=Date.now()+20000;while(Date.now()<until){try{const v=await fn();if(v)return v}catch{}await sleep(100)}throw Error('native kernel did not become ready')}

for (const outcome of ['reachable','unreachable','revoked']) test(`real kernel account grant uses its ${outcome} issuing kernel for a machine-only target`, {skip:!binary,timeout:40000}, async()=>{
 const reachable=outcome!=='unreachable'
 const root=await mkdtemp(path.join(tmpdir(),'chariox-relay-renewal-kernels-'))
 const identity=new RelayClientIdentity(createRelayKeypair().privateKey)
 const daemon=new RelayClientIdentity(createRelayKeypair().privateKey)
 const relay=new WebSocketServer({host:'127.0.0.1',port:0})
 await new Promise(r=>relay.once('listening',r))
 const relayUrl=`ws://127.0.0.1:${relay.address().port}`
 const calls=[],processes=[],locals=[],handles=[]
 let subjectExpiry,remote,bridgeError,authorizations=0,renewals=0
 const cloud=createServer(async(req,res)=>{
  try {
   let body='';for await(const c of req)body+=c
   const input=body?JSON.parse(body):{}
   if(req.url!=='/relay/token'){res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify({targets:[],revokedTokens:[]}));return}
   if(input.subjectKind!=='client'){res.writeHead(200,{'content-type':'application/json'});res.end('{}');return}
   const account=input.sessionToken==='synthetic-account-session'
   if(account){assert.equal(input.machineCredential,undefined);assert.equal(input.subject,'paired-account-client')}
   else {assert.equal(input.machineCredential,'synthetic-target-machine-credential');assert.equal(input.allowUnpairedClientSubject,true);assert.equal(input.subject,`machine-client:${hash('managed-machine')}:${hash('paired-account-client')}`)}
   assert.equal(input.publicKeyThumbprint,identity.publicKeyThumbprint)
   if(outcome==='revoked'&&calls.length){calls.push({authority:'account',refused:true});res.writeHead(403,{'content-type':'application/json'});res.end(JSON.stringify({error:'identity_revoked',message:'synthetic client revoked'}));return}
   const exp=(Date.now()+(reachable?1500:5000))/1000
   const claims={sub:input.subject,client_id:input.clientId,subject_kind:'client',account_id:input.accountId,user_id:'fixture-owner',realm_id:input.realmId,session_id:input.sessionId,public_key_thumbprint:input.publicKeyThumbprint,allowed_actions:input.allowedActions??['client.connect','packet.route'],allowed_targets:input.allowedTargets,iat:Date.now()/1000,exp,...(input.machineId?{machine_id:input.machineId}:{})}
   const data=`${Buffer.from('{"alg":"HS256","typ":"JWT"}').toString('base64url')}.${Buffer.from(JSON.stringify(claims)).toString('base64url')}`
   const token=data+'.'+createHmac('sha256','synthetic-fixture-signing-secret').update(data).digest('base64url')
   calls.push({authority:account?'account':'machine',claims,ttlMs:input.ttlMs})
   if(account&&!subjectExpiry)subjectExpiry=exp*1000
   res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify({token,expiresAt:new Date(exp*1000).toISOString()}))
  } catch {bridgeError=Error('Cloud fixture scope/identity assertion failed');res.writeHead(403);res.end('{}')}
 })
 const apiUrl=await listen(cloud)
 const native=async(role)=>{
  const home=path.join(root,role);await mkdir(home,{mode:0o700})
  const port=await freePort(),mcp=await freePort()
  const profile={api_url:apiUrl,email:'fixture@chariox.invalid',account_id:'fixture-account',user_id:'fixture-owner',account_slug:'fixture',realm_id:'fixture-realm',relay_url:relayUrl,issuer_id:'fixture-issuer',machine_id:role==='target'?'managed-machine':'account-machine',...(role==='target'?{machine_credential:'synthetic-target-machine-credential'}:{cloud_session_token:'synthetic-account-session',client_id:'paired-account-client'})}
  const log=await open(path.join(home,'kernel.log'),'w',0o600);handles.push(log)
  const env={...process.env,HOME:home,CHARIOX_HOME:path.join(home,'chariox'),XDG_CONFIG_HOME:path.join(home,'config'),XDG_STATE_HOME:path.join(home,'state'),CHARIOX_KERNEL_PORT:String(port),CHARIOX_MCP_PORT:String(mcp),CHARIOX_CLOUD_RELAY_CONFIG_JSON:JSON.stringify(profile)}
  // Eliminate inherited runtime/provider settings; fixture profiles are synthetic.
  delete env.CHARIOX_RELAY_URL;delete env.CHARIOX_RELAY_TOKEN;delete env.CHARIOX_DAEMON_ID
  const p=spawn(binary,[],{cwd:tmpdir(),env,stdio:['ignore',log.fd,log.fd]});processes.push(p)
  const local=new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`,{localAuthEnvironment:env,kernelPingIntervalMs:60000});locals.push(local)
  await wait(async()=>{await local.send({GetDaemonHealth:null});return true})
  const status=(await local.send({RelayStatus:null})).RelayStatus.status
  return {local,id:status.daemon_id,environment:env,process:p}
 }
 try {
  const issuer=await native('issuer'),target=await native('target')
  const request={IssueCloudRelayClientToken:{target_daemon_alias:target.id,client_id:'paired-account-client',session_id:'fixture-session',public_key_thumbprint:identity.publicKeyThumbprint}}
  const issued=(await issuer.local.send(request)).CloudRelayClientTokenIssued
  assert.equal(calls.length,1);assert.equal(calls[0].authority,'account')
  assert.equal(calls[0].claims.sub,'paired-account-client');assert.equal(calls[0].claims.machine_id,undefined)
  if(process.env.CHARIOX_RENEWAL_SKIP_TTL_ASSERT!=='1')assert.equal(calls[0].ttlMs,1800000,'preserve the original 30-minute client TTL')
  if(!reachable){issuer.local.destroy();issuer.process.kill('SIGKILL');await new Promise(r=>issuer.process.once('exit',r))}
  relay.on('connection',socket=>{
   socket.on('message',async data=>{try{
    const frame=JSON.parse(String(data))
    if(frame.kind==='client_connect'){
     authorizations++;const admitted=JSON.parse(Buffer.from(frame.auth_token.split('.')[1],'base64url'));assert.equal(admitted.sub,'paired-account-client');assert.equal(admitted.machine_id,undefined);assert.equal(admitted.public_key_thumbprint,identity.publicKeyThumbprint)
     socket.send(JSON.stringify({kind:'client_connected',target:frame.target,daemon_public_key:daemon.publicKeyBase64}))
    }else if(frame.kind==='client_request'){
     const envelope=JSON.parse(daemon.decrypt(frame.encrypted_request,identity.publicKeyBase64))
     if(envelope.request.IssueCloudRelayClientToken){renewals++;throw Error('renewal must never use the managed target authority')}
     const response=await target.local.send(envelope.request)
     socket.send(JSON.stringify({kind:'client_response',request_id:frame.request_id,error:null,encrypted_response:daemon.encrypt(identity.publicKeyBase64,JSON.stringify(response))}))
    }else if(frame.kind==='client_subscribe')socket.send(JSON.stringify({kind:'client_response',request_id:frame.request_id,error:null,encrypted_response:daemon.encrypt(identity.publicKeyBase64,'null')}))
   }catch{bridgeError=Error('native relay bridge assertion failed');socket.close()}})
  })
  remote=new LocalIpcClient(relayUrl,{relayAuthToken:issued.token.relay_token,targetDaemonId:target.id,relayIdentity:identity,kernelPingIntervalMs:60000,controlRequestRetryDeadlineMs:0,localAuthEnvironment:issuer.environment,relayAuthorizationIssuer:{endpoint:issuer.local.socketPath,daemonId:issuer.id}})
  const closed=[],notices=[]
  remote.onKernelEvent(event=>{if(event.event==='transport_closed')closed.push(event.message);if(event.event==='runtime_notices')for(const n of event.notices)notices.push(n.message)})
  await remote.send({GetDaemonHealth:null});await remote.subscribeToKernelEvents('fixture-session','fixture-attachment')
  if(outcome==='revoked'){
   await sleep(1100)
   assert.equal(closed.length,1);assert.match(closed[0],/authorization.*refused|revoked/i)
   assert.equal(calls.length,2,'revocation is not retried or re-paired')
   assert.equal(authorizations,2,'a refused grant is never applied')
   await assert.rejects(remote.send({GetDaemonHealth:null}),/authorization.*refused|revoked/i)
  }else if(reachable){
   await sleep(5200)
   assert.deepEqual(closed,[],'keep admission across several original expiries')
   assert.equal(notices.length,0)
   assert(calls.length>=4,'original issuer must renew across multiple original lifetimes')
   assert(calls.every(c=>c.authority==='account'&&c.claims.sub==='paired-account-client'&&c.claims.machine_id===undefined))
   if(process.env.CHARIOX_RENEWAL_SKIP_TTL_ASSERT!=='1')assert(calls.every(c=>c.ttlMs===1800000))
   await remote.send({GetDaemonHealth:null})
   assert(authorizations>=8,'both original lanes receive each issuer grant')
  }else{
   await sleep(900)
   assert.deepEqual(closed,[],'an unreachable issuer must retain the original admission until expiry')
   assert.equal(notices.length,1);assert.match(notices[0],/issuing kernel.*unavailable.*valid until/i)
   await remote.send({GetDaemonHealth:null})
   assert.equal(authorizations,2)
   await sleep(Math.max(0,subjectExpiry-Date.now())+100)
   assert.equal(closed.length,1);assert.match(closed[0],/expired.*issuing kernel/i)
   await assert.rejects(remote.send({GetDaemonHealth:null}),/expired.*issuing kernel/i)
   assert.equal(calls.length,1,'unreachable issuer must never fall back to the machine authority')
  }
  assert.equal(renewals,0);assert.equal(bridgeError,undefined)
 } finally {
  remote?.destroy();for(const l of locals)l.destroy()
  for(const p of processes)if(p.exitCode===null){p.kill('SIGTERM');await Promise.race([new Promise(r=>p.once('exit',r)),sleep(2000).then(()=>p.kill('SIGKILL'))])}
  for(const h of handles)await h.close()
  for(const s of relay.clients)s.terminate();await new Promise(r=>relay.close(r));cloud.closeAllConnections();await new Promise(r=>cloud.close(r))
  // These homes were created empty and used only synthetic fixture credentials.
  // No provider ran, no live Cloud enrollment or durable signing key was created.
  assert(root.startsWith(path.join(tmpdir(),'chariox-relay-renewal-kernels-')))
  await rm(root,{recursive:true,force:true})
 }
})

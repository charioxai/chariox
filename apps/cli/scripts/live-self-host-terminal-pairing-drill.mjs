import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { randomBytes, createHmac } from "node:crypto"
import { mkdtemp, rm, stat, readdir } from "node:fs/promises"
import { createServer } from "node:net"
import { createServer as createHttpServer } from "node:http"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { LocalIpcClient } from "../../../packages/kernel-client/dist/ipc.js"
import { createCliRelayIdentityStore } from "../dist/cli-relay-identity-store.js"
import { joinKernelTerminalPairingLink } from "../dist/relay-api.js"

const repo = fileURLToPath(new URL("../../..", import.meta.url))
const root = await mkdtemp(join(tmpdir(), "chariox-self-host-pairing-"))
const children = [], clients = []
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
async function freePort() { const server = createServer(); await new Promise(resolve => server.listen(0,"127.0.0.1",resolve)); const port = server.address().port; await new Promise(resolve=>server.close(resolve)); return port }
async function until(check, description) { const deadline=Date.now()+20_000; while(Date.now()<deadline) { if(await check()) return; await sleep(100) } throw new Error(`Timeout: ${description}`) }
const env = Object.fromEntries(Object.entries(process.env).filter(([key])=>!key.startsWith("CHARIOX_")))
const kernelRoot = join(root,"kernel"), clientRoot=join(root,"client")
const relayPort=await freePort(), kernelPort=await freePort(), mcpPort=await freePort(), operatorToken=randomBytes(32).toString("base64url")
const scoped=process.argv.includes("--scoped"), issuerSecret=randomBytes(32).toString("base64url")
function scopedToken(subject,kind,actions) {
  const head=Buffer.from(JSON.stringify({alg:"HS256",typ:"JWT"})).toString("base64url")
  const claims=Buffer.from(JSON.stringify({iss:"fixture-operator",sub:subject,subject_kind:kind,realm_id:"fixture-realm",allowed_actions:actions,allowed_targets:null,iat:Math.floor(Date.now()/1000)-1,exp:Math.floor(Number.MAX_SAFE_INTEGER/1000),jti:randomBytes(16).toString("hex")})).toString("base64url")
  const input=`${head}.${claims}`;return `${input}.${createHmac("sha256",issuerSecret).update(input).digest("base64url")}`
}
const kernelToken=scoped?scopedToken("fixture-self-host","kernel",["daemon.register","daemon.heartbeat","peer.request","peer.event"]):operatorToken
const clientToken=scoped?scopedToken("operator-terminal","client",["client.metadata.read","client.connect","packet.route"]):operatorToken
const relayUrl=`ws://127.0.0.1:${relayPort}`, localUrl=`ws://127.0.0.1:${kernelPort}`
let cloudCalls=0
const cloudTrap=createHttpServer((_request,response)=>{cloudCalls++;response.writeHead(503).end()})
await new Promise(resolve=>cloudTrap.listen(0,"127.0.0.1",resolve))
function start(binary, options) {
  const child=spawn(join(repo,"target/debug",binary),[],{cwd:repo,env:{...env,...options},stdio:"ignore"}); children.push(child); return child
}
const kernelEnv={CHARIOX_HOME:kernelRoot,...(scoped?{CHARIOX_DAEMON_ID:"fixture-self-host"}:{}),CHARIOX_KERNEL_PORT:String(kernelPort),CHARIOX_MCP_PORT:String(mcpPort),CHARIOX_RELAY_URL:relayUrl,CHARIOX_RELAY_TOKEN:kernelToken,CHARIOX_CLOUD_RELAY_API_URL:`http://127.0.0.1:${cloudTrap.address().port}`}
try {
  start("chariox-relay",{CHARIOX_HOME:join(root,"relay"),CHARIOX_RELAY_PORT:String(relayPort),...(scoped?{CHARIOX_RELAY_SCOPED_ISSUER:"fixture-operator",CHARIOX_RELAY_SCOPED_HMAC_SECRET:issuerSecret}:{CHARIOX_RELAY_TOKEN:operatorToken})})
  start("chariox-kernel",kernelEnv)
  const local=new LocalIpcClient(localUrl,{localAuthEnvironment:kernelEnv,controlRequestRetryDeadlineMs:2_000,controlResponseStallMs:3_000}); clients.push(local)
  await until(async()=>{try { const status=await local.send({RelayStatus:null});return status.RelayStatus?.status?.connected } catch {return false}},"kernel live relay registration")
  console.log("status: disposable kernel registered on the operator relay")
  if (scoped) {
    for (const intent of ["client", "machine"]) {
      await assert.rejects(local.send({CreatePairingInvite:{intent}}), /cannot be exported/)
    }
    console.log("status: legacy invites cannot export the scoped kernel credential")
  }
  const created=await local.send({CreateTerminalPairingLink:{terminal_type:"cli",alias:null,expires_in_ms:60_000}})
  const link=created.TerminalPairingLinkCreated.pairing
  if(scoped) assert.equal(JSON.parse(Buffer.from(link.pairing_link.split(".")[1],"base64url")).relay_token,"operator-client-token-required")
  const identity=createCliRelayIdentityStore(join(clientRoot,"identity.json")).getOrCreate()
  const options={relayAuthToken:clientToken,targetDaemonId:link.target_daemon_id,relayIdentity:identity,kernelEventStaleMs:0}
  const terminal=new LocalIpcClient(relayUrl,options);clients.push(terminal)
  await assert.rejects(terminal.send({GetWaitingRoomPublicSnapshot:null}),/unpaired|revoked/)
  console.log("status: unpaired runtime request denied")
  const joined=await joinKernelTerminalPairingLink(terminal,link.pairing_link,link.terminal_id,identity.publicKeyThumbprint)
  assert.equal(joined.kernel_pairing,true); assert.equal(joined.relay_token,undefined)
  assert.equal(joined.pairing.target_daemon_id,link.target_daemon_id)
  console.log("status: kernel admitted the terminal key without Cloud")
  const profile=await terminal.send({GetWaitingRoomPublicSnapshot:null});assert.ok(profile.WaitingRoomPublicSnapshot)
  // Restart-like reconnect reuses the private key and its durable kernel grant.
  const reopened=new LocalIpcClient(relayUrl,{...options,relayIdentity:createCliRelayIdentityStore(join(clientRoot,"identity.json")).load()});clients.push(reopened)
  await reopened.send({GetWaitingRoomPublicSnapshot:null})
  const wrongIdentity=createCliRelayIdentityStore(join(root,"foreign/identity.json")).getOrCreate()
  const wrong=new LocalIpcClient(relayUrl,{...options,relayIdentity:wrongIdentity});clients.push(wrong)
  await assert.rejects(joinKernelTerminalPairingLink(wrong,link.pairing_link,link.terminal_id,wrongIdentity.publicKeyThumbprint))
  let events=0;terminal.onKernelEvent(()=>events++)
  await terminal.subscribeToWaitingRoomInventory()
  await until(()=>events>0,"encrypted waiting-room events")
  async function grantFiles(directory) {
    const entries=await readdir(directory,{withFileTypes:true}), paths=[]
    for(const entry of entries) {
      if(entry.isDirectory()) paths.push(...await grantFiles(join(directory,entry.name)))
      else if(entry.name==="terminal-grants.json") paths.push(join(directory,entry.name))
    }
    return paths
  }
  const grants=await grantFiles(kernelRoot);assert.equal(grants.length,1)
  const grantPath=grants[0]
  assert.equal((await stat(grantPath)).mode & 0o777,0o600)
  console.log("status: private grant, restart reuse and encrypted events verified")
  await local.send({RevokePairedClient:{client_id:link.terminal_id}})
  await assert.rejects(reopened.send({GetWaitingRoomPublicSnapshot:null}),/unpaired|revoked/)
  await assert.rejects(joinKernelTerminalPairingLink(terminal,link.pairing_link,link.terminal_id,identity.publicKeyThumbprint))
  await sleep(400);const stopped=events;await sleep(1_200);assert.equal(events,stopped)
  assert.equal(cloudCalls,0)
  console.log("PASS live operator relay discovery; encrypted Cloud-free kernel pairing; durable 0600 terminal-key grant; restart reuse; foreign-key takeover denied; revocation denies requests and stops active events; zero Cloud requests")
} finally {
  for(const client of clients) client.destroy()
  for(const child of children) if(child.exitCode===null) child.kill("SIGTERM")
  await Promise.all(children.map(async child=>{if(child.exitCode!==null)return;await Promise.race([new Promise(resolve=>child.once("exit",resolve)),sleep(3_000)]);if(child.exitCode===null)child.kill("SIGKILL")}))
  await new Promise(resolve=>cloudTrap.close(resolve))
  await rm(root,{recursive:true,force:true})
}

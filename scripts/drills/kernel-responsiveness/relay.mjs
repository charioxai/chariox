import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHmac, createHash, randomBytes, randomUUID } from 'node:crypto'
import { mkdirSync, openSync, writeFileSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'

export async function disconnectPeer({ binary, relayBinary, env, root, evidence, freePort, rpc, measure }) {
  const children = [], issuer = `responsiveness-${randomUUID()}`, secret = randomBytes(32).toString('base64url')
  const token = (subject, kind, actions) => {
    const head = Buffer.from(JSON.stringify({alg:'HS256',typ:'JWT'})).toString('base64url')
    const body = Buffer.from(JSON.stringify({iss:issuer,sub:subject,subject_kind:kind,realm_id:issuer,allowed_actions:actions,allowed_targets:null,iat:Math.floor(Date.now()/1000)-1,exp:Math.floor(Date.now()/1000)+600,jti:randomUUID(),user_id:'local'})).toString('base64url')
    return `${head}.${body}.${createHmac('sha256',secret).update(`${head}.${body}`).digest('base64url')}`
  }
  const start = (program, extra, name) => {
    const log = openSync(join(evidence,`${name}.log`),'a',0o600)
    const child = spawn(program, [], { env: {...env,...extra}, stdio:['ignore',log,log] })
    assert(Number.isInteger(child.pid) && child.pid > 1); children.push(child); return child
  }
  const until = async check => { for(let i=0;i<300;i++) { try {if(await check())return} catch{}; await delay(100) } throw Error('relay readiness timed out') }
  let client
  try {
    const relayRoot = join(root,'relay'), peerRoot = join(root,'peer')
    mkdirSync(relayRoot); mkdirSync(peerRoot)
    for(const sub of ['chariox','home','config','cache','state','tmp','history','work','logs']) mkdirSync(join(peerRoot,sub))
    writeFileSync(join(peerRoot,'chariox/config.toml'),`[credential_vault]\nbackend = "chariox_encrypted"\npath = "${peerRoot}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${peerRoot}/chariox/state.db"\n`,{mode:0o600})
    const relayPort = await freePort(), ports = await Promise.all(Array.from({length:4},freePort)), url = `ws://127.0.0.1:${relayPort}`
    start(relayBinary,{HOME:relayRoot,CHARIOX_HOME:relayRoot,CHARIOX_RELAY_HOST:'127.0.0.1',CHARIOX_RELAY_PORT:String(relayPort),CHARIOX_RELAY_SCOPED_ISSUER:issuer,CHARIOX_RELAY_SCOPED_HMAC_SECRET:secret},'relay')
    const peerEnv = {...env,HOME:join(peerRoot,'home'),CHARIOX_HOME:join(peerRoot,'chariox'),TMPDIR:join(peerRoot,'tmp'),XDG_CONFIG_HOME:join(peerRoot,'config'),XDG_CACHE_HOME:join(peerRoot,'cache'),XDG_STATE_HOME:join(peerRoot,'state'),CHARIOX_SESSION_HISTORY_DIR:join(peerRoot,'history'),CHARIOX_LOG_DIR:join(peerRoot,'logs'),CHARIOX_DAEMON_SOCKET:join(peerRoot,'daemon.sock'),CHARIOX_DAEMON_ALIAS:'responsiveness-peer',CHARIOX_MACHINE_ALIAS:'responsiveness-peer',CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_OPENCODE_PORT:String(ports[2]),CHARIOX_CODEX_PORT:String(ports[3]),CHARIOX_SYNC_STALL_ROOT:peerRoot}
    const peer = start(binary,peerEnv,'relay-peer')
    client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`,{localAuthEnvironment:peerEnv,controlRequestRetryDeadlineMs:1000})
    const peerRpc = async request => {const result=await client.send(request); if(result.Error)throw Error(result.Error.code);return result}
    await until(async()=>!!(await peerRpc({RelayStatus:null})).RelayStatus)
    const identities = [(await rpc({RelayStatus:null})).RelayStatus.status.daemon_id,(await peerRpc({RelayStatus:null})).RelayStatus.status.daemon_id]
    for(const [index,send] of [rpc,peerRpc].entries()) await send({ConfigureRelay:{relay_url:url,relay_token:token(identities[index],'kernel',['daemon.register','daemon.heartbeat','packet.route','peer.request','peer.event'])}})
    await until(async()=> (await rpc({RelayStatus:null})).RelayStatus.status.connected && (await peerRpc({RelayStatus:null})).RelayStatus.status.connected)
    const pairing=(await peerRpc({CreateTerminalPairingLink:{terminal_type:'cli',expires_in_ms:300000}})).TerminalPairingLinkCreated.pairing
    await rpc({JoinKernelLink:{link:pairing.pairing_link,relay_token:token('responsiveness-client','client',['client.connect','client.metadata.read','packet.route'])}})
    const before=await rpc({RelayStatus:null})
    assert(Number.isInteger(peer.pid) && peer.pid > 1); peer.kill('SIGKILL')
    const unrelated=await measure(30)
    const recovery=!!(await rpc({RelayStatus:null})).RelayStatus.status.connected
    return {relay_sha256:createHash('sha256').update(readFileSync(relayBinary)).digest('hex'),peer_pid:peer.pid,paired:true,relay_connected_before:before.RelayStatus.status.connected,relay_connected_after:recovery,unrelated}
  } finally {
    await client?.close()
    for(const child of children.reverse()) if(child.exitCode===null && child.signalCode===null) {
      assert(Number.isInteger(child.pid) && child.pid>1); child.kill('SIGTERM')
      await Promise.race([new Promise(resolve=>child.once('exit',resolve)),delay(3000)])
      if(child.exitCode===null && child.signalCode===null) {child.kill('SIGKILL');await new Promise(resolve=>child.once('exit',resolve))}
    }
  }
}

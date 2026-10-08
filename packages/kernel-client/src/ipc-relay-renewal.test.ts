import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"
import { RelayClientIdentity, createRelayKeypair } from "./relay-crypto.js"

// Signed-token verification belongs to the real relay tests. This fixture
// exercises the shared SDK's encrypted issuer calls and both retained lanes.
for (const outcome of ["renew", "revoked", "wrong-key", "transient", "target-lost", "reauth-denied", "legacy", "legacy-busy", "machine-only"] as const) {
  test(`short-lived relay authorization ${outcome} on the existing control/event sockets`, async () => {
    const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
    const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
    const claims = { sub:"terminal", client_id:"terminal", subject_kind:"client", realm_id:"realm", account_id:"account", user_id:"owner", public_key_thumbprint:identity.publicKeyThumbprint, allowed_actions:["client.connect","packet.route"], allowed_targets:["kernel"] }
    const hash = (value: string) => createHash("sha256").update(value).digest("hex")
    const machineSubject = `machine-client:${hash("managed-machine")}:${hash(claims.sub)}`
    const token = (extra = {}) => `synthetic.${Buffer.from(JSON.stringify({...claims,exp:(Date.now()+1500)/1000,...extra})).toString("base64url")}.signature`
    const server = new WebSocketServer({host:"127.0.0.1",port:0})
    await new Promise<void>(resolve=>server.once("listening",resolve))
    const address = server.address(); assert.ok(address && typeof address === "object")
    let connections=0, renewals=0, authorizations=0, events=0, targetLosses=0
    let offlineUntil=0
    const timers: ReturnType<typeof setInterval>[]=[]
    server.on("connection",socket=>{
      connections++
      let handshakes=0
      socket.on("message",data=>{
        const frame=JSON.parse(String(data))
        if(frame.kind==="client_connect") {
          authorizations++;handshakes++
          if(outcome==="target-lost" && handshakes>1 && targetLosses===0) {
            targetLosses++;offlineUntil=Date.now()+100
            socket.send(JSON.stringify({kind:"close",reason:"target daemon disconnected from relay"}));socket.close();return
          }
          if(Date.now()<offlineUntil) {
            socket.send(JSON.stringify({kind:"close",reason:"target daemon is not connected to relay"}));socket.close();return
          }
          if(outcome==="reauth-denied" && handshakes>1) {
            socket.send(JSON.stringify({kind:"close",reason:"relay token has been revoked"}));socket.close();return
          }
          socket.send(JSON.stringify({kind:"client_connected",target:frame.target,daemon_public_key:daemon.publicKeyBase64}))
        } else if(frame.kind==="client_request") {
          const envelope=JSON.parse(daemon.decrypt(frame.encrypted_request,identity.publicKeyBase64))
          if(outcome==="legacy-busy" && envelope.request.GetDaemonHealth!==undefined) return
          const renewal=envelope.request.IssueCloudRelayClientToken
          if(renewal) {
            renewals++
            assert.equal(renewal.client_id,"terminal")
            assert.equal(renewal.target_daemon_alias,"kernel")
            assert.equal(renewal.public_key_thumbprint,identity.publicKeyThumbprint)
          }
          const denied=renewal && (outcome==="revoked" || outcome==="transient" && renewals===1)
          const response=envelope.request.RelayStatus!==undefined ? {RelayStatus:{status:{capabilities:outcome.startsWith("legacy")?[]:["terminal_relay_authorization_renewal_v1"]}}} : renewal ? {CloudRelayClientTokenIssued:{profile:{},token:{relay_url:`ws://127.0.0.1:${address.port}`,relay_token:token(outcome==="wrong-key"?{public_key_thumbprint:"b".repeat(64)}:outcome==="machine-only"?{sub:machineSubject,client_id:machineSubject,machine_id:"managed-machine"}:{}),token_expires_at:new Date(Date.now()+1500).toISOString()}}} : {ok:true}
          socket.send(JSON.stringify({kind:"client_response",request_id:frame.request_id,error:denied?{code:outcome==="revoked"?"identity_revoked":"cloud_unavailable",message:"synthetic refusal",retryable:outcome==="transient"}:null,encrypted_response:denied?null:daemon.encrypt(identity.publicKeyBase64,JSON.stringify(response))}))
        } else if(frame.kind==="client_subscribe") {
          socket.send(JSON.stringify({kind:"client_response",request_id:frame.request_id,error:null,encrypted_response:daemon.encrypt(identity.publicKeyBase64,"null")}))
          let id=0
          timers.push(setInterval(()=>{if(socket.readyState===1)socket.send(JSON.stringify({kind:"client_event",subscription_id:frame.subscription_id,event_id:++id,encrypted_event:daemon.encrypt(identity.publicKeyBase64,JSON.stringify({event:"runtime_notices",notices:[]}))}))},40))
        }
      })
    })
    const client=new LocalIpcClient(`ws://127.0.0.1:${address.port}`,{relayAuthToken:token(),targetDaemonId:"kernel",relayIdentity:identity,kernelPingIntervalMs:60_000,controlRequestRetryDeadlineMs:0})
    const closed: string[]=[], notices: string[]=[]
    client.onKernelEvent(event=>{if(event.event==="transport_closed")closed.push(event.message);else {if(event.event==="runtime_notices") for(const notice of event.notices)if(typeof notice.message==="string")notices.push(notice.message);events++}})
    try {
      if(outcome==="legacy-busy") {
        await assert.rejects(client.send({GetDaemonHealth:null}),/protocol 472.*update the kernel/i)
        assert.equal(renewals,0);return
      }
      await client.send({GetDaemonHealth:null})
      if(outcome!=="legacy") await client.subscribeToKernelEvents("session","attachment")
      await sleep(outcome==="machine-only"?900:outcome==="renew" || outcome==="transient" || outcome==="target-lost"?4200:outcome==="legacy"?200:1800)
      if(outcome==="legacy") {
        assert.equal(renewals,0,"an unsupported kernel must never issue renewal for its login client")
        assert.equal(closed.length,1)
        assert.match(closed[0]!,/protocol 472.*update the kernel/i)
        await assert.rejects(client.send({GetDaemonHealth:null}),/protocol 472/i)
        return
      }
      if(outcome==="machine-only") {
        assert.equal(renewals,1)
        assert.deepEqual(closed,[],"an unavailable issuing authority must retain admission until expiry")
        assert.equal(notices.length,1)
        assert.match(notices[0]!,/machine-only.*account-issued.*valid until.*issuing kernel/i)
        assert.equal(authorizations,2,"the changed machine subject must never be applied to either lane")
        await client.send({GetDaemonHealth:null})
        await sleep(900)
        assert.equal(closed.length,1)
        assert.match(closed[0]!,/machine-only.*expired|expired.*machine-only/i)
        assert.equal(renewals,1,"unavailable authority is not retried or re-paired")
        return
      }
      assert.ok(renewals>0,"SDK must request fresh authorization before expiry")
      if(outcome==="renew" || outcome==="transient" || outcome==="target-lost") {
        assert.ok(renewals>=2,"several grant expiries must renew")
        if(outcome==="target-lost") { assert.equal(targetLosses,1);assert.ok(connections>=3,"temporary target loss reconnects") }
        else assert.equal(connections,2,"neither socket reconnects")
        assert.ok(authorizations>=6,"both lanes acknowledge the fresh grant")
        assert.ok(events>30,"subscription events continue")
        assert.equal(closed.some(message=>/authorization.*(?:revoked|refused)/i.test(message)),false)
        if(outcome!=="target-lost") assert.deepEqual(closed,[])
      } else {
        assert.equal(renewals,1,"refusal must not retry revoked authority")
        assert.equal(closed.length,1)
        assert.match(closed[0]!,/authorization.*(?:revoked|refused|invalid)/i)
        await assert.rejects(client.send({GetDaemonHealth:null}),/authorization/i)
        const stopped=events;await sleep(100);assert.equal(events,stopped)
      }
    } finally {
      client.destroy();for(const timer of timers)clearInterval(timer)
      for(const socket of server.clients)socket.terminate()
      await new Promise<void>(resolve=>server.close(()=>resolve()))
    }
  })
}

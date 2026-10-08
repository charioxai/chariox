import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"
import { RelayClientIdentity, createRelayKeypair } from "./relay-crypto.js"

// Signed-token verification belongs to the real relay tests. This fixture
// exercises the shared SDK's encrypted issuer calls and both retained lanes.
for (const outcome of ["renew", "revoked", "wrong-key", "transient"] as const) {
  test(`short-lived relay authorization ${outcome} on the existing control/event sockets`, async () => {
    const daemon = new RelayClientIdentity(createRelayKeypair().privateKey)
    const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
    const claims = { sub:"terminal", client_id:"terminal", subject_kind:"client", realm_id:"realm", account_id:"account", user_id:"owner", public_key_thumbprint:identity.publicKeyThumbprint, allowed_actions:["client.connect","packet.route"], allowed_targets:["kernel"] }
    const token = (extra = {}) => `synthetic.${Buffer.from(JSON.stringify({...claims,exp:(Date.now()+1500)/1000,...extra})).toString("base64url")}.signature`
    const server = new WebSocketServer({host:"127.0.0.1",port:0})
    await new Promise<void>(resolve=>server.once("listening",resolve))
    const address = server.address(); assert.ok(address && typeof address === "object")
    let connections=0, renewals=0, authorizations=0, events=0
    const timers: ReturnType<typeof setInterval>[]=[]
    server.on("connection",socket=>{
      connections++
      socket.on("message",data=>{
        const frame=JSON.parse(String(data))
        if(frame.kind==="client_connect") {
          authorizations++
          socket.send(JSON.stringify({kind:"client_connected",target:frame.target,daemon_public_key:daemon.publicKeyBase64}))
        } else if(frame.kind==="client_request") {
          const envelope=JSON.parse(daemon.decrypt(frame.encrypted_request,identity.publicKeyBase64))
          const renewal=envelope.request.IssueCloudRelayClientToken
          if(renewal) {
            renewals++
            assert.equal(renewal.client_id,"terminal")
            assert.equal(renewal.target_daemon_alias,"kernel")
            assert.equal(renewal.public_key_thumbprint,identity.publicKeyThumbprint)
          }
          const denied=renewal && (outcome==="revoked" || outcome==="transient" && renewals===1)
          const response=renewal ? {CloudRelayClientTokenIssued:{profile:{},token:{relay_url:`ws://127.0.0.1:${address.port}`,relay_token:token(outcome==="wrong-key"?{public_key_thumbprint:"b".repeat(64)}:{}),token_expires_at:new Date(Date.now()+1500).toISOString()}}} : {ok:true}
          socket.send(JSON.stringify({kind:"client_response",request_id:frame.request_id,error:denied?{code:outcome==="revoked"?"identity_revoked":"cloud_unavailable",message:"synthetic refusal",retryable:outcome==="transient"}:null,encrypted_response:denied?null:daemon.encrypt(identity.publicKeyBase64,JSON.stringify(response))}))
        } else if(frame.kind==="client_subscribe") {
          socket.send(JSON.stringify({kind:"client_response",request_id:frame.request_id,error:null,encrypted_response:daemon.encrypt(identity.publicKeyBase64,"null")}))
          let id=0
          timers.push(setInterval(()=>{if(socket.readyState===1)socket.send(JSON.stringify({kind:"client_event",subscription_id:frame.subscription_id,event_id:++id,encrypted_event:daemon.encrypt(identity.publicKeyBase64,JSON.stringify({event:"runtime_notices",notices:[]}))}))},40))
        }
      })
    })
    const client=new LocalIpcClient(`ws://127.0.0.1:${address.port}`,{relayAuthToken:token(),targetDaemonId:"kernel",relayIdentity:identity,kernelPingIntervalMs:60_000,controlRequestRetryDeadlineMs:0})
    const closed: string[]=[]
    client.onKernelEvent(event=>{if(event.event==="transport_closed")closed.push(event.message);else events++})
    try {
      await client.send({GetDaemonHealth:null})
      await client.subscribeToKernelEvents("session","attachment")
      await sleep(outcome==="renew" || outcome==="transient"?4200:1800)
      assert.ok(renewals>0,"SDK must request fresh authorization before expiry")
      if(outcome==="renew" || outcome==="transient") {
        assert.ok(renewals>=2,"several grant expiries must renew")
        assert.equal(connections,2,"neither socket reconnects")
        assert.ok(authorizations>=6,"both lanes acknowledge the fresh grant")
        assert.ok(events>30,"subscription events continue")
        assert.deepEqual(closed,[])
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

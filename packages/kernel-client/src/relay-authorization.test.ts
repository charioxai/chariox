import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"
import { LocalIpcError } from "./local-ipc-error.js"
import { setTimeout as sleep } from "node:timers/promises"
import { relayAuthorization, requireRenewedRelayAuthorization, RelayAuthorizationRenewal, isLocalRelayIssuerEndpoint } from "./relay-authorization.js"

const original={sub:"terminal",subject_kind:"client",realm_id:"realm",account_id:"account",user_id:"owner",client_id:"terminal",public_key_thumbprint:"a".repeat(64),exp:Date.now()/1000+10,allowed_actions:["client.connect","packet.route"],allowed_targets:["kernel","friendly"]}
const token=(claims:unknown)=>`synthetic.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.signature`
for(const [name,changes] of Object.entries({subject:{sub:"foreign"},account:{account_id:"foreign"},owner:{user_id:"foreign"},realm:{realm_id:"foreign"},machine:{machine_id:"foreign"},key:{public_key_thumbprint:"b".repeat(64)},scope:{allowed_targets:["kernel","another"]},target:{allowed_targets:["another"]},permissions:{allowed_actions:["client.connect"]},expired:{exp:0}})) {
  test(`renewal rejects ${name} changes`,()=>{
    assert.throws(()=>requireRenewedRelayAuthorization(original,token({...original,exp:original.exp+10,...changes}),"kernel"),/invalid identity, key or scope/)
  })
}
test("renewal may narrow an alias while retaining the connected exact target",()=>{
  assert.deepEqual(requireRenewedRelayAuthorization(original,token({...original,exp:original.exp+10,allowed_targets:["kernel"]}),"kernel").allowed_targets,["kernel"])
  assert.equal(relayAuthorization("opaque-operator-token"),null)
  assert.equal(relayAuthorization(token({...original,subject_kind:"kernel"})),null)
})
const hash=(value:string)=>createHash("sha256").update(value).digest("hex")
const machineSubject=`machine-client:${hash("managed-machine")}:${hash(original.sub)}`
for(const [name,changes] of Object.entries({subject:{sub:"machine-client:foreign"},owner:{user_id:"foreign"},account:{account_id:"foreign"},realm:{realm_id:"foreign"},key:{public_key_thumbprint:"b".repeat(64)},scope:{allowed_targets:["kernel","another"]},permissions:{allowed_actions:["client.connect"]}})) {
  test(`a machine replacement with invalid ${name} still refuses immediately`,()=>{
    assert.throws(()=>requireRenewedRelayAuthorization(original,token({...original,exp:original.exp+10,sub:machineSubject,client_id:machineSubject,machine_id:"managed-machine",...changes}),"kernel"),
      error=>error instanceof Error && "code" in error && error.code==="authorization_denied")
  })
}
test("machine-scoped grants still renew under their unchanged machine authority",()=>{
  const admitted={...original,sub:machineSubject,client_id:machineSubject,machine_id:"managed-machine"}
  assert.equal(requireRenewedRelayAuthorization(admitted,token({...admitted,exp:admitted.exp+10}),"kernel").sub,machineSubject)
})
test("a stalled issuer cannot extend admission beyond the current grant expiry",async()=>{
  let refused=0,release!:(value:number)=>void
  const renewal=new RelayAuthorizationRenewal(Date.now()+80,()=>new Promise(resolve=>{release=resolve}),()=>{refused++})
  try {
    await sleep(140)
    assert.equal(refused,1)
    release(Date.now()+60_000)
    await sleep(50)
    assert.equal(refused,1,"a late success cannot restart a retired renewal")
  } finally {renewal.stop()}
})
test("explicit shutdown retires an in-flight issuer response",async()=>{
  let refused=0,release!:(value:number)=>void
  const renewal=new RelayAuthorizationRenewal(Date.now()+100,()=>new Promise(resolve=>{release=resolve}),()=>{refused++})
  await sleep(30);renewal.stop();release(Date.now()+100)
  await sleep(140);assert.equal(refused,0)
})


test("issuer routes accept only local endpoints without credential-bearing URLs", () => {
  for (const endpoint of ["/private/kernel.sock", "ws+unix:///private/kernel.sock", "ws://127.0.0.1:49911/kernel", "ws://[::1]:49911/kernel"]) assert.equal(isLocalRelayIssuerEndpoint(endpoint), true)
  for (const endpoint of ["wss://hosted.example/kernel", "ws://remote.example/kernel", "ws://user:secret@127.0.0.1/kernel", "ws://127.0.0.1/kernel?token=secret", "ws://127.0.0.1/other"]) assert.equal(isLocalRelayIssuerEndpoint(endpoint), false)
})

test("an unavailable issuer retries and resumes across multiple original expiries", async () => {
  let attempts = 0, successes = 0, refused = 0
  const notices: string[] = []
  const expiry = Date.now() + 1500
  const renewal = new RelayAuthorizationRenewal(expiry, async () => {
    attempts++
    if (attempts <= 2) throw new LocalIpcError("renew", "Original issuing kernel unavailable", "relay_renewal_issuer_unavailable", true)
    successes++
    return Date.now() + 1500
  }, () => { refused++ }, message => { notices.push(message) })
  try {
    await sleep(4200)
    assert.equal(refused, 0)
    assert.equal(notices.length, 1)
    assert(successes >= 3, "admission survives several original lifetimes after recovery")
  } finally { renewal.stop() }
})

import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"
import { relayAuthorization, requireRenewedRelayAuthorization, RelayAuthorizationRenewal } from "./relay-authorization.js"

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

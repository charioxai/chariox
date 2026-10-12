// MP-08 / MP-10 / MP-11: admission must use the kernel-verified account.
import assert from "node:assert/strict"
import test from "node:test"
import {prepareHostedLinkedProviderAccount} from "./hosted-linked-provider-account.mjs"
const requests = {
  linkProviderAccountProfileRequest: (provider,label,path) => ({LinkProviderAccountProfile:{provider,label,path}}),
  refreshProviderAccountProfileRequest: (provider,account_profile) => ({RefreshProviderAccountProfile:{provider,account_profile}}),
}
function harness(authState="authenticated") {
  const calls=[]
  const client={send:async request=>{
    calls.push(request)
    return {ProviderAccountProfile:{profile:{provider:"codex",profile_id:"linked-scope",auth_state:request.RefreshProviderAccountProfile?authState:"unknown"}}}
  }}
  return {calls,client,requests,provider:"codex",label:"drill-account",path:"/product/linked/scope"}
}
test("MP-08 / MP-10 / MP-11 linked profile is refreshed before composer admission",async()=>{
  const h=harness();const p=await prepareHostedLinkedProviderAccount(h)
  assert.equal(p.auth_state,"authenticated")
  assert.deepEqual(h.calls,[{LinkProviderAccountProfile:{provider:"codex",label:"drill-account",path:"/product/linked/scope"}}, {RefreshProviderAccountProfile:{provider:"codex",account_profile:"linked-scope"}}])
})
test("MP-08 / MP-10 / MP-11 failed verification prevents provider launch",async()=>{
  await assert.rejects(prepareHostedLinkedProviderAccount(harness("expired")),/not authenticated/)
})

test("MP-08 / MP-10 / MP-11 verification cannot substitute another profile",async()=>{
  const h=harness();const send=h.client.send
  h.client.send=async request=>{
    const response=await send(request)
    if(request.RefreshProviderAccountProfile) response.ProviderAccountProfile.profile.profile_id="other-scope"
    return response
  }
  await assert.rejects(prepareHostedLinkedProviderAccount(h),/another account/)
})
test("MP-08 / MP-10 / MP-11 link failure does not refresh or launch",async()=>{
  const h=harness();h.client.send=async request=>{h.calls.push(request);return {Error:{message:"scope unavailable"}}}
  await assert.rejects(prepareHostedLinkedProviderAccount(h),/scope unavailable/)
  assert.equal(h.calls.length,1)
})

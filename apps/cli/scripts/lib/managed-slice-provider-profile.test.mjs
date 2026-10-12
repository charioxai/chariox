// MP-08/MP-10/MP-11: explicit product-linked account selection for Slice import.
import assert from "node:assert/strict"
import test from "node:test"
import { linkManagedSliceProviderProfile } from "./managed-slice-provider-profile.mjs"

test("managed Slice provider import uses the profile returned by product link", async () => {
 const calls=[]
 const client={async send(request){calls.push(request);if(request.LinkProviderAccountProfile)return {ProviderAccountProfile:{profile:{profile_id:"linked-run-profile"}}};if(request.GetProviderAuthStatus)return {ProviderAuthStatus:{status:{auth_state:"authenticated"}}};return {ProviderAccountProfileDefaultSet:{}}}}
 const profile=await linkManagedSliceProviderProfile(client,"codex",{CODEX_HOME:"/owned/state/provider-accounts/local/codex/account/codex"})
 assert.equal(profile.profile_id,"linked-run-profile")
 assert.equal(calls[0].LinkProviderAccountProfile.path,"/owned/state/provider-accounts/local/codex/account/codex")
 assert.equal(calls[1].GetProviderAuthStatus.account_profile,"linked-run-profile")
 assert.equal(calls[2].SetDefaultProviderAccountProfile.account_profile,"linked-run-profile")
})
test("managed Slice provider import fails before mutation without scoped input",async()=>{
 let mutated=0;await assert.rejects(linkManagedSliceProviderProfile({async send(request){if(!request.ListProviderAccountProfiles)mutated++;return {ProviderAccountProfilesListed:{profiles:[]}}}},"opencode",{}),/explicit linked provider scope/);assert.equal(mutated,0)
})
test("managed Slice provider import refuses an unauthenticated linked profile",async()=>{
 let sent=0;const client={async send(request){sent++;return request.LinkProviderAccountProfile?{ProviderAccountProfile:{profile:{profile_id:"linked-run-profile"}}}:{ProviderAuthStatus:{status:{auth_state:"expired"}}}}};await assert.rejects(linkManagedSliceProviderProfile(client,"claude",{CLAUDE_CONFIG_DIR:"/owned/state/provider-accounts/local/claude/account/claude"}),/normal product enrollment/);assert.equal(sent,2)
})

test("managed Slice reuses an already enrolled default profile without filesystem login input",async()=>{
 const calls=[];const client={async send(request){calls.push(request);if(request.ListProviderAccountProfiles)return {ProviderAccountProfilesListed:{profiles:[{profile_id:"vaulted-claude",is_default:true,auth_state:"authenticated"}]}};if(request.GetProviderAuthStatus)return {ProviderAuthStatus:{status:{auth_state:"authenticated"}}};return {ProviderAccountProfile:{profile:{profile_id:"vaulted-claude"}}}}};const profile=await linkManagedSliceProviderProfile(client,"claude",{});assert.equal(profile.profile_id,"vaulted-claude");assert.equal(calls.some(x=>x.LinkProviderAccountProfile),false)
})

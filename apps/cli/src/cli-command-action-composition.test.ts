import assert from "node:assert/strict"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { createCliCommandActionComposition, type CliCommandActionCompositionDeps } from "./cli-command-action-composition.js"
import { makeCommandDeps } from "./command-actions-test-support.js"

// MP-08 / MP-11: actual composition + slash handler, with both authority profiles.
test("signed-in attached terminal preserves kernel enrollment when unlink is rejected", async t => {
  const root=mkdtempSync(join(tmpdir(),"chariox-kauth-unlink-"))
  const previous=process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME=root
  t.after(()=>{if(previous===undefined)delete process.env.CHARIOX_HOME;else process.env.CHARIOX_HOME=previous;rmSync(root,{recursive:true,force:true})})
  const notices:string[]=[]
  let preferenceWrites=0, unlinkRequests=0
  const human={apiUrl:"http://cloud.test",accountId:"account",clientId:"terminal",cloudSessionToken:"synthetic-client-access"}
  const client={send:async(request:Record<string,unknown>)=>{
    if("CloudRelayStatus" in request)return {CloudRelayStatus:{profile:{api_url:"http://cloud.test",account_id:"account",kernel_id:"kernel",kernel_enrolled:true}}}
    if("LogoutCloudRelay" in request){unlinkRequests++;throw new Error("Cloud rejected unlink")}
    throw new Error("unexpected request")
  }}
  const deps=new Proxy({...makeCommandDeps(),client,options:{clientId:"terminal",accountProfile:"default"},preferencesState:()=>({}),setPreferencesState:()=>{preferenceWrites++},kernelConnected:()=>true,cloudClient:{humanProfile:async()=>human},appendCloudNotice:(notice:string)=>notices.push(notice)}, {get:(target,key)=>key in target?target[key as keyof typeof target]:()=>{}})
  const handlers=createCliCommandActionComposition(deps as unknown as CliCommandActionCompositionDeps)
  await assert.rejects(handlers.handleCloudCommand({kind:"cloud",raw:"/cloud unlink",args:["unlink"]}),/Cloud rejected unlink/)
  assert.equal(unlinkRequests,1)
  assert.equal(preferenceWrites,0)
  assert.equal(notices.includes("cloud link cleared"),false)
})

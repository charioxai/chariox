// MP-08 / MP-10 / MP-11 A07: masked TUI entry and explicit user input regressions.
import assert from "node:assert/strict"
import test from "node:test"
import { createKernelApprovalController, type KernelApprovalKey } from "./kernel-approval-controller.js"
import { createHandoffEntry, handoffKeyText, HANDOFF_ENTER_CHOICE } from "./kernel-handoff-entry.js"
import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import type { RuntimeHandoff, HandoffResponseAction } from "@chariox/kernel-client/owner-handoff"
const h: RuntimeHandoff = {kind:"secret",reason:"human_verification",agent_id:"agent",task_id:"task",obligation_id:"obligation",explanation:"Enter the missing value",target:{tab_id:"tab",generation:1,document_id:"doc",node_ref:"backend:1",origin:"https://github.com",path:"/login",label:"Password"},expires_at_ms:Date.now()+900_000,save_to_vault_offered:true}
const interaction: RuntimeInteraction = {id:"handoff-obligation",kernel_operation_id:"handoff-obligation",kind:"permission",level:"warning",message:"Protected step",choices:[{id:"cancel",label:"Cancel",reply:"cancel"}],requested_at_ms:1,handoff:h}
const key=(name:string):KernelApprovalKey=>({name,preventDefault(){},stopPropagation(){}})
function fixture() {
 let connected=true
 let current={id:"room",agents:[],active_interactions:[interaction]} as unknown as RuntimeSession
 const actions:HandoffResponseAction[]=[]
 const controller=createKernelApprovalController({getSession:()=>current,connected:()=>connected,onView(){},onOpen(){},onClose(){},scroll(){},notify(){},applySession(){},showPasskeyPrompt:()=>false,
 respond:async()=>{throw new Error("value reached generic interaction reply")},respondHandoff:async(_room,_id,action)=>{actions.push(action);return {handoff_id:_id,status:"completed",action:action.kind}}})
 controller.show()
 return {controller,actions,disconnect(){connected=false;controller.sync()},replace(){current={...current,active_interactions:[]};controller.sync()}}
}
test("MP-08/MP-11 A07 S04: Unicode entry exposes only length and clears after one take",()=>{
 const entry=createHandoffEntry(h)
 assert.equal(entry.add("private-fixture-🔑"),true)
 assert.equal(JSON.stringify(entry.view()).includes("private-fixture"),false)
 assert.equal(entry.add("\n"),false)
 entry.toggleSave()
 assert.deepEqual(entry.take(),{kind:"enter_value",value:"private-fixture-🔑",save_to_vault_key:"github-com-handoff"})
 assert.equal(entry.view().length,0);assert.equal(entry.take(),null)
})
test("MP-08/MP-10/MP-11 A07 S04: protected entry rejects every C0 and C1 control",()=>{
 for(const code of [...Array.from({length:32},(_,i)=>i),...Array.from({length:33},(_,i)=>i+127)]){
  const control=String.fromCodePoint(code)
  const entry=createHandoffEntry(h)
  assert.equal(entry.add(`before${control}after`),false,`control U+${code.toString(16)}`)
  assert.equal(entry.view().length,0)
  assert.equal(entry.take(),null)
  assert.equal(handoffKeyText({name:control,sequence:control}),"")
 }
 const printable=" AZaz09~é界🔑"
 const entry=createHandoffEntry(h)
 assert.equal(entry.add(printable),true)
 assert.deepEqual(entry.take(),{kind:"enter_value",value:printable})
 assert.equal(handoffKeyText({name:"up",sequence:"\u001b[A"}),"")
 assert.equal(handoffKeyText({name:"space"})," ")
})
test("MP-08/MP-11 A07 S01/S04: popup paste never reaches prompt or generic reply",async()=>{
 const f=fixture();await f.controller.choose(interaction.id,HANDOFF_ENTER_CHOICE)
 let stopped=0
 f.controller.handlePaste({rawText:"private-fixture-991",preventDefault(){stopped++},stopPropagation(){stopped++}})
 assert.equal(stopped,2)
 assert.equal(JSON.stringify(f.controller.view()).includes("private-fixture"),false)
 f.controller.handleKey(key("return"));await new Promise<void>(r=>queueMicrotask(r))
 assert.deepEqual(f.actions,[{kind:"enter_value",value:"private-fixture-991"}])
 assert.equal(f.controller.view().handoffEntry,null)
 f.controller.dispose()
})
test("MP-08/MP-11 A07 S02/S03: disconnect, replacement and Escape discard typed input",async()=>{
 for(const action of ["disconnect","replace","escape"] as const){
  const f=fixture();await f.controller.choose(interaction.id,HANDOFF_ENTER_CHOICE)
  f.controller.handlePaste({text:"private-fixture",preventDefault(){},stopPropagation(){}})
  if(action==="escape")f.controller.handleKey(key("escape"));else f[action]()
  assert.equal(f.controller.view().handoffEntry,null);assert.equal(f.actions.length,0)
  f.controller.dispose()
 }
})

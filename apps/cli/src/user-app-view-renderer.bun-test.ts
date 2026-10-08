import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"
import { createRoot } from "solid-js"
import { createTestRenderer } from "@opentui/core/testing"
import { TextareaRenderable, BoxRenderable } from "@opentui/core"
import { createCliUserAppViewsComposition } from "./cli-user-app-views-composition.js"
import { createCliKernelApprovalComposition } from "./cli-kernel-approval-composition.js"

const flag = "CHARIOX_USER_APP_VIEWS_PROTOTYPE"
test("flag off creates no panel, timers, requests or input hooks",()=>{
  const old=process.env[flag];delete process.env[flag]
  try {assert.equal(createCliUserAppViewsComposition({} as never),undefined)}finally{if(old===undefined)delete process.env[flag];else process.env[flag]=old}
})
test("text panel preserves prompt draft/focus and forwards real OpenTUI keys",async()=>{
  const old=process.env[flag];process.env[flag]="1"
  const h=await createTestRenderer({width:100,height:24,useThread:false})
  const prompt=new TextareaRenderable(h.renderer,{initialValue:"draft kept"});h.renderer.root.add(prompt);prompt.focus()
  const requests:any[]=[];const view={view_id:"v",installation_id:"fixture",generation:"g",origin:"https://app.invalid",browser:{tab_id:"host-tab-1",generation:1}}
  let dispose!:()=>void
  const client={send:async(request:any)=>{requests.push(request);if(request.OpenUserAppView)return {UserAppViewOpened:{view}};if(request.ListUserAppViews)return {UserAppViewsListed:{views:[view]}};if(request.CloseUserAppView)return {UserAppViewClosed:{view_id:"v"}};return {KernelBrowser:{result:{generation:1,snapshot:{accessibility_nodes:[{node_ref:"n",role:"button",name:"Call App",disabled:false}]}}}}}}
  const controller=createRoot(cleanup=>{dispose=cleanup;return createCliUserAppViewsComposition({
    client:()=>client, renderer:h.renderer,dimensions:()=>({width:100,height:24}),themeRevision:()=>0,currentFocus:()=>prompt,promptFocus:()=>prompt,notify(){},approvalOwnsInput:()=>false,
  })!})
  h.renderer.keyInput.on("keypress",controller.handleKey)
  try {
    await controller.handle(["view","open","fixture"]);await h.renderOnce()
    assert.match(h.captureCharFrame(),/Call App/);assert.equal(prompt.focused,false)
    h.renderer.stdin.emit("data",Buffer.from("\t"));h.mockInput.pressKey("x");h.renderer.stdin.emit("data",Buffer.from("\r"));await controller.idle()
    assert.deepEqual(requests.filter(r=>r.KernelBrowser?.command.op==="input").map(r=>r.KernelBrowser.command.input),[{kind:"key",key:"Tab"},{kind:"text",text:"x"},{kind:"key",key:"Enter"}])
    h.renderer.stdin.emit("data",Buffer.from("\x1b"));await sleep(50);assert.equal(prompt.focused,true);assert.equal(prompt.plainText,"draft kept")
    assert.equal(controller.ownsInput(),false)
    h.renderer.keyInput.off("keypress",controller.handleKey)
  }finally{dispose();h.renderer.destroy();if(old===undefined)delete process.env[flag];else process.env[flag]=old}
})
test("flagged detached approval popup labels user domain and refuses a switched kernel",async()=>{
  const old=process.env[flag];process.env[flag]="1"
  const h=await createTestRenderer({width:100,height:28,useThread:false})
  const requests:any[]=[];let listener:(event:any)=>void=()=>{},source:any
  const original={send:async(r:any)=>{requests.push(r);return {UserDomainInteractionAnswered:{interaction_id:"a"}}}}
  source=original
  const client={currentClient:()=>source,onKernelEvent:(f:any)=>{listener=f;return ()=>{}}} as never
  let dispose!:()=>void
  const approvals=createRoot(cleanup=>{dispose=cleanup;return createCliKernelApprovalComposition({client,renderer:h.renderer,
    session:()=>({id:"",agents:[],active_interactions:[]}) as never,connected:()=>false,attached:()=>false,kernelConnected:()=>true,
    dimensions:()=>({width:100,height:28}),themeRevision:()=>0,currentFocus:()=>null,promptFocus:()=>null,closeOtherDialog(){},notify(){},flashFooter(){},applySession(){},attachmentId:()=>null,
  })})
  const box=new BoxRenderable(h.renderer,{});h.renderer.root.add(box);approvals.assignPopupBox(box)
  try{
    listener({event:"passkey_prompts_changed",prompts:[{kind:"critical_approval",session_id:"",interaction_id:"a",title:"Fixture approval",message:"Allow fixture?",approve_choice_id:"allow",refuse_choice_id:"deny",requested_at_ms:Date.now(),expires_at_ms:Date.now()+60000}]})
    h.renderer.keyInput.on("keypress",approvals.handleKey);h.renderer.stdin.emit("data",Buffer.from("\x1b[19~"));await h.renderOnce();assert.match(h.captureCharFrame(),/User domain/)
    source={send:async()=>{throw Error("wrong kernel")}}
    h.mockInput.pressKey("x");h.renderer.stdin.emit("data",Buffer.from("\r"));await sleep(20)
    assert.equal(requests.length,0);assert.equal(approvals.ownsInput(),true)
  }finally{h.renderer.keyInput.off("keypress",approvals.handleKey);dispose();h.renderer.destroy();if(old===undefined)delete process.env[flag];else process.env[flag]=old}
})

import assert from "node:assert/strict"
import test from "node:test"
import { createUserAppViewController } from "./user-app-view-controller.js"
import { formatUserAppViewOutline } from "./user-app-view-outline.js"
import { passkeyPromptsFromEvent } from "./passkey-popup-controller.js"
import { answerUserDomainInteraction } from "./user-domain-interaction-api.js"
import { handleAppSlashCommand } from "./app-command-handler.js"
import { parseSlashCommand } from "./commands.js"
import { tuiAppHelp } from "./app-command-catalog.js"

const view = { view_id: "v", installation_id: "fixture", generation: "g", origin: "https://app.invalid", browser: {tab_id: "host-tab-1", generation: 3} }
const outline = { accessibility_nodes: [{ node_ref: "n", role: "textbox", name: "Message", value: "", focused: true }] }
function harness() {
  const requests: any[] = [], notices: string[] = []
  let views: any[] = [], snapshot = outline
  let interactions: any[] = [{ id: "a", level: "warning", message: "Allow fixture?", choices: [{id:"deny", label:"Deny"}] }]
  const original = { async send(request: any): Promise<any> {
    requests.push(request)
    if (request.OpenUserAppView) { views.push(view); return {UserAppViewOpened: {view}} }
    if (request.ListUserAppViews) return {UserAppViewsListed: {views}}
    if (request.CloseUserAppView) { views = views.filter(v => v.view_id !== request.CloseUserAppView.view_id); return {UserAppViewClosed: {view_id: request.CloseUserAppView.view_id}} }
    if (request.KernelBrowser) return {KernelBrowser: {result: {generation: 3, snapshot}}}
    if (request.CallUserAppView) return {UserAppViewCallResult: {result: {ok:true},error:null}}
    if (request.SubscribeUserAppViews) return {UserAppViewsChanged: {cursor:1,views,interactions}}
    if (request.AnswerUserDomainInteraction) return {UserDomainInteractionAnswered: {interaction_id:request.AnswerUserDomainInteraction.interaction_id}}
    throw new Error("unexpected request")
  } }
  let client = original, opened = 0, closed = 0
  const controller = createUserAppViewController({ client: () => client, onView() {}, onOpen(){opened++}, onClose(){closed++}, scroll(){}, notify(v){notices.push(v)} })
  return { controller, original, requests, notices, counts:()=>[opened,closed], setViews(v:any[]){views=v}, setInteractions(v:any[]){interactions=v}, setClient(v:typeof client){client=v}, setSnapshot(v:any){snapshot=v} }
}
function key(name:string, sequence?:string, extra:any={}) { return {name,sequence,...extra,preventDefault(){},stopPropagation(){}} }

test("waiting-room App opens without session and shares the kernel browser seam", async () => {
  const h = harness()
  try { assert.equal(await h.controller.handle(["open","fixture"]),true)
    assert.deepEqual(h.requests[0],{OpenUserAppView:{installation_id:"fixture",host:"kernel_browser"}})
    assert.match(h.controller.view().text,/textbox "Message".*focused/)
    assert.deepEqual(h.requests[2],{KernelBrowser:{command:{op:"snapshot",...view.browser}}})
    h.controller.hide(); assert.deepEqual(h.counts(),[1,1])
    assert.equal(h.requests.some(r=>r.CloseUserAppView),false)
  } finally { await h.controller.dispose() }
  assert.deepEqual(h.requests.at(-1),{CloseUserAppView:{view_id:"v"}})
})
test("App open uses the kernel browser, flag-off verbs send nothing",async()=>{
  const h=harness()
  try { assert.equal(await h.controller.handle(["open","fixture"]),true)
    assert.deepEqual(h.requests[0],{OpenUserAppView:{installation_id:"fixture",host:"kernel_browser"}})
    const count=h.requests.length
    assert.equal(await h.controller.handle(["open","fixture","--session","room"]),false)
    assert.equal(h.requests.length,count)
    await assert.rejects(handleAppSlashCommand({sendAppRequest:async()=>{throw Error("must not send")},appendNotice(){},flashFooter(){}},parseSlashCommand("/app views") as any),/CHARIOX_USER_APP_VIEWS_PROTOTYPE/)
    assert.equal(tuiAppHelp(true).length,tuiAppHelp(false).length+2)
  } finally {await h.controller.dispose()}
})
test("keyboard App input binds the exact owned tab; Escape never closes the instance",async()=>{
  const h=harness()
  try {await h.controller.handle(["view","open","fixture"])
    for(const event of [key("tab"),key("x","X"),key("return")]) h.controller.handleKey(event)
    await h.controller.idle()
    assert.deepEqual(h.requests.filter(r=>r.KernelBrowser?.command.op==="input").map(r=>r.KernelBrowser.command),[
      {op:"input",...view.browser,input:{kind:"key",key:"Tab"}}, {op:"input",...view.browser,input:{kind:"text",text:"X"}}, {op:"input",...view.browser,input:{kind:"key",key:"Enter"}},
    ])
    h.controller.handleKey(key("escape"));assert.equal(h.controller.ownsInput(),false)
    h.controller.handleKey(key("x","hidden"));await h.controller.idle()
    assert.equal(h.requests.filter(r=>r.KernelBrowser?.command.op==="input").length,3)
  } finally {await h.controller.dispose()}
})
test("kernel switch rejects reads/actions and cleanup uses only the original client",async()=>{
  const h=harness();const foreign:any[]=[]
  try { await h.controller.handle(["view","open","fixture"])
    h.setClient({async send(r){foreign.push(r);throw Error("wrong kernel")}})
    await assert.rejects(h.controller.handle(["view","call","echo","{}"]),/previous kernel/)
    h.controller.handleKey(key("tab"));await h.controller.idle();assert.equal(foreign.length,0)
  }finally{await h.controller.dispose()}
  assert.deepEqual(h.requests.at(-1),{CloseUserAppView:{view_id:"v"}})
})
test("closed or replaced generation never receives stale input",async()=>{
  const h=harness()
  try {await h.controller.handle(["view","open","fixture"]);h.setViews([{...view,browser:{...view.browser,generation:4}}])
    h.controller.handleKey(key("return"));await h.controller.idle()
    assert.equal(h.requests.some(r=>r.KernelBrowser?.command.op==="input"),false)
    await assert.rejects(h.controller.handle(["view","call","echo","{}"]),/ended/)
  }finally{await h.controller.dispose()}
})
test("native instances list but cannot be text projected or silently migrated",async()=>{
  const h=harness()
  try {h.setViews([{...view,browser:undefined}]);await h.controller.handle(["views"])
    assert.match(h.controller.view().text,/client native/)
    await assert.rejects(h.controller.handle(["view","show","v"]),/native rendering/)
    assert.equal(h.requests.some(r=>r.OpenUserAppView),false)
  }finally{await h.controller.dispose()}
  assert.equal(h.requests.some(r=>r.CloseUserAppView),false)
})
test("direct keyboard command calls the existing App channel with parsed data",async()=>{
  const h=harness()
  try {await h.controller.handle(["view","open","fixture"]);await h.controller.handle(["view","call","echo",'{"text":"hi"}'])
    assert.deepEqual(h.requests.at(-1),{CallUserAppView:{view_id:"v",method:"echo",input:{text:"hi"}}})
    assert.deepEqual(h.notices,['{"ok":true}'])
    await assert.rejects(h.controller.handle(["view","call","echo","bad json"]),SyntaxError)
  }finally{await h.controller.dispose()}
})
test("owner approval choices are fetched from kernel, critical proof belongs only in popup",async()=>{
  const h=harness()
  try {await h.controller.handle(["view","approvals"]);assert.match(h.controller.view().text,/Allow fixture/)
    await assert.rejects(h.controller.handle(["view","answer","a","allow"]),/No such/)
    await h.controller.handle(["view","answer","a","deny"])
    assert.deepEqual(h.requests.at(-1),{AnswerUserDomainInteraction:{interaction_id:"a",choice_id:"deny",passkey:null,passkey_remember_minutes:null}})
    h.setInteractions([{id:"a",level:"critical",choices:[{id:"approve"}]}])
    await assert.rejects(h.controller.handle(["view","answer","a","approve"]),/F8/)
  }finally{await h.controller.dispose()}
})
test("paste and App outline treat terminal escapes as data, bounded output uses no DOM",async()=>{
  const h=harness()
  try {await h.controller.handle(["view","open","fixture"])
    h.controller.handlePaste({text:"safe",rawText:"\x1b[31munsafe",preventDefault(){},stopPropagation(){}});await h.controller.idle()
    assert.equal(h.requests.some(r=>r.KernelBrowser?.command.op==="input"),false)
    const formatted=formatUserAppViewOutline(view,{accessibility_nodes:Array.from({length:600},(_,i)=>({node_ref:`n${i}`,role:"button\x1b",description:"\x07",name:"safe",states:["\x1b[31m"]})),dom_nodes:[{text:"DOM JS"}]})
    assert.doesNotMatch(formatted,/[\x00-\x08\x0b-\x1f\x7f]/);assert.doesNotMatch(formatted,/DOM JS/);assert.match(formatted,/shortened/)
  }finally{await h.controller.dispose()}
})
test("Ctrl+W explicitly closes; malformed or failed kernel open is rejected",async()=>{
  const h=harness()
  try {await h.controller.handle(["view","open","fixture"]);h.controller.handleKey(key("w",undefined,{ctrl:true}));await h.controller.idle()
    assert.equal(h.controller.ownsInput(),false);assert.match(h.notices[0]!,/closed/)
    h.setClient({async send(){return {UserAppViewOpened:{view:{view_id:"bad"}}}}})
    await assert.rejects(h.controller.handle(["view","open","fixture"]),/invalid App view/)
  }finally{await h.controller.dispose()}
})
test("session-less critical popup is flag gated; proof reply requires matching kernel acknowledgment",async()=>{
  const prompt={kind:"critical_approval",session_id:"",interaction_id:"a",title:"Allow?",message:"Fixture",approve_choice_id:"allow",refuse_choice_id:"deny",requested_at_ms:1,expires_at_ms:100}
  assert.deepEqual(passkeyPromptsFromEvent([prompt]),[]);assert.deepEqual(passkeyPromptsFromEvent([prompt],true),[prompt])
  assert.deepEqual(passkeyPromptsFromEvent([{...prompt,kind:"sudo"}],true),[])
  let request:any
  await answerUserDomainInteraction({async send(r){request=r;return {UserDomainInteractionAnswered:{interaction_id:"a"}}}},"a","allow",{passkey:"synthetic",rememberMinutes:5})
  assert.deepEqual(request,{AnswerUserDomainInteraction:{interaction_id:"a",choice_id:"allow",passkey:"synthetic",passkey_remember_minutes:5}})
  await assert.rejects(answerUserDomainInteraction({async send(){return {UserDomainInteractionAnswered:{interaction_id:"other"}}}},"a","deny"),/did not confirm/)
})


test("actual TUI slash handler opens kernel-browser App tabs with and without a Room", async () => {
  const h = harness()
  const room: any[] = []
  const deps = {userAppViews:h.controller,sendAppRequest:async(request:any)=>{room.push(request);return {AppViewOpened:{installation_id:"fixture",tab_id:"room-tab",session_id:"room"}}},appendNotice(){},flashFooter(){}}
  try {
    await handleAppSlashCommand(deps,parseSlashCommand("/app open fixture") as any)
    assert.deepEqual(h.requests[0],{OpenUserAppView:{installation_id:"fixture",host:"kernel_browser"}})
    await handleAppSlashCommand({...deps,currentAppSessionId:()=>"room"},parseSlashCommand("/app open fixture") as any)
    assert.deepEqual(room,[])
    assert.equal(h.requests.filter(r=>r.OpenUserAppView?.host==="kernel_browser").length,2)
  } finally {await h.controller.dispose()}
})
test("queued keys after Escape are discarded before kernel mutation",async()=>{
  const h=harness()
  try {await h.controller.handle(["view","open","fixture"])
    h.controller.handleKey(key("tab"));h.controller.hide();await h.controller.idle()
    assert.equal(h.requests.some(r=>r.KernelBrowser?.command.op==="input"),false)
  }finally{await h.controller.dispose()}
})

// MP-08/MP-10: a person typing into an App form field must never lose a key.
function typingKernel(delayMs: number) {
  let field = "", inFlight = 0, maxInFlight = 0
  const log: string[] = [], notices: string[] = []
  const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
  const client = { async send(request: any): Promise<any> {
    inFlight++; maxInFlight = Math.max(maxInFlight, inFlight)
    try {
      await sleep(delayMs)
      if (request.ListUserAppViews) return { UserAppViewsListed: { views: [view] } }
      if (request.OpenUserAppView) return { UserAppViewOpened: { view } }
      if (request.CloseUserAppView) return { UserAppViewClosed: { view_id: "v" } }
      const command = request.KernelBrowser?.command
      if (command?.op === "input") {
        const input = command.input
        if (input.kind === "text") { field += input.text; log.push(`t:${input.text}`) }
        else if (input.key === "Backspace") { field = field.slice(0, -1); log.push("k:Backspace") }
        else { log.push(`k:${input.key}`) }
        return { KernelBrowser: { result: { generation: 3 } } }
      }
      if (command?.op === "snapshot")
        return { KernelBrowser: { result: { generation: 3, snapshot: { accessibility_nodes: [{ node_ref: "n", role: "textbox", name: "Message", value: field, focused: true }] } } } }
      throw new Error("unexpected request")
    } finally { inFlight-- }
  } }
  const controller = createUserAppViewController({ client: () => client, onView() {}, onOpen() {}, onClose() {}, scroll() {}, notify(v) { notices.push(v) } })
  return { controller, field: () => field, log, notices, maxInFlight: () => maxInFlight }
}
const typed = (char: string) => key(char === " " ? "space" : char, char)
const pause = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

test("a burst of 100 keys is applied in order and none is dropped", async () => {
  const kernel = typingKernel(8)
  try {
    await kernel.controller.handle(["view", "open", "fixture"])
    const text = Array.from({ length: 100 }, (_, i) => "abcdefghij"[i % 10]!)
    for (const char of text) kernel.controller.handleKey(typed(char))
    await kernel.controller.idle()
    assert.ok(kernel.field() === text.join(""), `field ${kernel.field().length} of 100 keys: ${kernel.field()}`)
    assert.deepEqual(kernel.notices, [])
  } finally { await kernel.controller.dispose() }
})
test("a burst of backspaces and Enter keeps every key and its order", async () => {
  const kernel = typingKernel(8)
  try {
    await kernel.controller.handle(["view", "open", "fixture"])
    for (const char of "hello world") kernel.controller.handleKey(typed(char))
    for (let i = 0; i < 6; i++) kernel.controller.handleKey(key("backspace"))
    kernel.controller.handleKey(key("return"))
    for (const char of "again") kernel.controller.handleKey(typed(char))
    await kernel.controller.idle()
    assert.ok(kernel.field() === "helloagain", `field ${kernel.field()}`)
    assert.deepEqual(kernel.log.filter(entry => entry.startsWith("k:") && entry !== "k:Backspace"), ["k:Enter"])
    assert.ok(kernel.log.indexOf("k:Enter") < kernel.log.findIndex(entry => entry === "t:a"), "Enter stays before the later text")
    assert.deepEqual(kernel.notices, [])
  } finally { await kernel.controller.dispose() }
})
test("sustained typing at 30 keys per second never drops a key", async () => {
  const kernel = typingKernel(20)
  try {
    await kernel.controller.handle(["view", "open", "fixture"])
    const text = "the quick brown fox jumps over the lazy dog, then types sixty keys."
    for (const char of text) { kernel.controller.handleKey(typed(char)); await pause(33) }
    await kernel.controller.idle()
    assert.ok(kernel.field() === text, `field ${kernel.field().length} of ${text.length}: ${kernel.field()}`)
    assert.deepEqual(kernel.notices, [])
  } finally { await kernel.controller.dispose() }
})
test("45 backspaces at 0.35 s intervals clear the whole field", async () => {
  const kernel = typingKernel(150)
  try {
    await kernel.controller.handle(["view", "open", "fixture"])
    kernel.controller.handleKey(typed("x"))
    const start = "y".repeat(45)
    kernel.controller.handlePaste({ text: start, rawText: start, preventDefault() {}, stopPropagation() {} } as any)
    await kernel.controller.idle()
    for (let i = 0; i < 46; i++) { kernel.controller.handleKey(key("backspace")); await pause(35) }
    await kernel.controller.idle()
    assert.ok(kernel.field() === "", `left ${kernel.field().length} characters`)
  } finally { await kernel.controller.dispose() }
})

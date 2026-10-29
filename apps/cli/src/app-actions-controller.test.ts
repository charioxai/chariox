import test from "node:test"
import assert from "node:assert/strict"
import { createAppActionsController } from "./app-actions-controller.js"

const catalog = { actions: [], registrations: [{name:"add",effect:"write",inputSchema:{type:"object"}}], state:{ok:true,value:{count:1}}, warnings:[{code:"undeclared_control",message:"button#missing"}] }
const view = {installation_id:"dev",target_id:"tab",generation:"9007199254740993",development:true,catalog_json:JSON.stringify(catalog)}
function harness() {
  let session = "room", response: any = {AppViewInspected:view}, shown = "", hidden = 0
  const requests: unknown[] = []
  let client = { send: async (request: unknown): Promise<any> => { requests.push(request); return response } }
  const controller = createAppActionsController({ client:() => client,session:() => session,
    show:text => { shown = text }, hide:() => { hidden++ },scroll:() => {},notify:message => assert.fail(message) })
  return {controller,requests,get shown() {return shown},get hidden() {return hidden},setResponse(value: any) {response=value},setSession(value: string) {session=value},setClient(value: typeof client) {client=value}}
}
test("DEV toggle shows the exact catalog JSON, closes with A, and never invokes an action", async () => {
  const h = harness()
  assert.equal(await h.controller.handle(["actions","toggle"]),true)
  assert.equal(h.shown.split("\n").slice(1).join("\n"),JSON.stringify(catalog))
  assert.deepEqual(h.requests,[{InspectAppView:{session_id:"room"}}])
  assert.equal(h.controller.ownsInput(),true)
  h.controller.handleKey({name:"a"})
  assert.equal(h.controller.ownsInput(),false)
  assert.equal(h.hidden,1)
  assert.equal(await h.controller.handle(["actions"]),false)
})
test("the agent-view toggle is DEV-only and rejects surplus arguments", async () => {
  const h=harness();h.setResponse({AppViewInspected:{...view,development:false}})
  await assert.rejects(h.controller.handle(["actions","toggle"]),/only on a DEV installation/)
  assert.equal(h.controller.ownsInput(),false)
  await assert.rejects(h.controller.handle(["actions","toggle","extra"]),/usage/)
})
test("a Room or kernel switch hides the panel and discards delayed reads",async () => {
  const h=harness();await h.controller.handle(["actions","toggle"])
  h.setSession("other");assert.equal(h.controller.ownsInput(),false)
  let finish!: (value:any) => void
  h.setClient({send:() => new Promise(resolve => {finish=resolve})})
  const pending=h.controller.handle(["actions","toggle"])
  h.setSession("third");finish({AppViewInspected:view});await pending
  assert.equal(h.controller.ownsInput(),false)
})

test("DEV panel preserves kernel JSON number formatting and numeric-key order",async () => {
  const h=harness()
  const catalog_json='{"actions":[],"registrations":[],"state":{"ok":true,"value":{"20":1.0,"3":2.0}},"warnings":[]}'
  h.setResponse({AppViewInspected:{...view,catalog_json}})
  await h.controller.handle(["actions","toggle"])
  assert.equal(h.shown.slice(h.shown.indexOf("\n")+1),catalog_json)
})

test("DEV inspector lets F8 reach the shared kernel approval handler",async () => {
  const h=harness();await h.controller.handle(["actions","toggle"])
  assert.equal(h.controller.handleKey({name:"f8"}),false)
  assert.equal(h.controller.ownsInput(),true)
})


test("DEV inspector consumes release and repeat events without refreshing, scrolling or closing", async () => {
  const requests: unknown[] = [], scrolls: number[] = []
  let hidden = 0, prevented = 0, stopped = 0
  const controller = createAppActionsController({
    client: () => client, session: () => "room", show: () => {},
    hide: () => { hidden++ }, scroll: direction => { scrolls.push(direction) },
    notify: message => assert.fail(message),
  })
  const client = { send: async (request: unknown) => { requests.push(request); return {AppViewInspected:view} } }
  await controller.handle(["actions", "toggle"])
  for (const eventType of ["release", "repeat"] as const) {
    for (const name of ["r", "a", "escape", "pageup", "pagedown"]) {
      assert.equal(controller.handleKey({name, eventType,
        preventDefault: () => { prevented++ }, stopPropagation: () => { stopped++ },
      }), true)
      assert.equal(controller.ownsInput(), true)
    }
  }
  assert.equal(requests.length, 1)
  assert.deepEqual(scrolls, [])
  assert.equal(hidden, 0)
  assert.equal(prevented, 10)
  assert.equal(stopped, 10)
  assert.equal(controller.handleKey({name: "r", eventType: "press",
    preventDefault: () => { prevented++ }, stopPropagation: () => { stopped++ },
  }), true)
  await new Promise(resolve => setImmediate(resolve))
  assert.deepEqual(requests, [{InspectAppView:{session_id:"room"}}, {InspectAppView:{session_id:"room",target_id:"tab"}}])
  assert.equal(prevented, 11)
  assert.equal(stopped, 11)
})

import test from "node:test"
import assert from "node:assert/strict"
import { executeAppCommand } from "./shell-app-command.js"
const catalog = {actions:[],registrations:[{name:"add",effect:"write",inputSchema:{type:"object"}}],state:{ok:true,value:{count:1}},warnings:[{code:"undeclared_control",message:"button#missing"}]}
const view = {installation_id:"dev",target_id:"tab",generation:"9007199254740993",development:true,catalog_json:JSON.stringify(catalog)}
test("/app actions uses the open view read and preserves catalog JSON bytes",async () => {
  const requests:unknown[]=[]
  const client={send:async (request:unknown) => {requests.push(request);return {AppViewInspected:view}}}
  const result=await executeAppCommand(["actions","--json"],client,{sessionId:"room"})
  assert.equal(result.ok,true);assert.equal(result.message,JSON.stringify(catalog))
  assert.deepEqual(requests,[{InspectAppView:{session_id:"room"}}])
  const text=await executeAppCommand(["actions","--tab","tab","--session","other"],client)
  assert.match(text.message!,/DEV App dev.*generation 9007199254740993/)
  assert.match(text.message!,/"effect":"write"/);assert.match(text.message!,/undeclared_control: button#missing/)
  assert.deepEqual(requests[1],{InspectAppView:{session_id:"other",target_id:"tab"}})
})
test("/app actions validates flags and requires a Room without side effects",async () => {
  const client={send:async () => {throw new Error("must not send")}}
  for (const args of [["actions"],["actions","--tab"],["actions","--json","--json"],["actions","--unknown"],["actions","toggle"]]) {
    assert.equal((await executeAppCommand(args,client)).ok,false)
  }
})

test("catalog JSON survives numeric keys and integral floats byte for byte", async () => {
  const catalog_json = '{"actions":[],"registrations":[],"state":{"ok":true,"value":{"20":1.0,"3":2.0}},"warnings":[]}'
  assert.notEqual(JSON.stringify(JSON.parse(catalog_json)),catalog_json)
  const result = await executeAppCommand(["actions","--json"],{send:async () => ({AppViewInspected:{...view,catalog_json}})},{sessionId:"room"})
  assert.equal(result.message,catalog_json)
})

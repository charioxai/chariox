import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcError } from "./ipc.js"
import { requestInitialSessionView } from "./initial-session-view.js"
const options = {clientId:"cli",sessionId:"18de8597bddf72dc",model:"default",effort:"",accountProfile:"work"}
test("MP-08 / MP-10 / MP-11 aliases and custom pane groups retain ordinary resolution", () => {
  const unexpected=async()=>{throw Error("unnecessary early attach")}
  assert.equal(requestInitialSessionView({} as never,{...options,sessionId:"review"},{},unexpected),null)
  assert.equal(requestInitialSessionView({} as never,options,{ui:{maxAgentsPerScreen:4}},unexpected),null)
  assert.equal(requestInitialSessionView({} as never,{...options,createSession:true},{},unexpected),null)
})
test("MP-08 / MP-10 / MP-11 a hexadecimal alias falls back only when the ID is absent", async () => {
  const missing=async()=>{throw new LocalIpcError("attach view","missing","session_not_found",false)}
  assert.equal(await requestInitialSessionView({} as never,options,{},missing),null)
  const denied=async()=>{throw new LocalIpcError("attach view","denied","capability_denied",false)}
  await assert.rejects(requestInitialSessionView({} as never,options,{},denied)!, /denied/)
})

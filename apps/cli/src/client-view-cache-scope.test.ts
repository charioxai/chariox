// MP-08 / MP-10 / MP-11: the mounted app's mutable client must retain only its current target scope.
import assert from "node:assert/strict"
import test from "node:test"
import {bindCliViewScope,cliViewScope} from "./client-view-cache.js"
import {createMutableLocalIpcClient} from "./mutable-local-ipc-client.js"
import type {LocalIpcClient} from "./ipc.js"

test("MP-11 cached view scope follows the real mutable IPC wrapper without borrowing a previous target",async()=>{
 const client={send:async(request:object)=>"RelayStatus" in request?{RelayStatus:{status:{daemon_id:"kernel-a"}}}:{CloudRelayStatus:{profile:null}}} as unknown as LocalIpcClient
 const expected=await bindCliViewScope(client,{} as never,{})
 assert.ok(expected)
 const mounted=createMutableLocalIpcClient(client)
 assert.deepEqual(await cliViewScope(mounted),expected)
 const other={...client} as LocalIpcClient
 mounted.swapClient(other)
 assert.equal(await cliViewScope(mounted),null,"a target pivot must not reuse its source principal/kernel cache scope")
 mounted.swapClient(client)
 assert.deepEqual(await cliViewScope(mounted),expected)
})

test("MP-11 logout fences an already-resolved cache principal", async () => {
  const {clearClientViews} = await import("./client-view-cache.js")
  const client = {send: async (request: object) => "RelayStatus" in request
    ? {RelayStatus: {status: {daemon_id: "kernel-a"}}} : {CloudRelayStatus: {profile: null}}} as unknown as LocalIpcClient
  assert.ok(await bindCliViewScope(client, {} as never, {}))
  assert.ok(await cliViewScope(client))
  await clearClientViews()
  assert.equal(await cliViewScope(client), null)
})

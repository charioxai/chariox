import assert from "node:assert/strict"
import test from "node:test"
import { WebSocketServer, type WebSocket } from "ws"
import { LocalIpcClient } from "./ipc.js"

test("local binary display events preserve non-UTF8 image bytes", { timeout: 10_000 }, async t => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address(); assert.ok(address && typeof address === "object")
  const payload = Buffer.from([0, 127, 128, 254, 255, 195, 40])
  const header = Buffer.from(JSON.stringify({ event: "kernel_browser_frame", subscription_id: "display",
    frame: { sequence: 1, kind: "png", data: [0, payload.length] } }))
  const prefix = Buffer.alloc(8); prefix.write("CXD1"); prefix.writeUInt32BE(header.length, 4)
  server.on("connection", socket => socket.on("message", data => {
    const request = JSON.parse(String(data))
    socket.send(JSON.stringify({ type: "response", request_id: request.request_id, response: { ok: true }, error: null }))
    socket.send(Buffer.concat([prefix, header, payload]))
  }))
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`)
  t.after(async () => { client.destroy(); for (const socket of server.clients) socket.terminate(); await new Promise<void>(resolve => server.close(() => resolve())) })
  let received!: (value: unknown) => void
  const frame = new Promise<unknown>(resolve => { received = resolve })
  client.onKernelEvent(event => { if (event.event === "kernel_browser_frame") received(event.frame) })
  await client.send({ KernelBrowser: { command: { op: "display_next", subscription_id: "display", generation: 1, after_sequence: 0 } } })
  const result = await Promise.race([frame, new Promise(resolve => setTimeout(() => resolve(null), 1000))])
  assert.ok(result, "the local client must deliver the binary display event")
  assert.deepEqual((result as { data: Uint8Array }).data, new Uint8Array(payload))
})

test("MD-DISPLAY transient control frames preserve the session cursor across reconnect", async (t) => {
  const server=new WebSocketServer({host:"127.0.0.1",port:0})
  await new Promise<void>(resolve=>server.once("listening",resolve))
  const address=server.address();assert.ok(address&&typeof address==="object")
  let eventSocket:WebSocket|undefined, subscriptions=0
  let sessionReady!:()=>void, resumed!:(cursor:number|null)=>void
  const ready=new Promise<void>(resolve=>{sessionReady=resolve})
  const resume=new Promise<number|null>(resolve=>{resumed=resolve})
  server.on("connection",socket=>socket.on("message",payload=>{
    const frame=JSON.parse(String(payload))
    if(frame.type==="subscribe"){
      eventSocket=socket;subscriptions++
      if(subscriptions>1)resumed(frame.resume_from_event_id)
      socket.send(JSON.stringify({type:"response",request_id:frame.request_id,response:{ok:true},error:null}))
      if(subscriptions===1)socket.send(JSON.stringify({type:"event",event_id:500,event:{event:"session_metadata_changed"}}))
    }else{
      socket.send(JSON.stringify({type:"event",event_id:1,event:{event:"kernel_browser_frame",subscription_id:"display",frame:{sequence:1}}}))
      socket.send(JSON.stringify({type:"response",request_id:frame.request_id,response:{ok:true},error:null}))
    }
  }))
  const client=new LocalIpcClient(`ws://127.0.0.1:${address.port}`,{reconnectJitterMs:0})
  client.onKernelEvent(event=>{if(event.event==="session_metadata_changed")sessionReady()})
  t.after(async()=>{client.destroy();for(const socket of server.clients)socket.terminate();await new Promise<void>(resolve=>server.close(()=>resolve()))})
  await client.subscribeToKernelEvents("session","attachment");await ready
  await client.send({KernelBrowser:{command:{op:"display_next",subscription_id:"display",generation:1,after_sequence:0}}})
  eventSocket!.terminate()
  assert.equal(await Promise.race([resume,new Promise(resolve=>setTimeout(()=>resolve("timeout"),5000))]),500)
})

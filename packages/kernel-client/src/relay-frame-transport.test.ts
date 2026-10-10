import test from "node:test"
import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { RelayFrameReceiver, relayTransportUrl, relayMaxMessageBytes } from "./relay-frame-transport.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
const chunk = (id: number, offset: number, total: number, payload: string) => JSON.stringify({kind:"transport_chunk",transfer_id:id,offset,total_bytes:total,payload})
test("MP-08 MP-10 MP-11 UTF-8 reassembly preserves correlation and interleaved small frames", () => {
  const receiver = new RelayFrameReceiver(), receipts: string[] = []
  const text = JSON.stringify({kind:"client_response",request_id:"large",payload:"🙂".repeat(8000)})
  let completed: string | null = null, offset = 0
  const points = Array.from(text)
  const pieces = Array.from({length: Math.ceil(points.length/2000)}, (_,n) => points.slice(n*2000,(n+1)*2000).join(""))
  for (const payload of pieces) {
    if (!payload) continue
    completed = receiver.receive(chunk(1,offset,new TextEncoder().encode(text).length,payload), frame => receipts.push(frame))
    offset += new TextEncoder().encode(payload).length
    assert.equal(receiver.receive('{"kind":"client_response","request_id":"small"}', () => assert.fail()), '{"kind":"client_response","request_id":"small"}')
  }
  assert.equal(completed,text)
  assert.equal(JSON.parse(receipts.at(-1)!).offset,new TextEncoder().encode(text).length)
})
test("MP-08 MP-10 MP-11 forged bounds, overlapping and cross-transfer chunks fail without receipts or secret output", () => {
  const secret = "private-canary", receiver = new RelayFrameReceiver(), receipt = () => assert.fail("invalid chunks cannot advance credit")
  for (const value of [chunk(0,0,17000,secret), chunk(1,0,relayMaxMessageBytes+1,secret), chunk(1,0,17000,"x".repeat(17000)),chunk(1,1,17000,secret)]) {
    assert.throws(() => receiver.receive(value,receipt), error => error instanceof Error && !error.message.includes(secret))
  }
  receiver.receive(chunk(1,0,17000,secret),() => {})
  for (const value of [chunk(1,0,17000,secret),chunk(2,secret.length,17000,secret),chunk(1,secret.length,18000,secret)]) assert.throws(() => receiver.receive(value,receipt))
})
test("MP-08 MP-10 MP-11 transport wire snapshot requires protocol 490", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION,490)
  assert.equal(relayTransportUrl("wss://example.test/socket?existing=1"),"wss://example.test/socket?existing=1&chariox_transport=chunks-v1")
  assert.equal(createHash("sha256").update(chunk(1,0,17000,"public")).digest("hex"),"63f2c15eb230dffa4358b314b5c798c07ac7d6c726781fd09314f6e2810b9e05")
})

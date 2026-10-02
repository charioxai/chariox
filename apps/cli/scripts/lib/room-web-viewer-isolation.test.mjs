// MP-08/MP-10/MP-11: viewer slowdown must preserve terminal transport.
import assert from 'node:assert/strict'
import test from 'node:test'
import { createWebFaultTransport } from './room-web-fault-transport.mjs'
function route(url) {
  const received=[];let fromClient,fromServer;let closed=0
  const server={send:value=>received.push(['server',value]),onMessage:f=>fromServer=f,close:()=>closed++}
  const socket={url:()=>url,send:value=>received.push(['client',value]),onMessage:f=>fromClient=f,connectToServer:()=>server,close:()=>closed++}
  return {socket,received,client:value=>fromClient(value),server:value=>fromServer(value),closed:()=>closed}
}
test('MP-08/MP-10 slow viewer does not delay terminal packets or close terminal recovery',async()=>{
 const transport=createWebFaultTransport({delayMs:10}),terminal=route('ws://127.0.0.1:1/relay'),viewer=route('ws://127.0.0.1:1/display/owned/stream')
 transport.route(terminal.socket);transport.route(viewer.socket);transport.setMode('slow-viewer')
 terminal.client('terminal-control');viewer.server('opaque-encrypted-frame')
 assert.deepEqual(terminal.received,[['server','terminal-control']])
 assert.deepEqual(viewer.received,[])
 await new Promise(r=>setTimeout(r,20));assert.deepEqual(viewer.received,[['client','opaque-encrypted-frame']])
 transport.setMode('valid');assert.equal(terminal.closed(),0);assert.equal(viewer.closed(),2);transport.dispose()
})

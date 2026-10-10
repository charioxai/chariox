// MP-08/MP-10/MP-11: timing must never retain auth or plaintext.
import test from 'node:test';import assert from 'node:assert/strict';
import{packetSignature,transportHops}from'./browser-computer-timing.mjs';
test('MP-10 opaque ciphertext joins JSON and binary routing forms',()=>{
 const ciphertext=Buffer.alloc(16,7),json=JSON.stringify({kind:'client_event',event_id:7,encrypted_event:{ciphertext:ciphertext.toString('base64')}});
 const header=Buffer.from(JSON.stringify({kind:'client_event',event_id:7})),prefix=Buffer.alloc(8);prefix.write('CXR1');prefix.writeUInt32BE(header.length,4);
 assert.equal(packetSignature(1,json),packetSignature(2,Buffer.concat([prefix,header,ciphertext]).toString('base64')));
 assert(Number.isSafeInteger(packetSignature(1,json)));
 assert.equal(packetSignature(1,json,true).sequence,7);
 assert.deepEqual(packetSignature(1,json,true),packetSignature(2,Buffer.concat([prefix,header,ciphertext]).toString('base64'),true));
 for(const payload of ['{"kind":"client_hello","auth_token":"synthetic"}','{"kind":"daemon_register"}','invalid'])assert.equal(packetSignature(1,payload),null);
});
test('MP-10 hop decomposition only joins the same packet and rejects reversed clocks',()=>{
 const row=(stage,at_ms,packet=1)=>({stage,at_ms,packet});
 const hops=transportHops([row('viewer_send',100),row('relay_read',135),row('relay_write_start',136),row('relay_write_end',137),row('kernel_receive',138),row('kernel_request_start',139),row('kernel_request_end',149),row('viewer_receive',150,2)]);
 assert.equal(hops.viewer_to_relay.p95_ms,35);assert.equal(hops.relay_to_kernel.p95_ms,1);assert.equal(hops.kernel_queue.p95_ms,1);assert.equal(hops.kernel_request.p95_ms,10);assert.equal(hops.relay_to_viewer.samples,0);
 assert.equal(transportHops([row('viewer_send',10),row('relay_read',5)]).viewer_to_relay.samples,0);
});

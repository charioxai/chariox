// MP-08/MP-10/MP-11: allocated placement contract and bump must move together.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {localProtocolVersion,relayPeerProtocolVersion} from './lib/protocol-versions.mjs';
const shapes=new URL('../apps/kernel/src/local/api/tests/protocol_shapes/',import.meta.url);
test('MP-08/MP-10/MP-11: page placement has its allocated 529/137 floor and pinned wire shape',()=>{
 const bytes=readFileSync(new URL('browser-display-placement-529.json',shapes));
 const fixture=JSON.parse(bytes);
 assert(localProtocolVersion>=fixture.local,'page placement requires local529');
 assert(relayPeerProtocolVersion>=fixture.relay_peer,'page placement requires peer137');
 const rust=readFileSync(new URL('kernel_browser.rs',shapes),'utf8');
 assert(rust.includes(createHash('sha256').update(bytes).digest('hex')),'Rust snapshot must pin these exact bytes');
 const command=fixture.request.KernelBrowser.command,result=fixture.response.KernelBrowser.result;
 assert.equal(command.target.kind,'region');
 assert.deepEqual(Object.keys(command.target).sort(),['document_id','kind','mirror_subscription_id','region_id','sequence']);
 assert.deepEqual(result.region_offset,{x:10.5,y:5.25});
});


test('MP-08/MP-10/MP-11: placement upgrade retains the preceding 528 release',()=>{
 const transitions=JSON.parse(readFileSync(new URL('../apps/kernel/managed-upgrade-protocol-transitions.json',import.meta.url)));
 assert(transitions.upgradeFrom.includes(528));
 assert(transitions.rollbackTo.includes(528));
});

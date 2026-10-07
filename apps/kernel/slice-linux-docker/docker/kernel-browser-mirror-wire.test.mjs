import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {gunzipSync} from 'node:zlib';
import {readFileSync} from 'node:fs';
import {mirrorChunks,mirrorWireProtocolVersion,mirrorWirePeerVersion} from './kernel-browser-mirror-wire.mjs';
function shape(){
 const packet={subscription_id:'s',tab_id:'t',generation:2,document_id:'d',sequence:3,base_sequence:null,reset:true,hash:'a'.repeat(64),root:'n1',nodes:[{id:'n1',parent:null,children:['n2'],kind:'element',tag:'p',style:{color:'black'}},{id:'n2',parent:'n1',children:[],kind:'text',text:'x'.repeat(200000)}],removed:[],resources:[],fonts:[],scroll:{x:0,y:0},focused:null,selection:null,tiles:[],css_width:1280,css_height:800,device_scale_factor:2};
 const chunks=mirrorChunks(packet),first=chunks[0],frame=JSON.parse(gunzipSync(Buffer.concat(chunks.map(c=>Buffer.from(c.payload_base64,'base64')))));
 return {local_protocol:mirrorWireProtocolVersion,relay_peer:mirrorWirePeerVersion,chunk:{...first,parts:'<integer>',frame_bytes:'<integer>',transfer_bytes:'<integer>',payload_base64:'<base64>'},frame_fields:Object.keys(frame).sort(),style_ref:frame.nodes[0].style_ref,styles:frame.styles};
}
test('MD-454/94: compressed credit and CSS palette response shape/hash are pinned',()=>{
 assert.equal(mirrorWireProtocolVersion,454);assert.equal(mirrorWirePeerVersion,94);
 const actual=shape(),expected=JSON.parse(readFileSync(new URL('./kernel-browser-mirror-wire-454.json',import.meta.url)));
 assert.deepEqual(actual,expected);assert.equal(createHash('sha256').update(JSON.stringify(actual)).digest('hex'),'51a2977a3f6c79d185188b762bf7059b963db079bb98da4b583152be1c6a7181');
});
export {shape};

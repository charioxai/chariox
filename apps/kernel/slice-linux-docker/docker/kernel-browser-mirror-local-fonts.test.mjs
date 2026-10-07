import test from 'node:test';import assert from 'node:assert/strict';import {inflateSync} from 'node:zlib';
import {sfntToWoff,mirrorFontKey,materializeMirrorLocalFonts,readSystemFont} from './kernel-browser-mirror-local-fonts.mjs';
function sfnt(){const bytes=Buffer.alloc(112);bytes.writeUInt32BE(0x10000);bytes.writeUInt16BE(1,4);bytes.write('test',12);bytes.writeUInt32BE(123,16);bytes.writeUInt32BE(28,20);bytes.writeUInt32BE(84,24);bytes.fill(65,28);return bytes}
test('WOFF preserves sfnt table bytes, checksums, directory bounds and padded decoded size',()=>{
 const bytes=sfnt(),woff=sfntToWoff(bytes);assert.equal(woff.toString('ascii',0,4),'wOFF');assert.equal(woff.readUInt32BE(8),woff.length);assert.equal(woff.readUInt32BE(16),bytes.length);assert.equal(woff.readUInt32BE(60),123);
 const at=woff.readUInt32BE(48),length=woff.readUInt32BE(52);assert.deepEqual(inflateSync(woff.subarray(at,at+length)),bytes.subarray(28));
 const bad=Buffer.from(bytes);bad.writeUInt32BE(999,20);assert.throws(()=>sfntToWoff(bad),/table/);assert.throws(()=>sfntToWoff(Buffer.from('not a font')),/unsupported/);
});
test('font resolver refuses paths, fontconfig pattern injection and non-font identifiers',async()=>{
 for(const name of ['../../keys/private','Arial:file=/private','--help','name\nprivate'])assert.equal(await readSystemFont(name),null);
});
test('only native public font references change sanitized CSS; protected policies perform no font reads',async()=>{
 const style={'font-family':'sans-serif','font-weight':'400'},calls=[];
 const world={sessionId:'s',contextId:1,connection:{async send(method,params){calls.push(method);
  if(method==='Runtime.evaluate')return params.returnByValue?{result:{value:[mirrorFontKey(style)]}}:{result:{objectId:'public-fonts'}};
  if(method==='Runtime.getProperties')return {result:[{name:'0',value:{subtype:'node',objectId:'public-text-parent'}}]};
  if(method==='DOM.requestNode')return {nodeId:7};
  if(method==='CSS.getPlatformFontsForNode')return {fonts:[{postScriptName:'Fixture-Regular',isCustomFont:false,glyphCount:12}]};return {};
 }}};
 const source={nodes:[{style:{...style}}]},body=sfnt();let reads=0;
 const material=await materializeMirrorLocalFonts(world,source,[],{readFont:async name=>{assert.equal(name,'Fixture-Regular');reads++;return body}});
 assert.equal(reads,1);assert.equal(material.resources.length,1);assert.equal(material.fonts.length,1);assert.match(source.nodes[0].style['font-family'],/^"cx-native-[a-f0-9]{64}"$/);assert.equal(calls.at(-1),'Runtime.releaseObjectGroup');
 calls.length=0;assert.deepEqual(await materializeMirrorLocalFonts(world,source,['synthetic protected bytes']),{resources:[],fonts:[]});assert.deepEqual(calls,[]);
});
test('native ascent/descent are size-specific, with valid sfnt checksums and unchanged glyph tables',async()=>{
 const {normalizeFontMetrics}=await import('./kernel-browser-mirror-local-fonts.mjs');
 const entries=[['OS/2',Buffer.alloc(78)],['glyf',Buffer.from('PUBLIC GLYPH DATA')],['head',Buffer.alloc(54)],['hhea',Buffer.alloc(36)]];
 let size=12+16*entries.length;const offsets=entries.map(([,body])=>{const offset=size;size+=(body.length+3)&~3;return offset}),bytes=Buffer.alloc(size);
 bytes.writeUInt32BE(0x10000);bytes.writeUInt16BE(entries.length,4);
 entries.forEach(([tag,body],i)=>{const at=12+16*i;bytes.write(tag,at);bytes.writeUInt32BE(offsets[i],at+8);bytes.writeUInt32BE(body.length,at+12);body.copy(bytes,offsets[i])});
 bytes.writeUInt16BE(2048,offsets[2]+18);bytes.writeInt16BE(1901,offsets[3]+4);bytes.writeInt16BE(-483,offsets[3]+6);
 const before=Buffer.from(bytes),after=normalizeFontMetrics(bytes,{size:32,ascent:30,descent:8});assert.deepEqual(bytes,before);
 assert.equal(after.readInt16BE(offsets[3]+4),1920);assert.equal(after.readInt16BE(offsets[3]+6),-512);
 assert.equal(after.readInt16BE(offsets[0]+68),1920);assert.equal(after.readUInt16BE(offsets[0]+76),512);
 assert.deepEqual(after.subarray(offsets[1],offsets[1]+entries[1][1].length),entries[1][1]);
 let sum=0;for(let i=0;i<after.length;i+=4)sum=(sum+after.readUInt32BE(i))>>>0;assert.equal(sum,0xb1b0afba);
 assert.equal(normalizeFontMetrics(bytes,{size:0,ascent:30,descent:8}),bytes);assert.equal(normalizeFontMetrics(bytes,{size:32,ascent:999,descent:8}),bytes);
 assert.notEqual(mirrorFontKey({'font-size':'16px'}),mirrorFontKey({'font-size':'32px'}));
});

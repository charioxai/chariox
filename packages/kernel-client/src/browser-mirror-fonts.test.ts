import test from 'node:test'
import assert from 'node:assert/strict'
import {deflateSync} from 'node:zlib'
import {nativeFontMetricOverrides} from './browser-mirror-fonts.js'
function font(compressed=true){
 const head=Buffer.alloc(54),hhea=Buffer.alloc(36);head.writeUInt16BE(4096,16);head.writeUInt16BE(2048,18);hhea.writeInt16BE(1843,4);hhea.writeInt16BE(-538,6)
 const tables=[['head',head],['hhea',hhea]] as const,parts=tables.map(([,b])=>compressed?deflateSync(b):b);let end=84;const offsets=parts.map(b=>{const n=end;end+=(b.length+3)&~3;return n}),bytes=Buffer.alloc(end)
 bytes.write('wOFF');bytes.writeUInt32BE(0x10000,4);bytes.writeUInt32BE(end,8);bytes.writeUInt16BE(2,12)
 tables.forEach(([tag,b],i)=>{const at=44+20*i;bytes.write(tag,at);bytes.writeUInt32BE(offsets[i]!,at+4);bytes.writeUInt32BE(parts[i]!.length,at+8);bytes.writeUInt32BE(b.length,at+12);parts[i]!.copy(bytes,offsets[i])});return bytes
}
test('native WOFF metrics preserve fractional ascent/descent after verified decompression',async()=>{
 for(const compressed of [false,true])assert.deepEqual(await nativeFontMetricOverrides(font(compressed)),{ascent:'89.990234%',descent:'26.269531%'})
})
test('ordinary, malformed, oversized and unmarked fonts do not supply metric overrides',async()=>{
 assert.equal(await nativeFontMetricOverrides(Buffer.from('ordinary web font')),null)
 const raw=font(false);raw.writeUInt16BE(0,84+16);assert.equal(await nativeFontMetricOverrides(raw),null)
 const bad=font();bad.writeUInt32BE(1000000,44+12);assert.equal(await nativeFontMetricOverrides(bad),null)
 const truncated=font().subarray(0,90);assert.equal(await nativeFontMetricOverrides(truncated),null)
 const bomb=font();bomb.writeUInt32BE(1,44+12);assert.equal(await nativeFontMetricOverrides(bomb),null)
})

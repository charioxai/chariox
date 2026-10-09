// MD-5: inspect decoded pixels, not a success string from the capture helper.
import test from "node:test";
import assert from "node:assert/strict";
import { inflateSync, deflateSync, crc32 } from "node:zlib";
import { randomBytes } from "node:crypto";
import { MAX_BROWSER_ARTIFACT_BYTES } from "./browser-controller-artifacts.mjs";
import { decodePng, encodePng, maskPng, captureProtectedPage, wholeFrameMask } from "./kernel-browser-pixels.mjs";

function decoded(data) {
  const png = Buffer.from(data, "base64"), parts = [];
  const width = png.readUInt32BE(16), height = png.readUInt32BE(20);
  for (let i = 8; i < png.length;) {
    const length = png.readUInt32BE(i);
    if (png.toString("ascii", i + 4, i + 8) === "IDAT") parts.push(png.subarray(i + 8, i + 8 + length));
    i += length + 12;
  }
  const rows = inflateSync(Buffer.concat(parts));
  const pixel = (x, y) => [...rows.subarray(y * (width * 4 + 1) + 1 + x * 4, y * (width * 4 + 1) + 1 + x * 4 + 4)];
  return { width, height, pixel };
}
test("MD-5: trusted mask replaces pixels while keeping unrelated pixels", () => {
  const pixels = Buffer.alloc(20 * 20 * 4, 255);
  const original = encodePng(20, 20, pixels);
  const masked = decoded(maskPng(original, [[8, 8, 2, 2]]));
  assert.deepEqual(masked.pixel(8, 8), [0, 0, 0, 255]);
  assert.deepEqual(masked.pixel(4, 4), [0, 0, 0, 255]); // padding
  assert.deepEqual(masked.pixel(0, 0), [255, 255, 255, 255]);
  assert.deepEqual(decoded(original).pixel(8, 8), [255, 255, 255, 255]);
  assert.throws(() => maskPng(original.slice(0, -12), []), /frame/);
});
test("MP-11: registration alone leaves every pixel visible; failed target lookup refuses capture", async () => {
  let captures=0;
  assert.equal(await captureProtectedPage({}, {}, ['synthetic-only-secret'], [], async()=>{captures++;return 'ordinary';}), 'ordinary');
  await assert.rejects(captureProtectedPage({}, {target_id:'target'}, ['synthetic-only-secret'], [{target_id:'target'}], async()=>{captures++;return 'raw';}), /capture unavailable/);
  assert.equal(captures,1);
});

test('MP-08/MP-11: negotiated controller bounds admit exact images and reject mismatches', () => {
  const bounds={width:1920,height:1080};
  const png=encodePng(bounds.width,bounds.height,Buffer.alloc(bounds.width*bounds.height*4,255));
  assert.throws(()=>decodePng(png),/frame format/,'display defaults remain bounded');
  const frame=decodePng(maskPng(png,[[100,100,20,20]],1,bounds),1,bounds);
  assert.deepEqual([...frame.pixels.subarray((110*frame.width+110)*4,(110*frame.width+110)*4+4)],[0,0,0,255]);
  assert.throws(()=>maskPng(png,[],1,{width:1921,height:1080}),/frame/);
  assert.throws(()=>decodePng(png,1,{width:1920,height:1079}),/frame/);
  for(const width of [0,-1,NaN,Infinity,1.5])assert.throws(()=>decodePng(png,1,{width,height:1080}),/frame/);
});

test('MP-08/MP-11: controller byte bounds preserve host limits and bounded inflation', () => {
  const viewport={width:1800,height:1000,maxBytes:MAX_BROWSER_ARTIFACT_BYTES};
  const pixels=randomBytes(viewport.width*viewport.height*4);
  const png=encodePng(viewport.width,viewport.height,pixels);
  const size=Buffer.from(png,'base64').length;
  assert(size>4*1024*1024&&size<MAX_BROWSER_ARTIFACT_BYTES);
  assert.throws(()=>decodePng(png,2),/unsupported frame/,'host-display byte cap stays 4 MiB');
  assert.throws(()=>decodePng(png,1,{width:viewport.width,height:viewport.height}),/unsupported frame/);
  assert.deepEqual(decodePng(png,1,viewport).pixels,pixels);
  assert.equal(decodePng(maskPng(png,[],1,viewport),1,viewport).width,viewport.width);
  assert.throws(()=>decodePng(png,1,{...viewport,maxBytes:size-1}),/unsupported frame/);
  for(const maxBytes of [0,-1,NaN,Infinity,1.5])assert.throws(()=>decodePng(png,1,{...viewport,maxBytes}),/frame/);
  const oversized=encodePng(1800,1200,randomBytes(1800*1200*4));
  assert(Buffer.from(oversized,'base64').length>MAX_BROWSER_ARTIFACT_BYTES);
  assert.throws(()=>decodePng(oversized,1,{...viewport,height:1200}),/unsupported frame/);
  const header=Buffer.alloc(13);header.writeUInt32BE(1);header.writeUInt32BE(1,4);header[8]=8;header[9]=6;
  const chunk=(type,body)=>{const bytes=Buffer.concat([Buffer.from(type),body]),result=Buffer.alloc(body.length+12);result.writeUInt32BE(body.length);bytes.copy(result,4);result.writeUInt32BE(crc32(bytes),result.length-4);return result;};
  const bomb=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(Buffer.alloc(1024))),chunk('IEND',Buffer.alloc(0))]);
  assert.throws(()=>decodePng(bomb.toString('base64'),1,{width:1,height:1,maxBytes:MAX_BROWSER_ARTIFACT_BYTES}),/larger than/,'inflation remains bound to exact dimensions');
});

// MD-DISPLAY-02/04: arbitrary RGB/RGBA rows exercise every PNG predictor,
// including first-row and left-edge rules; expected pixels are independent.
test("MD-DISPLAY optimized PNG filters preserve all pixels and reject corruption", () => {
  const chunk = (type, body) => {
    const bytes = Buffer.concat([Buffer.from(type), body]), result = Buffer.alloc(body.length + 12);
    result.writeUInt32BE(body.length); bytes.copy(result, 4); result.writeUInt32BE(crc32(bytes), result.length - 4); return result;
  };
  const paeth = (a,b,c) => { const p=a+b-c, ds=[Math.abs(p-a),Math.abs(p-b),Math.abs(p-c)]; return [a,b,c][ds.indexOf(Math.min(...ds))]; };
  for (const channels of [3,4]) for (const filter of [0,1,2,3,4]) {
    const width=17,height=5,stride=width*channels, raw=Buffer.from(Array.from({length:stride*height},(_,i)=>(i*71+i%13)&255)), rows=Buffer.alloc((stride+1)*height);
    for(let y=0;y<height;y++) {
      rows[y*(stride+1)]=filter;
      for(let x=0;x<stride;x++) {
        const i=y*stride+x,a=x>=channels?raw[i-channels]:0,b=y?raw[i-stride]:0,c=y&&x>=channels?raw[i-stride-channels]:0;
        const predict=[0,a,b,Math.floor((a+b)/2),paeth(a,b,c)][filter];rows[y*(stride+1)+1+x]=(raw[i]-predict)&255;
      }
    }
    const header=Buffer.alloc(13);header.writeUInt32BE(width);header.writeUInt32BE(height,4);header[8]=8;header[9]=channels===3?2:6;
    const png=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',header),chunk('IDAT',deflateSync(rows)),chunk('IEND',Buffer.alloc(0))]);
    const decoded=decodePng(png.toString('base64'));
    for(let p=0;p<width*height;p++) assert.deepEqual([...decoded.pixels.subarray(p*4,p*4+4)],[raw[p*channels],raw[p*channels+1],raw[p*channels+2],channels===4?raw[p*channels+3]:255]);
    png[33]^=1;assert.throws(()=>decodePng(png.toString('base64')),/corrupt|frame/);
  }
});

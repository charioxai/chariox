// MD-5: inspect decoded pixels, not a success string from the capture helper.
import test from "node:test";
import assert from "node:assert/strict";
import { inflateSync, deflateSync, crc32 } from "node:zlib";
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
test("MD-5: failed/unknown layout masks the full frame without exposing capture", async () => {
  let captures = 0;
  const result = await captureProtectedPage({ ensureConnection: async () => { throw new Error("synthetic-only-secret"); } }, { target_id: "target" }, ["synthetic-only-secret"], [], async () => { captures++; return "raw-pixels"; });
  assert.equal(result, wholeFrameMask());
  assert.equal(captures, 0);
  const frame = decoded(result);
  assert.deepEqual([frame.width, frame.height], [1280, 800]);
  assert.deepEqual(frame.pixel(0, 0), [0, 0, 0, 255]);
  assert.deepEqual(frame.pixel(1279, 799), [0, 0, 0, 255]);
  assert.equal(await captureProtectedPage({}, {}, [], [], async () => "ordinary"), "ordinary");
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

// MP-08/MP-10/MP-11: indexed PNGs decode exactly or not at all.
test('MP-11 indexed PNG decoding rejects out-of-palette indices, transparency and a missing palette', async () => {
  const { deflateSync, crc32 } = await import('node:zlib');
  const chunk = (type, data) => { const body = Buffer.concat([Buffer.from(type), data]), out = Buffer.alloc(body.length + 8); out.writeUInt32BE(data.length); body.copy(out, 4); out.writeUInt32BE(crc32(body), out.length - 4); return out; };
  const header = Buffer.alloc(13); header.writeUInt32BE(2); header.writeUInt32BE(1, 4); header[8] = 8; header[9] = 3;
  const png = (...chunks) => Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header), ...chunks, chunk('IEND', Buffer.alloc(0))]).toString('base64');
  const data = indices => chunk('IDAT', deflateSync(Buffer.from([0, ...indices])));
  const palette = chunk('PLTE', Buffer.from([1, 2, 3, 4, 5, 6]));
  assert.deepEqual([...decodePng(png(palette, data([1, 0]))).pixels], [4, 5, 6, 255, 1, 2, 3, 255]);
  assert.throws(() => decodePng(png(palette, data([2, 0]))), /palette index/);
  assert.throws(() => decodePng(png(palette, chunk('tRNS', Buffer.from([0])), data([0, 0]))), /unsupported frame format/);
  assert.throws(() => decodePng(png(data([0, 0]))), /chunk order/);
});

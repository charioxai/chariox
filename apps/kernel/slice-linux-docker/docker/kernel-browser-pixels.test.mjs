// MD-5: inspect decoded pixels, not a success string from the capture helper.
import test from "node:test";
import assert from "node:assert/strict";
import { inflateSync } from "node:zlib";
import { encodePng, maskPng, captureProtectedPage, wholeFrameMask } from "./kernel-browser-pixels.mjs";

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

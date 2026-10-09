// MD-5: bounded CDP PNG masking, using the Room's trusted region locator.
// Unsupported/racing layout receives an opaque whole-frame mask. Page code
// never participates in drawing/removing the masks. No desktop dependency.
import { deflateSync, inflateSync } from "node:zlib";
import { locateBrowserRegions } from "./browser-observation-regions.mjs";

const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const crcTable = Uint32Array.from({ length: 256 }, (_, byte) => {
  let value = byte;
  for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ ((value & 1) ? 0xedb88320 : 0);
  return value >>> 0;
});
function crc(bytes) {
  let value = 0xffffffff;
  for (const byte of bytes) value = (value >>> 8) ^ crcTable[(value ^ byte) & 255];
  return (value ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const body = Buffer.concat([Buffer.from(type), data]);
  const result = Buffer.alloc(body.length + 8);
  result.writeUInt32BE(data.length); body.copy(result, 4); result.writeUInt32BE(crc(body), result.length - 4);
  return result;
}
export function encodePng(width, height, pixels) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 6;
  const rows = Buffer.alloc(height * (width * 4 + 1));
  for (let y = 0; y < height; y++) pixels.copy(rows, y * (width * 4 + 1) + 1, y * width * 4, (y + 1) * width * 4);
  return Buffer.concat([signature, chunk("IHDR", header), chunk("IDAT", deflateSync(rows)), chunk("IEND", Buffer.alloc(0))]).toString("base64");
}
export function opaqueFrame(width = 1280, height = 800) {
  const pixels = Buffer.alloc(width * height * 4);
  for (let i = 3; i < pixels.length; i += 4) pixels[i] = 255;
  return encodePng(width, height, pixels);
}
const maskedFrame = opaqueFrame();
export const wholeFrameMask = () => maskedFrame;
function paeth(a, b, c) {
  const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}
export function decodePng(data, scale = 1) {
  const png = Buffer.from(data, "base64");
  if (!png.subarray(0, 8).equals(signature) || png.length > 4 * 1024 * 1024) throw new Error("MD-5: unsupported frame");
  let width, height, channels, ended = false;
  const compressed = [];
  for (let offset = 8; offset + 12 <= png.length;) {
    const length = png.readUInt32BE(offset), end = offset + length + 12;
    if (end > png.length) throw new Error("MD-5: truncated frame");
    const type = png.toString("ascii", offset + 4, offset + 8), body = png.subarray(offset + 8, end - 4);
    if (crc(png.subarray(offset + 4, end - 4)) !== png.readUInt32BE(end - 4)) throw new Error("MD-5: corrupt frame");
    if (type === "IHDR") {
      if (body.length !== 13 || width) throw new Error("MD-5: invalid frame header");
      width = body.readUInt32BE(0); height = body.readUInt32BE(4);
      channels = body[9] === 6 ? 4 : body[9] === 2 ? 3 : 0;
      if (!width || !height || width > 1280 * scale || height > 800 * scale || body[8] !== 8 || !channels || body[10] || body[11] || body[12]) throw new Error("MD-5: unsupported frame format");
    } else if (type === "IDAT") {
      if (!width || ended) throw new Error("MD-5: invalid frame chunk order");
      compressed.push(body);
    } else if (type === "IEND") {
      if (body.length || end !== png.length) throw new Error("MD-5: invalid frame end");
      ended = true;
    }
    offset = end;
  }
  if (!width || !compressed.length || !ended) throw new Error("MD-5: missing frame data");
  const stride = width * channels, rows = inflateSync(Buffer.concat(compressed), { maxOutputLength: (stride + 1) * height });
  if (rows.length !== (stride + 1) * height) throw new Error("MD-5: invalid frame size");
  const decoded = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = rows[y * (stride + 1)], source = y * (stride + 1) + 1, offset = y * stride;
    if (filter > 4) throw new Error("MD-5: invalid frame filter");
    if (filter === 0) { rows.copy(decoded, offset, source, source + stride); continue; }
    // MD-DISPLAY-02/04: select the predictor once per row, rather than
    // branching for every HiDPI byte. First-row/left-edge predictors are zero.
    if (filter === 1 || (y === 0 && filter === 4)) {
      for (let x = 0; x < channels; x++) decoded[offset + x] = rows[source + x];
      for (let x = channels; x < stride; x++) decoded[offset + x] = (rows[source + x] + decoded[offset + x - channels]) & 255;
    } else if (filter === 2) {
      if (y === 0) rows.copy(decoded, offset, source, source + stride);
      else for (let x = 0; x < stride; x++) decoded[offset + x] = (rows[source + x] + decoded[offset + x - stride]) & 255;
    } else if (filter === 3) {
      for (let x = 0; x < stride; x++) {
        const i = offset + x, left = x >= channels ? decoded[i - channels] : 0, up = y ? decoded[i - stride] : 0;
        decoded[i] = (rows[source + x] + ((left + up) >>> 1)) & 255;
      }
    } else {
      for (let x = 0; x < channels; x++) decoded[offset + x] = (rows[source + x] + decoded[offset + x - stride]) & 255;
      for (let x = channels; x < stride; x++) {
        const i = offset + x;
        decoded[i] = (rows[source + x] + paeth(decoded[i - channels], decoded[i - stride], decoded[i - stride - channels])) & 255;
      }
    }
  }
  const pixels = channels === 4 ? decoded : Buffer.alloc(width * height * 4);
  if (channels === 3) {
    for (let source = 0, target = 0; source < decoded.length; source += 3, target += 4) {
      pixels[target] = decoded[source]; pixels[target + 1] = decoded[source + 1]; pixels[target + 2] = decoded[source + 2]; pixels[target + 3] = 255;
    }
  }
  return { width, height, pixels };
}
export function maskPixels({width,height,pixels}, regions) {
  if (regions.length > 50_000) throw new Error("MD-5: region limit");
  for (const region of regions) {
    if (!Array.isArray(region) || region.length !== 4 || !region.every(Number.isFinite)) throw new Error("MD-5: invalid region");
    const [x, y, w, h] = region;
    // Match the Room mask's outward rounding/padding.
    for (let py = Math.max(0, Math.floor(y) - 4); py < Math.min(height, Math.ceil(y + h) + 4); py++) {
      for (let px = Math.max(0, Math.floor(x) - 4); px < Math.min(width, Math.ceil(x + w) + 4); px++) {
        const i = (py * width + px) * 4; pixels.fill(0, i, i + 3); pixels[i + 3] = 255;
      }
    }
  }
  return {width,height,pixels};
}
export function maskPng(data, regions, scale = 1) {
  const frame=maskPixels(decodePng(data,scale),regions);
  return encodePng(frame.width,frame.height,frame.pixels);
}
export async function captureProtectedPage(browser, tab, values, targets, capture, scale = 1, clip = null) {
  if (!targets.length && !browser.fillTargets?.size) return capture();
  try {
    const locate = () => locateBrowserRegions(targets.filter(target => target.target_id === tab.target_id), browser, values, { contentTarget: tab.target_id, contentScale: scale });
    const before = await locate(), data = await capture(), after = await locate();
    // Moving/navigating content cannot be bound to this exact frame.
    if (JSON.stringify(before) !== JSON.stringify(after)) throw Error("MP-11: fill target moved during capture");
    return maskPng(data, before.map(([x,y,w,h]) => [(x-(clip?.x??0)*scale)*(clip?.scale??1),(y-(clip?.y??0)*scale)*(clip?.scale??1),w*(clip?.scale??1),h*(clip?.scale??1)]), scale);
  } catch { throw Error("MP-11: fill target capture unavailable; retry"); }
}

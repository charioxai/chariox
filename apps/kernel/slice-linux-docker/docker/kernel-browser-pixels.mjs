// MD-5: bounded CDP PNG masking, using the Room's trusted region locator.
// Unsupported/racing layout receives an opaque whole-frame mask. Page code
// never participates in drawing/removing the masks. No desktop dependency.
import { deflateSync, inflateSync } from "node:zlib";
import { locateBrowserRegions } from "./browser-observation-regions.mjs";

const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
function crc(bytes) {
  let value = 0xffffffff;
  for (const byte of bytes) {
    value ^= byte;
    for (let i = 0; i < 8; i++) value = (value >>> 1) ^ ((value & 1) ? 0xedb88320 : 0);
  }
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
export function maskPng(data, regions) {
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
      if (!width || !height || width > 1280 || height > 800 || body[8] !== 8 || !channels || body[10] || body[11] || body[12]) throw new Error("MD-5: unsupported frame format");
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
  const decoded = Buffer.alloc(stride * height), pixels = Buffer.alloc(width * height * 4);
  for (let y = 0; y < height; y++) {
    const filter = rows[y * (stride + 1)];
    if (filter > 4) throw new Error("MD-5: invalid frame filter");
    for (let x = 0; x < stride; x++) {
      const i = y * stride + x, a = x >= channels ? decoded[i - channels] : 0, b = y ? decoded[i - stride] : 0, c = y && x >= channels ? decoded[i - stride - channels] : 0;
      const predictor = [0, a, b, Math.floor((a + b) / 2), paeth(a, b, c)][filter];
      decoded[i] = (rows[y * (stride + 1) + 1 + x] + predictor) & 255;
    }
    for (let x = 0; x < width; x++) {
      const source = y * stride + x * channels, target = (y * width + x) * 4;
      decoded.copy(pixels, target, source, source + 3); pixels[target + 3] = channels === 4 ? decoded[source + 3] : 255;
    }
  }
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
  return encodePng(width, height, pixels);
}
export async function captureProtectedPage(browser, tab, values, targets, capture) {
  if (!values.length) return capture();
  try {
    const locate = () => locateBrowserRegions(targets.filter(target => target.target_id === tab.target_id), browser, values, { contentTarget: tab.target_id });
    const before = await locate(), data = await capture(), after = await locate();
    // Moving/navigating content cannot be bound to this exact frame.
    if (JSON.stringify(before) !== JSON.stringify(after)) return wholeFrameMask();
    return maskPng(data, before);
  } catch { return wholeFrameMask(); }
}

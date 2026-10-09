import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MP-08/MP-11: shared Vault fill-target masks; unavailable geometry retries.
import { deflateSync, inflateSync, crc32 } from 'node:zlib';
import { locateBrowserRegions } from "./browser-observation-regions.mjs";

// MP-11: preserve the complete viewport policy across protected crop merges.
export const displayFullMaskRegions = Symbol('displayFullMaskRegions');

const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
// MP-08/MP-10/MP-11: native CRC keeps PNG integrity checks off the JS loop.
const crc = crc32;
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
export function opaqueFrame(width = geometry.width, height = geometry.height) {
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
  let width, height, channels, palette = null, ended = false;
  const compressed = [];
  for (let offset = 8; offset + 12 <= png.length;) {
    const length = png.readUInt32BE(offset), end = offset + length + 12;
    if (end > png.length) throw new Error("MD-5: truncated frame");
    const type = png.toString("ascii", offset + 4, offset + 8), body = png.subarray(offset + 8, end - 4);
    if (crc(png.subarray(offset + 4, end - 4)) !== png.readUInt32BE(end - 4)) throw new Error("MD-5: corrupt frame");
    if (type === "IHDR") {
      if (body.length !== 13 || width) throw new Error("MD-5: invalid frame header");
      width = body.readUInt32BE(0); height = body.readUInt32BE(4);
      // MP-08/MP-10: the native exact raster writes indexed PNGs (exact
      // per-RGB palettes, at most 256 colours) as well as RGB/RGBA.
      channels = body[9] === 6 ? 4 : body[9] === 2 ? 3 : body[9] === 3 ? 1 : 0;
      if (!width || !height || width > geometry.width * scale || height > geometry.height * scale || body[8] !== 8 || !channels || body[10] || body[11] || body[12]) throw new Error("MD-5: unsupported frame format");
    } else if (type === "PLTE") {
      if (!width || channels !== 1 || palette || compressed.length || !body.length || body.length % 3 || body.length > 768) throw new Error("MD-5: invalid frame palette");
      palette = Buffer.from(body);
    } else if (type === "tRNS") {
      throw new Error("MD-5: unsupported frame format");
    } else if (type === "IDAT") {
      if (!width || ended || (channels === 1 && !palette)) throw new Error("MD-5: invalid frame chunk order");
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
  if (channels === 1) {
    const colours = palette.length / 3;
    for (let source = 0, target = 0; source < decoded.length; source++, target += 4) {
      const index = decoded[source];
      if (index >= colours) throw new Error("MD-5: invalid frame palette index");
      pixels[target] = palette[index * 3]; pixels[target + 1] = palette[index * 3 + 1]; pixels[target + 2] = palette[index * 3 + 2]; pixels[target + 3] = 255;
    }
  } else if (channels === 3) {
    for (let source = 0, target = 0; source < decoded.length; source += 3, target += 4) {
      pixels[target] = decoded[source]; pixels[target + 1] = decoded[source + 1]; pixels[target + 2] = decoded[source + 2]; pixels[target + 3] = 255;
    }
  }
  return { width, height, pixels };
}
const opaquePixel=Buffer.from([0,0,0,255]);
export function maskPixels({width,height,pixels}, regions) {
  if (regions.length > 50_000) throw new Error("MD-5: region limit");
  for (const region of regions) {
    if (!Array.isArray(region) || region.length !== 4 || !region.every(Number.isFinite)) throw new Error("MD-5: invalid region");
    const [x, y, w, h] = region;
    // Match the Room mask's outward rounding/padding.
    const left=Math.max(0,Math.floor(x)-4),right=Math.min(width,Math.ceil(x+w)+4);
    // MP-08/MP-10/MP-11: opaque recovery can cover4M pixels at DPR2.
    // Fill each admitted row in native code; preserve outward bounds and alpha.
    if(right>left)for(let py=Math.max(0,Math.floor(y)-4);py<Math.min(height,Math.ceil(y+h)+4);py++)
      pixels.fill(opaquePixel,(py*width+left)*4,(py*width+right)*4);
  }
  return {width,height,pixels};
}
export function maskPng(data, regions, scale = 1) {
  const frame=maskPixels(decodePng(data,scale),regions);
  return encodePng(frame.width,frame.height,frame.pixels);
}
// MP-08/MP-11: never mutate a leased raster or let its unmasked shared file
// bypass protection in an encoder/native tile. Only masked bytes leave here.
export const displayMaskRegions = Symbol('kernel display protection');
// Private transform capability, never serialized to a client. The encoder
// masks a COW mapping before its independent input/output privacy guards.
export const displayNativeMaskSource = Symbol('kernel native mask source');
export function maskNativeRaster(raw, regions, previousRegions, previous) {
  if(!regions.length)return raw;
  const {width,height}=raw;
  const stable=previousRegions&&JSON.stringify(previousRegions)===JSON.stringify(regions);
  if(raw.shared&&typeof raw.copyPixels==='function'&&typeof raw.readRegion==='function'){
    const masks=regions.map(r=>Object.freeze({...r}));
    if(masks.length>50000||masks.some(r=>![r.x,r.y,r.width,r.height].every(Number.isFinite)||r.width<0||r.height<0))throw Error('MP-11: native mask geometry');
    Object.freeze(masks);
    let snapshot;
    const result={...raw,[displayMaskRegions]:masks,damage:stable?raw.damage:[0,0,width,height],damage_tiles:stable?raw.damage_tiles:null,adjacent_damage_tiles:stable?raw.adjacent_damage_tiles:null,shift_adjacent:null,signature:`masked-${raw.serial}`,
      retain:()=>raw.retain(),release:()=>raw.release()};
    delete result.shared;delete result.copyPixels;
    Object.defineProperties(result,{
      [displayNativeMaskSource]:{value:raw.shared,enumerable:true},
      pixels:{get:()=>snapshot??=maskPixels({width,height,pixels:raw.copyPixels()},masks.map(r=>[r.x,r.y,r.width,r.height])).pixels},
      readRegion:{value:(x,y,w,h)=>maskPixels({width:w,height:h,pixels:raw.readRegion(x,y,w,h)},masks.map(r=>[r.x-x,r.y-y,r.width,r.height])).pixels},
    });
    raw.retain(); // wrapper owns a lease independently of its capture caller
    return result;
  }
  let pixels;
  const box=raw.damage;
  // MP-08/MP-10/MP-11: these bounds come from the complete native byte
  // comparison. Reuse only a contiguous, equally masked, immutable base.
  // Region reads stay below the existing32K-pixel private read bound.
  if(stable&&previous?.width===width&&previous.height===height&&raw.serial===previous.serial+1&&
     Buffer.isBuffer(previous.pixels)&&typeof raw.readRegion==='function'&&Array.isArray(box)&&box.length===4&&
     box.every(Number.isSafeInteger)&&box[0]>=0&&box[1]>=0&&box[2]>box[0]&&box[3]>box[1]&&box[2]<=width&&box[3]<=height&&
     (box[2]-box[0])*(box[3]-box[1])<=32768){
    const [left,top,right,bottom]=box,w=right-left,h=bottom-top,data=raw.readRegion(left,top,w,h);
    if(!Buffer.isBuffer(data)||data.length!==w*h*4)throw Error('MP-11: protected damage read bound');
    pixels=Buffer.from(previous.pixels);
    for(let y=0;y<h;y++)data.copy(pixels,((top+y)*width+left)*4,y*w*4,(y+1)*w*4);
  }else pixels=raw.copyPixels?.()??Buffer.from(raw.pixels);
  const frame=maskPixels({width,height,pixels},regions.map(r=>[r.x,r.y,r.width,r.height]));
  // MP-08/MP-10/MP-11: scheduling hint over protected pixels only. This is
  // never an attestation or an equality proof: stripes compare exact bytes,
  // and native exact damage must ship even when this hint collides.
  // Identical trusted masks cannot change a pixel outside the actual readback
  // damage. First masks, moved masks and opaque recovery still require a full
  // repair; the capture owner supplies the preceding admitted mask geometry.
  const damage=stable?raw.damage:[0,0,width,height];
  // MP-08/MP-10: native serials are scheduling hints, never pixel equality.
  // Exact tiles/refinement compare admitted bytes, and stripes compare every
  // row before omitting it. Avoid hashing another16MiB on the input loop.
  const hint=Number.isSafeInteger(raw.serial)&&raw.serial>0?`masked-${raw.serial}`:crc32(frame.pixels).toString(16);
  const result={...raw,...frame,[displayMaskRegions]:regions,damage,shift_adjacent:null,signature:hint,retain(){},release(){}};
  delete result.shared;delete result.readRegion;delete result.copyPixels;
  return result;
}
// MP-08/MP-11: crop/thumbnail only an already-protected viewport. The display
// thumbnail is a damage hint; exact crops preserve every admitted native pixel.
export function cropProtectedPng(data,clip,scale=1){
  if(!clip)return {data_base64:data,width:geometry.width*scale,height:geometry.height*scale};
  if(clip.width===geometry.width&&clip.height===geometry.height&&(clip.scale??1)===1)return {data_base64:data,width:geometry.width*scale,height:geometry.height*scale};
  const frame=decodePng(data,scale),factor=clip.scale??1;
  const full=clip.width===geometry.width&&clip.height===geometry.height;
  const left=full?0:Math.round(clip.x*scale),top=full?0:Math.round(clip.y*scale);
  const width=Math.round(clip.width*scale*factor),height=Math.round(clip.height*scale*factor);
  if(!Number.isFinite(factor)||factor<=0||factor>1||![left,top,width,height].every(Number.isSafeInteger)||left<0||top<0||width<1||height<1||left+clip.width*scale>frame.width||top+clip.height*scale>frame.height)throw Error('MP-11: protected crop bounds');
  const pixels=Buffer.alloc(width*height*4);
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){
    const source=((top+Math.floor(y/factor))*frame.width+left+Math.floor(x/factor))*4;
    frame.pixels.copy(pixels,(y*width+x)*4,source,source+4);
  }
  return {width,height,data_base64:encodePng(width,height,pixels)};
}
export async function captureProtectedPage(browser, tab, values, targets, capture, scale = 1, clip = null, onMaskedRegions = () => {}) {
  if (!targets.length && !browser.fillTargets?.size) return capture();
  try {
    const locate = () => locateBrowserRegions(targets.filter(target => target.target_id === tab.target_id), browser, values, { contentTarget: tab.target_id, contentScale: scale });
    const before = await locate(), data = await capture(), after = await locate();
    // Moving/navigating content cannot be bound to this exact frame.
    if (JSON.stringify(before) !== JSON.stringify(after)) throw Error("MP-11: fill target moved during capture");
    const masked=maskPng(data, before.map(([x,y,w,h]) => [(x-(clip?.x??0)*scale)*(clip?.scale??1),(y-(clip?.y??0)*scale)*(clip?.scale??1),w*(clip?.scale??1),h*(clip?.scale??1)]), scale);
    onMaskedRegions(before);
    return masked;
  } catch { throw Error("MP-11: fill target capture unavailable; retry"); }
}

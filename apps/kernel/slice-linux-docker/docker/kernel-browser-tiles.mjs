// MD-DISPLAY-02/04: immutable native-pixel damage and bounded exact tiles.
import {encodePng} from "./kernel-browser-pixels.mjs";
export function dirtyTiles(previous, current, region = null, all = false) {
  const tiles = [];
  if (!all && (!previous || previous.width !== current.width || previous.height !== current.height)) return tiles;
  const { width, height, pixels } = current;
  // Small native damage gets small exact tiles, reducing collateral text and
  // narrow-link serialization. Whole-screen repair retains bounded128px tiles.
  const block = !all && region && region.width<=256 && region.height<=256 && region.width*region.height<=32768 ? 32 : 128;
  const left = region ? Math.floor(region.x / block) * block : 0;
  const top = region ? Math.floor(region.y / block) * block : 0;
  const right = region ? Math.min(width, region.x + region.width) : width;
  const bottom = region ? Math.min(height, region.y + region.height) : height;
  for (let y = top; y < bottom; y += block) for (let x = left; x < right; x += block) {
    const w = Math.min(block, width - x), h = Math.min(block, height - y);
    let changed = all;
    for (let row = y; row < y + h && !changed; row++) {
      const offset = (row * width + x) * 4;
      changed = !pixels.subarray(offset, offset + w * 4).equals(previous.pixels.subarray(offset, offset + w * 4));
    }
    if (!changed) continue;
    const data = Buffer.alloc(w * h * 4);
    for (let row = 0; row < h; row++) pixels.copy(data, row * w * 4, ((y + row) * width + x) * 4, ((y + row) * width + x + w) * 4);
    tiles.push({ x, y, width: w, height: h, format: 'png', data_base64: encodePng(w, h, data) });
  }
  return tiles;
}


// MD-DISPLAY-02/04: exact small native damage, only on a contiguous serial
// following a complete exact frame. No approximation or reduced raster.
export function nativeDamageTiles(raw, checkOnly = false, adjacent = false) {
  if(!raw || (raw.shared&&typeof raw.readRegion!=='function') || raw.format!=='bgr0' || !Number.isSafeInteger(raw.width) || !Number.isSafeInteger(raw.height) ||
    raw.width<1 || raw.width>2560 || raw.height<1 || raw.height>1600 ||
    (typeof raw.readRegion==='function'?raw.length:raw.pixels?.length)!==raw.width*raw.height*4 || !Array.isArray(raw.damage) || raw.damage.length!==4) return null;
  // MP-08/MP-10/MP-11: only the native helper's exact byte-proved sparse set.
  if(raw.nativeExact&&(adjacent||Array.isArray(raw.damage_tiles))){
    const tiles=adjacent?raw.adjacent_damage_tiles:raw.damage_tiles;
    if(!Array.isArray(tiles))return null;
    // MP-08/MP-10: Retina quadruples physical pixels, with the same CSS input bound.
    const limit=raw.width===2560&&raw.height===1600?128:32;
    if(!tiles.length||tiles.length>limit||tiles.some(r=>!Array.isArray(r)||r.length!==4||!r.every(Number.isSafeInteger)||r[0]<0||r[1]<0||r[2]<=r[0]||r[3]<=r[1]||r[2]>raw.width||r[3]>raw.height||(r[2]-r[0])*(r[3]-r[1])>1024)||tiles.reduce((n,r)=>n+(r[2]-r[0])*(r[3]-r[1]),0)>limit*1024)return null;
    if(checkOnly)return true;throw Error('MP-11: native sparse tiles require native exact preparation');
  }
  const [left,top,right,bottom]=raw.damage;
  if(!raw.damage.every(Number.isSafeInteger) || left<0 || top<0 || right<=left || bottom<=top ||
    right>raw.width || bottom>raw.height || (right-left)*(bottom-top)>32768) return null;
  if(checkOnly)return true;
  const tiles=[];
  for(let y=Math.floor(top/32)*32;y<bottom;y+=32)for(let x=Math.floor(left/32)*32;x<right;x+=32){
    const width=Math.min(32,raw.width-x),height=Math.min(32,raw.height-y),pixels=Buffer.alloc(width*height*4);
    const region=raw.readRegion?.(x,y,width,height);
    for(let row=0;row<height;row++)for(let col=0;col<width;col++){
      const src=region?(row*width+col)*4:((y+row)*raw.width+x+col)*4,dst=(row*width+col)*4,data=region??raw.pixels;
      pixels[dst]=data[src+2];pixels[dst+1]=data[src+1];pixels[dst+2]=data[src];pixels[dst+3]=255;
    }
    tiles.push({x,y,width,height,format:'png',data_base64:encodePng(width,height,pixels)});
  }
  return tiles;
}

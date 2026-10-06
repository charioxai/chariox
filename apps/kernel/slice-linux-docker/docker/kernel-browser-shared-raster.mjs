// MP-08/MP-10/MP-11: pooled immutable leases over private kernel-owned shm files.
import {readFileSync} from 'node:fs';
import path from 'node:path';
export class SharedRasterPool {
 constructor(root,release){this.root=root;this.release=release;this.slots=new Map();}
 apply(header){
  const {slot,serial,width,height,signature}=header;
  if(!Number.isSafeInteger(slot)||slot<0||slot>=3||this.slots.has(slot)||
     !Number.isSafeInteger(serial)||serial<1||![[1280,800],[2560,1600],[1920,1080]].some(([w,h])=>width===w&&height===h))throw Error('MP-11: shared raster lease bound');
  const shared={path:path.join(this.root,String(slot)),length:width*height*4};
  let refs=1,retired=false,snapshot;
  const raw={...header,shared,length:shared.length,format:'bgr0'};
  delete raw.slot;
  Object.defineProperties(raw,{
   pixels:{get:()=>{if(retired)throw Error('MP-11: retired raster');const bytes=snapshot??=readFileSync(shared.path);if(bytes.length!==shared.length)throw Error('MP-11: shared raster size');return bytes}},
   retain:{value:()=>{if(retired)throw Error('MP-11: retired raster');refs++}},
   release:{value:()=>{if(refs<1)throw Error('MP-11: duplicate raster release');if(--refs===0){retired=true;this.slots.delete(slot);this.release(slot,serial)}}},
  });
  this.slots.set(slot,raw);return raw;
 }
}

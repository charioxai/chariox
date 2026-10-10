// MP-08/MP-10/MP-11: pooled immutable leases over private kernel-owned shm files.
import {readFileSync,openSync,readSync,closeSync,fstatSync,constants} from 'node:fs';
import path from 'node:path';
// MP-08/MP-10: matches the native worker's RASTER_SLOTS.
export const rasterSlots=6;
export class SharedRasterPool {
 constructor(root,release){this.root=root;this.release=release;this.slots=new Map();}
 apply(header){
  const {slot,serial,width,height,signature}=header;
  if(!Number.isSafeInteger(slot)||slot<0||slot>=rasterSlots||this.slots.has(slot)||
     !Number.isSafeInteger(serial)||serial<1||![[1280,800],[2560,1600],[1920,1080]].some(([w,h])=>width===w&&height===h))throw Error('MP-11: shared raster lease bound');
  const shared={path:path.join(this.root,String(slot)),length:width*height*4};
  let refs=1,retired=false,snapshot,descriptor;
  const checkedFile=()=>{
    if(retired)throw Error('MP-11: retired raster');
    if(descriptor===undefined)descriptor=openSync(shared.path,constants.O_RDONLY|constants.O_NOFOLLOW);
    const info=fstatSync(descriptor);
    if(!info.isFile()||info.uid!==process.getuid()||info.mode&0o077||info.size!==shared.length)throw Error('MP-11: shared region owner/size');
    return descriptor;
  };
  const raw={...header,shared,length:shared.length,format:'bgr0'};
  delete raw.slot;
  Object.defineProperties(raw,{
   pixels:{get:()=>{if(retired)throw Error('MP-11: retired raster');const bytes=snapshot??=readFileSync(shared.path);if(bytes.length!==shared.length)throw Error('MP-11: shared raster size');return bytes}},
   // MP-08/MP-10: mask a detached private copy directly. Materializing and
   // then cloning the16MiB attestation cache doubled allocation at DPR2.
   copyPixels:{value:()=>{
    const file=checkedFile(),data=Buffer.allocUnsafe(shared.length);
    let offset=0;while(offset<data.length){const n=readSync(file,data,offset,data.length-offset,offset);if(n<=0)throw Error('MP-11: shared raster truncated');offset+=n;}
    return data;
   }},
   readRegion:{value:(x,y,w,h)=>{
    if(retired)throw Error('MP-11: retired raster');
    if(![x,y,w,h].every(Number.isSafeInteger)||x<0||y<0||w<1||h<1||x+w>width||y+h>height||w*h>32768)throw Error('MP-11: shared region bound');
    checkedFile();
    const data=Buffer.alloc(w*h*4);
    for(let row=0;row<h;row++)if(readSync(descriptor,data,row*w*4,w*4,((y+row)*width+x)*4)!==w*4)throw Error('MP-11: shared region truncated');
    return data;
   }},
   retain:{value:()=>{if(retired)throw Error('MP-11: retired raster');refs++}},
   release:{value:()=>{if(refs<1)throw Error('MP-11: duplicate raster release');if(--refs===0){retired=true;if(descriptor!==undefined)closeSync(descriptor);this.slots.delete(slot);this.release(slot,serial)}}},
  });
  this.slots.set(slot,raw);return raw;
 }
}

// MP-08/MP-10/MP-11: immutable pooled leases and stale release rejection.
import test from 'node:test';import assert from 'node:assert/strict';
import {SharedRasterPool} from './kernel-browser-shared-raster.mjs';
test('MP-08/MP-10 pooled small damage uses bounded exact region reads without a full raster copy',async()=>{
 const {mkdtemp,writeFile,rm,chmod}=await import('node:fs/promises');const {tmpdir}=await import('node:os');const path=await import('node:path');
 const {nativeDamageTiles}=await import('./kernel-browser-tiles.mjs');const {decodePng}=await import('./kernel-browser-pixels.mjs');
 const root=await mkdtemp(path.join(tmpdir(),'chariox-pool-region-'));let raw;
 try{
  const data=Buffer.alloc(1280*800*4);for(let n=0;n<data.length;n+=4){data[n]=32;data[n+1]=64;data[n+2]=128}
  const file=path.join(root,'0');await writeFile(file,data,{mode:0o600});
  raw=new SharedRasterPool(root,()=>{}).apply({slot:0,serial:1,width:1280,height:800,damage:[1248,0,1280,32]});
  assert.equal(nativeDamageTiles(raw,true),true,'leased shared bytes must retain sparse exact eligibility');
  const tiles=nativeDamageTiles(raw);assert.equal(tiles.length,1);
  assert.deepEqual([...decodePng(tiles[0].data_base64).pixels.subarray(0,4)],[128,64,32,255]);
  assert.throws(()=>raw.readRegion(-1,0,32,32),/region/);assert.throws(()=>raw.readRegion(0,0,1280,800),/region/);
  await chmod(file,0o644);assert.throws(()=>raw.readRegion(0,0,32,32),/owner/);
  await chmod(file,0o600);raw.release();assert.throws(()=>raw.readRegion(0,0,32,32),/retired/);raw=null;
 }finally{raw?.release();await rm(root,{recursive:true,force:true})}
});
test('MP-11 writer cannot reuse a slot while any encoder holds its raster',()=>{
 const releases=[];const p=new SharedRasterPool('/private',(slot,serial)=>releases.push([slot,serial]));
 const header={slot:0,serial:1,width:1920,height:1080};const raw=p.apply(header);
 raw.retain();raw.release();assert.throws(()=>p.apply({...header,serial:2}),/lease/);assert.deepEqual(releases,[]);
 raw.release();assert.deepEqual(releases,[[0,1]]);assert.throws(()=>raw.retain(),/retired/);assert.throws(()=>raw.release(),/duplicate/);
 const next=p.apply({...header,serial:2});next.release();assert.deepEqual(releases,[[0,1],[0,2]]);
 assert(!JSON.stringify(raw).includes('pixels'));assert.throws(()=>p.apply({...header,slot:6}),/bound/);
});

test('MP-10 attestation reads one snapshot rather than one file per channel',async()=>{
 const {mkdtemp,writeFile,rm}=await import('node:fs/promises');const {tmpdir}=await import('node:os');const path=await import('node:path');
 const root=await mkdtemp(path.join(tmpdir(),'chariox-pool-test-'));
 try{
  const data=Buffer.alloc(1280*800*4,12);await writeFile(path.join(root,'0'),data,{mode:0o600});
  const raw=new SharedRasterPool(root,()=>{}).apply({slot:0,serial:1,width:1280,height:800});
  const snapshot=raw.pixels;assert.strictEqual(raw.pixels,snapshot);assert.equal(snapshot.length,data.length);raw.release();
  assert.throws(()=>raw.pixels,/retired/);
 }finally{await rm(root,{recursive:true,force:true})}
});

test('MP-08/MP-10/MP-11 private masking copies are detached from the lease and reject changed ownership or retired data',async()=>{
 const {mkdtemp,writeFile,chmod,rm}=await import('node:fs/promises');const {tmpdir}=await import('node:os');const path=await import('node:path');
 const root=await mkdtemp(path.join(tmpdir(),'chariox-mask-copy-'));let raw;
 try{
  const file=path.join(root,'0');await writeFile(file,Buffer.alloc(1280*800*4,17),{mode:0o600});
  raw=new SharedRasterPool(root,()=>{}).apply({slot:0,serial:1,width:1280,height:800});
  const a=raw.copyPixels(),b=raw.copyPixels();a.fill(0);assert.equal(b[0],17);assert.equal(raw.pixels[0],17);
  await chmod(file,0o644);assert.throws(()=>raw.copyPixels(),/owner/);await chmod(file,0o600);
  raw.release();assert.throws(()=>raw.copyPixels(),/retired/);raw=null;
 }finally{raw?.release();await rm(root,{recursive:true,force:true})}
});

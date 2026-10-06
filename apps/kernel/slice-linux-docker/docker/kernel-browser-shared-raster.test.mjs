// MP-08/MP-10/MP-11: immutable pooled leases and stale release rejection.
import test from 'node:test';import assert from 'node:assert/strict';
import {SharedRasterPool} from './kernel-browser-shared-raster.mjs';
test('MP-11 writer cannot reuse a slot while any encoder holds its raster',()=>{
 const releases=[];const p=new SharedRasterPool('/private',(slot,serial)=>releases.push([slot,serial]));
 const header={slot:0,serial:1,width:1920,height:1080};const raw=p.apply(header);
 raw.retain();raw.release();assert.throws(()=>p.apply({...header,serial:2}),/lease/);assert.deepEqual(releases,[]);
 raw.release();assert.deepEqual(releases,[[0,1]]);assert.throws(()=>raw.retain(),/retired/);assert.throws(()=>raw.release(),/duplicate/);
 const next=p.apply({...header,serial:2});next.release();assert.deepEqual(releases,[[0,1],[0,2]]);
 assert(!JSON.stringify(raw).includes('pixels'));assert.throws(()=>p.apply({...header,slot:3}),/bound/);
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

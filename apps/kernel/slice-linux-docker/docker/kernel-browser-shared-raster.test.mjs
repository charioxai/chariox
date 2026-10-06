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

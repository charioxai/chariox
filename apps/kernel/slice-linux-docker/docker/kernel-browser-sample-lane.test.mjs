import test from 'node:test';
import assert from 'node:assert/strict';
import {SampleLane} from './kernel-browser-sample-lane.mjs';
test('MD-DISPLAY physical input waits for viewport restoration, then precedes queued samples',async()=>{
 const lane=new SampleLane(),order=[];let release;
 const held=new Promise(r=>release=r);
 const first=lane.run('capture',async()=>{order.push('temporary-viewport');await held;order.push('restored-viewport')});
 await Promise.resolve();
 const next=lane.run('capture',async()=>order.push('next-sample'));
 const input=lane.run('input',async()=>order.push('physical-input'));
 await Promise.resolve();assert.deepEqual(order,['temporary-viewport']);
 release();await Promise.all([first,next,input]);assert.deepEqual(order,['temporary-viewport','restored-viewport','physical-input','next-sample']);
 await assert.rejects(lane.run('input',async()=>{throw Error('cancelled')}));
 assert.equal(await lane.run('capture',async()=>42),42);
});

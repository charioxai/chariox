import test from 'node:test';
import assert from 'node:assert/strict';
import { readlink } from 'node:fs/promises';
import { shapeViewerLeg } from './drill-netem.mjs';
test('MD-DISPLAY netem refuses the host namespace and invalid profiles before mutation',async()=>{
 const current=await readlink('/proc/self/ns/net');
 await assert.rejects(shapeViewerLeg('wan40','ws://127.0.0.1:1',current),/refuse host namespace/);
 await assert.rejects(shapeViewerLeg('wan80','ws://127.0.0.1:1',undefined),/refuse host namespace/);
 await assert.rejects(shapeViewerLeg('wan40','ws://127.0.0.1:1','spoofed'),/refuse host namespace/);
 await assert.rejects(shapeViewerLeg('invalid','ws://127.0.0.1:1','different'),/invalid netem/);
});

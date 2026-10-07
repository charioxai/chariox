// MP-08/MP-10/MP-11: a long-running Rust test can report status mid-stderr chunk.
import test from 'node:test';
import assert from 'node:assert/strict';
import {KernelLogCapture} from './drill-kernel-log.mjs';

test('kernel timing parsing keeps stdout out of split stderr records',()=>{
 const log=new KernelLogCapture();
 const record={stage:'input_priority_wait',duration_ms:0.001};
 const line='MD-DISPLAY-TIMING '+JSON.stringify(record)+'\n';
 log.record('stderr',Buffer.from(line.slice(0,35)));
 log.record('stdout',Buffer.from('test display has been running for over 60 seconds\n'));
 log.record('stderr',Buffer.from(line.slice(35)));
 assert.deepEqual(log.timings(),[record]);
});

// MP-08/MP-10/MP-11: a long-running Rust test can report status mid-stderr chunk.
import test from 'node:test';
import assert from 'node:assert/strict';
import {KernelLogCapture,summarizePushGate} from './drill-kernel-log.mjs';

test('kernel timing parsing keeps stdout out of split stderr records',()=>{
 const log=new KernelLogCapture();
 const record={stage:'input_priority_wait',duration_ms:0.001};
 const line='MD-DISPLAY-TIMING '+JSON.stringify(record)+'\n';
 log.record('stderr',Buffer.from(line.slice(0,35)));
 log.record('stdout',Buffer.from('test display has been running for over 60 seconds\n'));
 log.record('stderr',Buffer.from(line.slice(35)));
 assert.deepEqual(log.timings(),[record]);
});

test('malformed kernel timing retains failure metadata for receipt and cleanup',()=>{
 const log=new KernelLogCapture();
 log.record('stderr',Buffer.from('MD-DISPLAY-TIMING {"stage":\n'));
 assert.deepEqual(log.timings(),[]);
 assert.deepEqual(log.diagnosticErrors,[{stream:'stderr',line:1,error:'invalid timing JSON'}]);
});

test('MP-08/MP-10/MP-11 closed polls do not inflate closure counts or turn missing instrumentation into zero',()=>{
 const polls=[200,220,240].map(duration_ms=>({stage:'push_oldest_unacked',duration_ms}));
 assert.equal(summarizePushGate(polls).closures,null);
 const summary=summarizePushGate([...polls,{stage:'push_gate_started',duration_ms:0},{stage:'push_gate_closed',duration_ms:200},{stage:'push_gate_reopened',duration_ms:60}]);
 assert.equal(summary.closures,1);assert.equal(summary.reopened,1);assert.deepEqual(summary.oldest_unacked_poll_age_ms,[200,220,240]);
 assert.deepEqual(summary.closed_at_age_ms,[200]);assert.deepEqual(summary.closed_duration_ms,[60]);
 assert.equal(summarizePushGate([{stage:'push_gate_started',duration_ms:0}]).closures,0);
});

// MP-10: retain departed processes and reject PID reuse as an accounting base.
import test from 'node:test';import assert from 'node:assert/strict';
import {CpuCounter,cpuSpan} from './drill-cpu.mjs';
test('MP-10 CPU retains exited encoder consumption across spans and PID reuse',()=>{
 const c=new CpuCounter(100),before={at_ms:0,cpu_seconds:c.observe([{pid:42,start:'1',ticks:100,role:'pipeline'}])};
 c.observe([{pid:42,start:'1',ticks:125,role:'pipeline'}]);c.observe([]);
 const after={at_ms:1000,cpu_seconds:c.observe([{pid:42,start:'2',ticks:50,role:'source'}])};
 assert.deepEqual(cpuSpan(before,after).cores,{source:.5,pipeline:.25,viewer:0,harness:0});
});

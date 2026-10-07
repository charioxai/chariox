// MP-10: positive delta must move down, as in the official upstream viewer.
import test from 'node:test';import assert from 'node:assert/strict';import {wheelMessages} from './baseline-input.mjs';
test('MP-10 native baseline wheel matches upstream direction and bounded magnitude',()=>{
 assert.deepEqual(wheelMessages(700,600,120),['m,700,600,8,1','m,700,600,0,0']);
 assert.deepEqual(wheelMessages(700,600,-240),['m,700,600,16,2','m,700,600,0,0']);
 for(const args of [[NaN,600,120],[1920,600,120],[700,600,0],[700,600,9000]])assert.throws(()=>wheelMessages(...args),/invalid baseline wheel/);
});

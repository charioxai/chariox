// MP-08/MP-10: laptops reserve a proportion; shared builders retain explicit limits.
import test from 'node:test';import assert from 'node:assert/strict';
import {memoryFloorGiB} from './drill-resources.mjs';
test('MP-10 seven GiB laptop does not inherit a builder floor',()=>{assert.equal(memoryFloorGiB(undefined,7*1024**3),1.05);assert.equal(memoryFloorGiB(undefined,64*1024**3),2);assert.equal(memoryFloorGiB('12',64*1024**3),12);assert.equal(memoryFloorGiB(undefined,1024**3),.5);assert.throws(()=>memoryFloorGiB('NaN'),/reserve/)});

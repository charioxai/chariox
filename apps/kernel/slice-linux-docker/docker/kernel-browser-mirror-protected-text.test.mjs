import test from 'node:test';
import assert from 'node:assert/strict';
import {protectedTextScanner} from './kernel-browser-mirror-protected-text.mjs';
test('registered echoes are masked across source nodes, including chunk boundaries',()=>{
 const marked=new WeakSet(),scan=protectedTextScanner(['synthetic-private-value'],marked);
 const nodes=[{data:'x'.repeat(16370)+'synthetic-'},{data:'private-'},{data:'value'},{data:' ordinary'}];
 nodes.forEach(scan);assert.deepEqual(nodes.map(n=>marked.has(n)),[true,true,true,false]);
});
test('long offscreen text is scanned completely without retaining the full page',()=>{
 const marked=new WeakSet(),scan=protectedTextScanner(['synthetic-private-value'],marked);
 const ordinary={data:'x'.repeat(3*1024*1024)},tail={data:'synthetic-private-value'};
 scan(ordinary);scan(tail);assert(!marked.has(ordinary));assert(marked.has(tail));
});
test('overlapping variants mark every contributing node and ignore empty values',()=>{
 const marked=new WeakSet(),scan=protectedTextScanner(['','aba','bab'],marked);
 const nodes=[{data:'a'},{data:'b'},{data:'a'},{data:'b'}];nodes.forEach(scan);assert(nodes.every(n=>marked.has(n)));
 const emptyScan=protectedTextScanner([''],marked),safe={data:'ordinary'};emptyScan(safe);assert(!marked.has(safe));
});
test('an oversized protection dictionary refuses rather than truncates',()=>{
 assert.throws(()=>protectedTextScanner(['x'.repeat(2097153)],new WeakSet()),/dictionary budget/);
});

// MP-08/MP-10/MP-11: public loader diagnostics only.
import {test} from 'node:test';import assert from 'node:assert/strict';import {nativeLoader} from './native-loader.mjs';
test('MP-10: hardware loader prefers the system driver dependency stack',()=>{const r=nativeLoader('/worker','/kit/loader','/kit/lib',()=>'',()=>true);assert.equal(r.mode,'host-vaapi-stack');assert.ok(r.library_path.startsWith('/usr/lib:'));});
test('MP-10: incompatible host records exact loader error before bundled fallback',()=>{const r=nativeLoader('/worker','/kit/loader','/kit/lib',()=>{const e=Error('loader');e.stderr='GLIBC_2.41 not found';throw e;},()=>true);assert.equal(r.mode,'bundled-fallback');assert.match(r.failures[0].error,/GLIBC_2.41/);});

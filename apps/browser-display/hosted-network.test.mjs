// MP-11: supplementary ownership checks; no live acceptance claim.
import test from 'node:test';import assert from 'node:assert/strict';
import {assertHostedNamespace} from './hosted-network.mjs';
test('MP-11 hosted shaping refuses host and foreign namespaces before any mutation',()=>{
 for(const name of [undefined,'','pr893','md-display-123456789abg'])assert.throws(()=>assertHostedNamespace(name,'net:[2]','net:[1]'));
 assert.throws(()=>assertHostedNamespace('md-display-123456789abc','net:[1]','net:[1]'));
 assert.doesNotThrow(()=>assertHostedNamespace('md-display-123456789abc','net:[2]','net:[1]'));
});

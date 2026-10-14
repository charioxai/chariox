// MP-11: supplementary ownership checks; no live acceptance claim.
import test from 'node:test';import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {assertHostedNamespace,hostedTarget} from './hosted-network.mjs';
test('MP-11 hosted shaping refuses host and foreign namespaces before any mutation',()=>{
 for(const name of [undefined,'','pr893','md-display-123456789abg'])assert.throws(()=>assertHostedNamespace(name,'net:[2]','net:[1]'));
 assert.throws(()=>assertHostedNamespace('md-display-123456789abc','net:[1]','net:[1]'));
 assert.doesNotThrow(()=>assertHostedNamespace('md-display-123456789abc','net:[2]','net:[1]'));
});
test('MP-11 hosted drill targets are required configuration, never a pinned preview host',()=>{
 for(const target of [undefined,{},{upstream:'192.0.2.1'},{upstream:'not-an-ip',hosts:['cloud.example.test']},{upstream:'192.0.2.1',hosts:[]},{upstream:'192.0.2.1',hosts:['bad host']}])assert.throws(()=>hostedTarget(target),/MD_HOSTED_UPSTREAM/);
 assert.equal(hostedTarget({upstream:'192.0.2.1',hosts:['cloud.example.test','relay.example.test']}),'MAP cloud.example.test 127.0.0.1, MAP relay.example.test 127.0.0.1');
 const run=spawnSync(process.execPath,[new URL('./hosted-drill.mjs',import.meta.url).pathname,'/nonexistent-md-output','/nonexistent-md-tools'],{env:{PATH:process.env.PATH},encoding:'utf8'});
 assert.notEqual(run.status,0);assert.match(run.stderr,/MD_HOSTED_ORIGIN is required/);
});

// MP-08 / MP-10 / MP-11: reject repository-local runtime state.
import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {repository} from './paths.mjs';
test('rejects source checkout as asset root before reading build manifest',()=>{
 const run=spawnSync(process.execPath,['--input-type=module','-e',"const {verifyBuild,repository}=await import("+JSON.stringify(new URL('./paths.mjs',import.meta.url).href)+");await verifyBuild('test',repository);"],{cwd:repository,encoding:'utf8',env:{...process.env,CTXVAL_LANE:repository}});
 assert.notEqual(run.status,0);assert.match(run.stderr,/separate from source checkouts/);
});

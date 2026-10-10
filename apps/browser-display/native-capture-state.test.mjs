// MP-08/MP-10/MP-11: exercise the actual C readback/admission state machine.
// Native headers/libs are explicit; this supplementary test opens no X server.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

test('MP-11 native readback preserves byte-proved bases across dropped and stale admissions',{
 skip:process.platform!=='linux'||!process.env.CHARIOX_NATIVE_DISPLAY_INCLUDE||!process.env.CHARIOX_NATIVE_DISPLAY_LIB,
},async()=>{
 const root=await mkdtemp(path.join(tmpdir(),'chariox-capture-state-'));
 const binary=path.join(root,'capture-state');
 const source=fileURLToPath(new URL('../kernel/src/display_native/capture.test.c',import.meta.url));
 const paths=(key,flag)=>process.env[key].split(path.delimiter).flatMap(p=>[flag,p]);
 try{
  const compile=spawnSync(process.env.CC||'cc',['-O2',...paths('CHARIOX_NATIVE_DISPLAY_INCLUDE','-I'),...paths('CHARIOX_NATIVE_DISPLAY_LIB','-L'),source,'-o',binary,'-lX11','-lXext','-lXdamage','-lXcomposite','-lXtst'],{encoding:'utf8'});
  assert.equal(compile.status,0,compile.stderr||compile.error?.message);
  const run=spawnSync(binary,[],{encoding:'utf8'});
  assert.equal(run.status,0,run.stderr||run.error?.message);
  assert.match(run.stdout,/row comparisons after current admission: 800 \(expected 800\)/);
 }finally{await rm(root,{recursive:true,force:true})}
});

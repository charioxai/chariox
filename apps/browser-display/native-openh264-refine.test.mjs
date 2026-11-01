// MP-08/MP-10: run the production OpenH264 rate model against the installed
// Cisco binary (CHARIOX_BROWSER_DISPLAY_OPENH264). No libav or X server needed.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

test('MP-08/MP-10 a page IDR spends its budget and the unchanged picture refines at the rate',{
 skip:process.platform!=='linux'||!process.env.CHARIOX_BROWSER_DISPLAY_OPENH264,
},async()=>{
 const root=await mkdtemp(path.join(tmpdir(),'chariox-openh264-refine-'));
 const binary=path.join(root,'refine');
 const source=fileURLToPath(new URL('../kernel/src/display_native/openh264-refine.test.c',import.meta.url));
 try{
  const compile=spawnSync(process.env.CC||'cc',['-O2',source,'-o',binary,'-ldl','-lm','-pthread'],{encoding:'utf8'});
  assert.equal(compile.status,0,compile.stderr||compile.error?.message);
  const run=spawnSync(binary,[],{encoding:'utf8'});
  assert.equal(run.status,0,`${run.stdout}${run.stderr}`);
  assert.match(run.stdout,/page IDR budget and refinement PASS/);
 }finally{await rm(root,{recursive:true,force:true})}
});

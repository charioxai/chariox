// MP-11 (review #893 P2): native pointer input on a product-owned private X server.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {OwnedDisplay} from '../kernel/slice-linux-docker/docker/kernel-browser-owned-display.mjs';

test('MP-11 native pointer input reaches only the uncovered owned window',{
 skip:process.platform!=='linux'||!process.env.CHARIOX_NATIVE_DISPLAY_INCLUDE||!process.env.CHARIOX_NATIVE_DISPLAY_LIB,
},async()=>{
 const root=await mkdtemp(path.join(tmpdir(),'chariox-capture-pointer-'));
 const display=new OwnedDisplay(root),binary=path.join(root,'capture-pointer');
 const source=fileURLToPath(new URL('../kernel/src/display_native/capture.c',import.meta.url));
 const probe=fileURLToPath(new URL('../kernel/src/display_native/capture-pointer.test.c',import.meta.url));
 const paths=(key,flag)=>process.env[key].split(path.delimiter).flatMap(p=>[flag,p]);
 try{
  const compile=spawnSync(process.env.CC||'cc',['-O2',...paths('CHARIOX_NATIVE_DISPLAY_INCLUDE','-I'),...paths('CHARIOX_NATIVE_DISPLAY_LIB','-L'),'-DCAPTURE_SOURCE="'+source+'"',probe,'-o',binary,'-lX11','-lXext','-lXdamage','-lXcomposite','-lXtst'],{encoding:'utf8'});
  assert.equal(compile.status,0,compile.stderr||compile.error?.message);
  const environment=await display.start();
  const run=spawnSync(binary,[],{encoding:'utf8',env:{...process.env,...environment},timeout:10000});
  assert.equal(run.status,0,run.stderr||run.error?.message);
  assert.match(run.stdout,/reaches only the uncovered owned window/);
 }finally{await display.close();await rm(root,{recursive:true,force:true})}
});

// MP-08/MP-11: capture the root of an actual owned desktop X server (owner 0).
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {spawn,spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {setTimeout as delay} from 'node:timers/promises';

test('MP-08/MP-11 desktop-mode capture reads the owned desktop root and refuses native input',{
 skip:process.platform!=='linux'||!process.env.CHARIOX_NATIVE_DISPLAY_INCLUDE||!process.env.CHARIOX_NATIVE_DISPLAY_LIB,
},async()=>{
 const root=await mkdtemp(path.join(tmpdir(),'chariox-capture-desktop-')),binary=path.join(root,'capture-desktop');
 const source=fileURLToPath(new URL('../kernel/src/display_native/capture.c',import.meta.url));
 const probe=fileURLToPath(new URL('../kernel/src/display_native/capture-desktop.test.c',import.meta.url));
 const paths=(key,flag)=>process.env[key].split(path.delimiter).flatMap(p=>[flag,p]);
 // Same geometry as the product's owned desktop (linux-owned-desktop.mjs).
 const x=spawn('/usr/bin/Xvfb',['-displayfd','3','-screen','0','1280x800x24','-nolisten','tcp'],{stdio:['ignore','ignore','ignore','pipe']});
 let text='';x.stdio[3].on('data',b=>{text+=b});
 try{
  const compile=spawnSync(process.env.CC||'cc',['-O2',...paths('CHARIOX_NATIVE_DISPLAY_INCLUDE','-I'),...paths('CHARIOX_NATIVE_DISPLAY_LIB','-L'),'-DCAPTURE_SOURCE="'+source+'"',probe,'-o',binary,'-lX11','-lXext','-lXdamage','-lXcomposite','-lXtst'],{encoding:'utf8'});
  assert.equal(compile.status,0,compile.stderr||compile.error?.message);
  for(let n=0;n<100&&!/^\d+\n$/.test(text);n++)await delay(20);
  assert.match(text,/^\d+\n$/);
  const run=spawnSync(binary,[],{encoding:'utf8',env:{...process.env,DISPLAY:':'+text.trim()},timeout:10000});
  assert.equal(run.status,0,run.stderr||run.error?.message);
  assert.match(run.stdout,/owned desktop root capture reads every window and refuses native input/);
 }finally{if(Number.isSafeInteger(x.pid)&&x.pid>1&&x.exitCode===null)x.kill('SIGTERM');await rm(root,{recursive:true,force:true})}
});

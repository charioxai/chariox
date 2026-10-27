// MP-08 / MP-10 / MP-11: supplementary literal provider-command boundary.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,access,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {providerNetwork} from './provider-proxy.mjs';
test('official executable path cannot become shell syntax',async()=>{
 const root=await mkdtemp(join(tmpdir(),'context-proxy-')),marker=root+'/injected';let network;
 try{
  const executable=root+"/provider '$(touch injected)'";
  await writeFile(executable,'#!/bin/sh\nprintf "%s\\n" "$@"\n',{mode:0o700});const env={CHARIOX_CLAUDE_BIN:executable};
  network=await providerNetwork({root,env,provider:'claude',record:async()=>{}});
  const result=spawnSync(env.CHARIOX_CLAUDE_BIN,['literal $ argument'],{cwd:root,encoding:'utf8'});
  assert.equal(result.status,0,result.stderr);assert.equal(result.stdout,'literal $ argument\n');await assert.rejects(access(marker),{code:'ENOENT'});
 }finally{await network?.close();await rm(root,{recursive:true,force:true})}
});

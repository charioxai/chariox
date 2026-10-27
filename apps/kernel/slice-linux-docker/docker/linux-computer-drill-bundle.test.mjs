// MP-08 / MP-10 / MP-11: load the standalone runner's copied module graph.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,copyFile,mkdir,rm} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {pathToFileURL,fileURLToPath} from 'node:url';

test('MP-11 standalone Computer drill bundles every imported accessibility dependency',async()=>{
 const source=fileURLToPath(new URL('../',import.meta.url));
 const names=JSON.parse(execFileSync('python3',['-c',`
import importlib.util,json,sys
from pathlib import Path
source=Path(sys.argv[1])
spec=importlib.util.spec_from_file_location('drill_helper_closure',source/'drill-helper-closure.py')
closure=importlib.util.module_from_spec(spec);spec.loader.exec_module(closure)
print(json.dumps(closure.default_helpers(source/'docker')))
`,source],{encoding:'utf8'}));
 const root=await mkdtemp('/var/tmp/culinux-module-graph-');
 try {
  await mkdir(root+'/docker');
  for(const name of names)await copyFile(new URL(name,import.meta.url),root+'/docker/'+name);
  for(const name of ['native-accessibility.mjs','native-computer.mjs','linux-owned-desktop.mjs']){
   const url=pathToFileURL(root+'/docker/'+name).href;
   execFileSync(process.execPath,['--input-type=module','-e','await import(process.argv[1])',url],{stdio:'pipe',timeout:10000});
  }
  assert(names.includes('kernel-browser-refusal.mjs'));
  assert(names.includes('linux-desktop-session.py'), 'MP-08 / MP-11 detached desktop supervisor must ship in the standalone drill');
 } finally {await rm(root,{recursive:true,force:true});}
});

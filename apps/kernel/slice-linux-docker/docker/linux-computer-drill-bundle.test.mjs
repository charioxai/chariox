// MP-08 / MP-10 / MP-11: load the standalone runner's copied module graph.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,readFile,copyFile,mkdir,rm} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';

test('MP-11 standalone Computer drill bundles every imported accessibility dependency',async()=>{
 const source=new URL('../linux-computer-local-drill.py',import.meta.url);
 const inventory=(await readFile(source,'utf8')).split('head=subprocess')[0].split('files=')[1];
 const names=[...inventory.matchAll(/'([^']+\.(?:mjs|py))'/g)].map(match=>match[1]);
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

// The drill loads Python helpers by file name (`with_name('x.py')`, `load('x')`) at import or first use, so a missing one only fails mid-run.
test('MP-11 standalone Computer drill ships the transitive Python helper closure of its default file list',async()=>{
 const source=new URL('../linux-computer-local-drill.py',import.meta.url);
 const inventory=(await readFile(source,'utf8')).split('head=subprocess')[0].split('files=')[1];
 const shipped=new Set([...inventory.matchAll(/'([^']+\.(?:mjs|py))'/g)].map(match=>match[1]));
 const missing=[],pending=[...shipped].filter(name=>name.endsWith('.py')),seen=new Set(pending);
 for(const name of pending){
  const text=await readFile(new URL(name,import.meta.url),'utf8');
  for(const [,helper] of [...text.matchAll(/with_name\('([^']+\.py)'\)/g),...text.matchAll(/\bload\('([^']+)'\)/g).map(match=>[0,match[1]+'.py'])]){
   if(!shipped.has(helper)&&!missing.includes(helper))missing.push(helper);
   if(!seen.has(helper)){seen.add(helper);pending.push(helper);}
  }
 }
 assert.deepEqual(missing,[]);
});

// MP-08 / MP-11: the native kernel ships a runnable controller outside checkout.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,mkdtemp,writeFile,rm} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import os from 'node:os';
test('MP-08 embedded native controller answers real health without source modules',async()=>{
 const root=await mkdtemp(path.join(os.tmpdir(),'culinux-asset-test-'));
 try {
  const inventory=await readFile(new URL('../../src/runtime/kernel_browser_assets.rs',import.meta.url),'utf8');
  const names=[...inventory.matchAll(/include_bytes!\("\.\.\/\.\.\/slice-linux-docker\/docker\/([^"\n]+)"\)/g)].map(m=>m[1]);
  assert(names.includes('kernel-browser-host.mjs'));
  for(const name of names)await writeFile(path.join(root,name),await readFile(new URL(name,import.meta.url)));
  const reply=execFileSync(process.execPath,[path.join(root,'kernel-browser-host.mjs'),'stdio',path.join(root,'profile')],{input:JSON.stringify({id:1,method:'health',params:{}})+'\n',encoding:'utf8',timeout:10000});
  assert.equal(JSON.parse(reply.trim()).result.state,'ready');
 }finally{await rm(root,{recursive:true,force:true});}
});


test('MP-08 slice image native accessibility module graph imports outside checkout',async()=>{
 const root=await mkdtemp(path.join(os.tmpdir(),'culinux-image-assets-'));
 try {
  const dockerfile=await readFile(new URL('Dockerfile',import.meta.url),'utf8');
  const names=new Set([...dockerfile.matchAll(/apps\/kernel\/slice-linux-docker\/docker\/([A-Za-z0-9._-]+\.(?:mjs|py))/g)].map(m=>m[1]));
  for(const name of names)await writeFile(path.join(root,name),await readFile(new URL(name,import.meta.url)));
  execFileSync(process.execPath,['--input-type=module','-e',`await import(${JSON.stringify(new URL('file://'+root+'/native-accessibility.mjs').href)})`],{timeout:10000});
 }finally{await rm(root,{recursive:true,force:true});}
});

test('MP-11 #904 review 2 slice image copies every Python helper its installed helpers load',async()=>{
 const dockerfile=await readFile(new URL('Dockerfile',import.meta.url),'utf8');
 const copied=new Set([...dockerfile.matchAll(/apps\/kernel\/slice-linux-docker\/docker\/([A-Za-z0-9._-]+\.py)/g)].map(m=>m[1]));
 const missing=new Set();
 for(const name of copied){
  const source=await readFile(new URL(name,import.meta.url),'utf8');
  const siblings=[...source.matchAll(/with_name\(\s*['"]([A-Za-z0-9._-]+\.py)['"]\s*\)/g)].map(m=>m[1]).concat([...source.matchAll(/\bload\(\s*'([A-Za-z0-9._-]+)'\s*\)/g)].map(m=>m[1]+'.py'));
  for(const sibling of siblings)if(!copied.has(sibling))missing.add(name+' -> '+sibling);
 }
 assert.deepEqual([...missing],[]);
});

test('MP-08 #904 review 5 native kernel asset inventory has no duplicate entries',async()=>{
 const inventory=await readFile(new URL('../../src/runtime/kernel_browser_assets.rs',import.meta.url),'utf8');
 const names=[...inventory.matchAll(/include_bytes!\("\.\.\/\.\.\/slice-linux-docker\/docker\/([^"\n]+)"\)/g)].map(m=>m[1]);
 assert.deepEqual(names.filter((name,index)=>names.indexOf(name)!==index),[]);
});

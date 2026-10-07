// MP-08/MP-11: bundled product host imports must remain closed outside checkout.
import test from 'node:test';import assert from 'node:assert/strict';import {readFile} from 'node:fs/promises';
test('MP-08/MP-11 kernel bundled controller assets include every relative static import',async()=>{
 const inventory=await readFile(new URL('../kernel/src/runtime/kernel_browser_assets.rs',import.meta.url),'utf8');
 const names=new Set([...inventory.matchAll(/include_bytes!\("[^"\n]+\/([^"/]+)"\)/g)].map(m=>m[1]));
 assert(names.has('kernel-browser-host.mjs'));
 for(const name of names){
  if(!name.endsWith('.mjs'))continue;
  const source=await readFile(new URL('../kernel/slice-linux-docker/docker/'+name,import.meta.url),'utf8');
  for(const match of source.matchAll(/(?:from\s*|import\s*)['"]\.\/([^'"]+)['"]/g))assert(names.has(match[1]),`${name} imports missing bundled ${match[1]}`);
 }
});

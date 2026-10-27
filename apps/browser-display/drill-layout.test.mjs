// MP-08/MP-10/MP-11: the real local drill viewer must load its whole module graph.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

test('MP-10: local display drill serves every relative viewer module import',async()=>{
 const runner=await readFile(new URL('./drill.mjs',import.meta.url),'utf8');
 const routes=runner.match(/const assets=\{([^}]+)\}/)?.[1]??'';
 const served=new Set([...routes.matchAll(/'\/([^']+)':'[^']+'/g)].map(match=>match[1]));
 const pending=['harness.html'],seen=new Set();
 while(pending.length){
  const name=pending.pop();
  if(seen.has(name))continue;
  seen.add(name);
  assert(served.has(name),`MP-10: drill viewer module is not served: ${name}`);
  const source=await readFile(new URL('./'+name,import.meta.url),'utf8');
  for(const match of source.matchAll(/(?:from\s*|import\s*)['"]\.\/([^'"]+)['"]/g))pending.push(match[1]);
 }
 assert(seen.has('presenter.mjs'),'MP-10: start at the real harness entry');
});

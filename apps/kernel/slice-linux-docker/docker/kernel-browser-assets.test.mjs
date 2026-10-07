// MP-08 / MP-11: the native kernel ships a runnable controller outside checkout.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,mkdtemp,writeFile,rm} from 'node:fs/promises';
import {execFileSync,spawnSync} from 'node:child_process';
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

// MP-08 / MP-10 / MP-11: native bundle must load the shared protection modules.
test("MP-08 / MP-10: materialized native controller imports without a checkout", async () => {
  const manifest = fileURLToPath(new URL("../../src/runtime/kernel_browser_assets.rs", import.meta.url));
  const source = await readFile(manifest, "utf8");
  const root = await mkdtemp(path.join(os.tmpdir(), "appsbudget-assets-"));
  try {
    const entries = [...source.matchAll(/\(\s*"([^"]+)"\s*,\s*include_bytes!\("([^"]+)"\)\s*,?\s*\)/g)];
    assert.ok(entries.length > 0);
    for (const [, name, relative] of entries)
      await writeFile(path.join(root, name), await readFile(path.resolve(path.dirname(manifest), relative)));
    const child = spawnSync(process.execPath, ["--input-type=module", "-e",
      "const { pathToFileURL } = await import('node:url'); await import(pathToFileURL(process.argv[1]).href);",
      path.join(root, "kernel-browser-host.mjs")],
    { timeout: 30_000, encoding: "utf8", env: { PATH: "/usr/bin:/bin", HOME: root, TMPDIR: root } });
    assert.equal(child.status, 0, child.stderr);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

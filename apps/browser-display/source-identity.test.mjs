// MD-DISPLAY-02/04: a relocated kit must bind execution to measured public bytes.
import test from 'node:test';import assert from 'node:assert/strict';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';import {tmpdir} from 'node:os';
import path from 'node:path';import {createHash} from 'node:crypto';
import {sourceIdentity} from './source-identity.mjs';
test('MD-DISPLAY portable identity checks every kit file and refuses tampering/path traversal',async()=>{
 const root=await mkdtemp(path.join(tmpdir(),'chariox-md-kit-test-')),before=process.env.MD_KIT_MANIFEST;
 try{
  const source='a'.repeat(40),data='public fixture';await writeFile(path.join(root,'fixture'),data);
  const filename=path.join(root,'KIT_MANIFEST.json');process.env.MD_KIT_MANIFEST=filename;
  const manifest={source,files:[{path:'fixture',sha256:createHash('sha256').update(data).digest('hex')}]};
  await writeFile(filename,JSON.stringify(manifest));assert.equal((await sourceIdentity()).source,source);
  await writeFile(path.join(root,'fixture'),'changed');await assert.rejects(sourceIdentity(),/changed kit file/);
  manifest.files[0].path='../fixture';await writeFile(filename,JSON.stringify(manifest));await assert.rejects(sourceIdentity(),/invalid kit path/);
 }finally{if(before===undefined)delete process.env.MD_KIT_MANIFEST;else process.env.MD_KIT_MANIFEST=before;await rm(root,{recursive:true,force:true})}
});

// MD-DISPLAY-02/04: portable kit source identity without a Git installation.
import {execFileSync} from 'node:child_process';
import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
export async function sourceIdentity() {
  if(!process.env.MD_KIT_MANIFEST)return {
    source:execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),
    source_dirty:Boolean(execFileSync('git',['status','--porcelain'],{encoding:'utf8'}).trim()),
  };
  const filename=process.env.MD_KIT_MANIFEST;
  if(!path.isAbsolute(filename))throw Error('MD-DISPLAY: absolute kit manifest required');
  const bytes=await readFile(filename),manifest=JSON.parse(bytes),root=path.dirname(filename);
  if(!/^[a-f0-9]{40}$/.test(manifest.source)||!manifest.files?.length)throw Error('MD-DISPLAY: invalid kit source');
  for(const file of manifest.files){
    if(path.isAbsolute(file.path)||file.path.split('/').includes('..'))throw Error('MD-DISPLAY: invalid kit path');
    const actual=await readFile(path.join(root,file.path));
    if(createHash('sha256').update(actual).digest('hex')!==file.sha256)throw Error('MD-DISPLAY: changed kit file '+file.path);
  }
  return {source:manifest.source,source_dirty:false,kit_manifest_sha256:createHash('sha256').update(bytes).digest('hex')};
}

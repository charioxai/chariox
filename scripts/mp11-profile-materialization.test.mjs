import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import syncFs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {copyPrivateCredential,createPrivateDrillRuntime} from '../apps/cli/scripts/lib/private-drill-runtime.mjs'
const repo=process.env.MP11_TEST_SOURCE??path.resolve(import.meta.dirname,'..')
const source=syncFs.readFileSync(path.join(repo,'apps/cli/scripts/lib/live-provider-thread-transfer-runtime.mjs'),'utf8')
function functionText(name){
 const start=source.indexOf(`function ${name}(`);const async=source.slice(start-6,start)==='async '
 const begin=source.indexOf('{',start);let level=0,quote=null,escape=false,end
 for(let index=begin;index<source.length;index++){
  const c=source[index];if(quote){if(escape)escape=false;else if(c==='\\')escape=true;else if(c===quote)quote=null;continue}
  if('"\'`'.includes(c)){quote=c;continue}if(c==='{')level++;if(c==='}'&&!--level){end=index+1;break}
 }
 return (async?'async ':'')+source.slice(start,end)
}
const copy=new Function('mkdir','copyFile','chmod','path','copyPrivateCredential',`${functionText('copySecretIfPresent')};return copySecretIfPresent`)(fs.mkdir,fs.copyFile,fs.chmod,path,copyPrivateCredential)
test('MP-11 F25 credential aliases preserve unrelated bytes and modes',async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'mp11-profile-'))
 try{const input=path.join(root,'source');const unrelated=path.join(root,'unrelated');const target=path.join(root,'target');await fs.writeFile(input,'synthetic',{mode:0o600});await fs.writeFile(unrelated,'unrelated',{mode:0o644});await fs.symlink(unrelated,target)
 await assert.rejects(copy(input,target));assert.equal(await fs.readFile(unrelated,'utf8')==='unrelated',true);assert.equal((await fs.stat(unrelated)).mode&0o777,0o644)
 }finally{await fs.rm(root,{recursive:true,force:true})}
})
test('MP-11 F25 private credential mode is set before publication',async()=>{
 const runtime=await createPrivateDrillRuntime('mp11-profile-mode')
 try{const source=path.join(runtime.root,'source');const target=path.join(runtime.root,'child','auth.json');await fs.writeFile(source,'synthetic',{mode:0o600});assert.equal(await copy(source,target),true);assert.equal((await fs.stat(target)).mode&0o777,0o600)
 }finally{await runtime.cleanup()}
})
test('MP-11 F25 failed second credential copy removes partial worker state',async()=>{
 let allocated;let count=0
 const scope={HOME:'/synthetic',CODEX_HOME:'/synthetic/codex',OPENCODE_DATA_HOME:'/synthetic/opencode',OPENCODE_CONFIG_DIR:'/synthetic/config',XDG_CONFIG_HOME:'/synthetic/config',XDG_DATA_HOME:'/synthetic/data',XDG_STATE_HOME:'/synthetic/state',XDG_CACHE_HOME:'/synthetic/cache'}
 const dependencies={DEFAULT_PROVIDERS:['codex','opencode'],providersNeedClaudeCredentials:()=>false,CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE:'synthetic',realProviderEnv:()=>scope,createPrivateDrillRuntime:async()=>{const runtime=await createPrivateDrillRuntime('mp11-partial');allocated=runtime.root;return runtime},path,os,mkdir:fs.mkdir,copySecretIfPresent:async(_source,destination)=>{allocated??=path.dirname(path.dirname(destination));if(++count===2)throw new Error('synthetic second copy failure');await fs.writeFile(destination,'synthetic',{mode:0o600});return true}}
 const prepare=new Function(...Object.keys(dependencies),`${functionText('prepareIsolatedWorkerProviderEnv')};return prepareIsolatedWorkerProviderEnv`)(...Object.values(dependencies))
 try{await assert.rejects(prepare(['codex','opencode'],'test'),/synthetic second copy failure/);assert.equal(await fs.stat(allocated).then(()=>true,()=>false),false)}finally{if(allocated)await fs.rm(allocated,{recursive:true,force:true})}
})

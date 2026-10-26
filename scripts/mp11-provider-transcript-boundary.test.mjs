import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import {providerTranscriptMatches,providerTranscriptRoots} from '../apps/cli/scripts/lib/live-external-provider-live-parity-evidence.mjs'
test('MP-11 F28 filenames cannot admit similar or wrong native session ids',async()=>{
 const root=await fs.mkdtemp(path.join(os.tmpdir(),'mp11-transcript-'))
 try{
  for(const provider of ['codex','claude','opencode']) {
   const file=path.join(root,'session-1.jsonl')
   await fs.writeFile(file,JSON.stringify(provider==='codex'?{type:'session_meta',payload:{id:'session-10'}}:{sessionId:'session-10',private:'synthetic-secret'})+'\n')
   assert.equal(await providerTranscriptMatches(provider,file,'session-1'),false)
  }
 }finally{await fs.rm(root,{recursive:true,force:true})}
})
test('MP-11 F28 provider transcript roots require an exact selected scope',()=>{
 assert.throws(()=>providerTranscriptRoots('claude',{}),/explicit/)
 assert.deepEqual(providerTranscriptRoots('claude',{CLAUDE_CONFIG_DIR:'/synthetic/linked-profile'}),['/synthetic/linked-profile'])
 assert.deepEqual(providerTranscriptRoots('opencode',{XDG_DATA_HOME:'/synthetic/xdg'}),['/synthetic/xdg/opencode'])
})

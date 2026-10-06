// MP-08/MP-10/MP-11: frozen prompt and owned fixture lifetime, zero solver calls.
import test from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { stopOwnedProcess } from './round2/owned-processes.mjs'
const scripts=import.meta.dirname
test('MP-08/MP-10 TimeWarp goal interpolation preserves the byte-exact frozen prompt', async()=>{
  const frozen=await readFile(process.env.TIMEWARP_FROZEN_RUNNER,'utf8')
  const current=await readFile(`${scripts}/timewarp-round3.mjs`,'utf8')
  const template=text=>text.match(/const prompt\s*=\s*(`MP-08 \/ MP-10 benchmark smoke\.[\s\S]*?`)/)[1]
  assert.equal(template(current),template(frozen))
})
test('MP-11 actual TimeWarp runner has no raw signals or name-based process kills',async()=>{
  const source=await readFile(`${scripts}/timewarp-round3.mjs`,'utf8')
  assert(!/process\.kill|\.kill\(|pkill|killall/.test(source))
  assert.match(source,/stopOwnedProcess/)
})
test('MP-11 Shop supervisor stdin EOF settles its own server through positive PID guards',async()=>{
  const child=spawn('python3',[`${scripts}/timewarp-shop-supervisor.py`,'python3','-u','-c',
    'import time; print("MP-11 ready",flush=True); time.sleep(60)'],{detached:true,stdio:['pipe','pipe','pipe']})
  await once(child,'spawn');const exited=once(child,'exit')
  try {
    await once(child.stdout,'data');child.stdin.end()
    const result=await Promise.race([exited,new Promise((_,reject)=>setTimeout(()=>reject(Error('MP-11 supervisor timeout')),10000).unref())])
    assert.deepEqual(result,[0,null])
  }finally {await stopOwnedProcess(child,{detached:true})}
})

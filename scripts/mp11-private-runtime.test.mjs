import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'
import {withPrivateDrillRuntime} from '../apps/cli/scripts/lib/private-drill-runtime.mjs'
for(const fail of [false,true])for(const keep of [false,true]) {
 test(`MP-11 F30 private identities are removed on ${fail?'failure':'success'} with keep=${keep}`,async()=>{
  let root
  const operation=withPrivateDrillRuntime('mp11-test',async value=>{
   root=value;assert.equal((await fs.stat(root)).mode&0o777,0o700)
   await fs.mkdir(path.join(root,'profile'));await fs.writeFile(path.join(root,'profile','auth-fixture'),'synthetic-private',{mode:0o600})
   if(fail)throw new Error('synthetic-failure')
   return {keep,evidence:{status:'passed'}}
  })
  if(fail)await assert.rejects(operation,/synthetic-failure/);else assert.deepEqual((await operation).evidence,{status:'passed'})
  assert.equal(await fs.stat(root).then(()=>true,()=>false),false)
 })
}

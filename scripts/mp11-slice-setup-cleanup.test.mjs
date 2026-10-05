import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
const sourceRoot=process.env.MP11_TEST_SOURCE??path.resolve(import.meta.dirname,'..')
function actualSetup(Client){
 const text=fs.readFileSync(path.join(sourceRoot,'apps/cli/scripts/live-remote-native-tui-drill.mjs'),'utf8')
 const a=text.indexOf('async function createHomeManagedLocalDockerSlice(');const b=text.indexOf('\nasync function deleteHomeManagedSlice(',a)
 return new Function('LocalIpcClient','unwrap','createSliceRequest','startSliceRequest','importSliceProviderAuthRequest','deleteSliceRequest',`${text.slice(a,b)};return createHomeManagedLocalDockerSlice`)(Client,response=>response,()=>({kind:'create'}),id=>({kind:'start',id}),(id,provider)=>({kind:'auth',id,provider}),id=>({kind:'delete',id}))
}
for(const fault of ['second-auth','missing-worker','cleanup'])test(`MP-11 F29 ${fault} retains owned slice cleanup`,async()=>{
 const calls=[]
 class Client{
  async send(request){calls.push(request)
   if(request.kind==='create')return {slice:{id:'slice-fixture'}}
   if(request.kind==='start')return {slice:{id:'slice-fixture',...(fault==='second-auth'?{worker_kernel_id:'worker',worker_kernel_ref:'worker-ref'}:{})}}
   if(request.kind==='auth'&&request.provider==='claude'&&fault==='second-auth')throw new Error('synthetic import failure')
   if(request.kind==='delete'&&fault==='cleanup')throw new Error('synthetic cleanup failure')
   return {}
  }
  async close(){calls.push({kind:'close'})}
 }
 await assert.rejects(actualSetup(Client)({homeKernelUrl:'synthetic',workspace:'/synthetic',providers:['codex','claude'],localAuthEnvironment:{}}),fault==='cleanup'?/cleanup failed/:undefined)
 assert.equal(calls.filter(call=>call.kind==='delete').length,1)
 assert.equal(calls.at(-1).kind,'close')
})

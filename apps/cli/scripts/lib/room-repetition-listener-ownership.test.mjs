// MP-10/MP-11: foreign Created-container proxy must not become our historical-port leak.
import assert from 'node:assert/strict'
import test from 'node:test'
import {confirmedForeignProxyPids} from './room-repetition-listener-ownership.mjs'
import {classifyOwnedListeners} from './room-repetition-gates.mjs'
const listener='LISTEN 0 4096 127.0.0.1:44001 0.0.0.0:* users:(("docker-proxy",pid=21,fd=7))'
const args=new Map([[21,['docker-proxy','-proto','tcp','-host-ip','127.0.0.1','-host-port','44001','-container-ip','172.18.0.7','-container-port','44001']]])
const container={id:'foreign',name:'chariox-slice-foreign',ips:['172.18.0.7'],bindings:[{hostIp:'127.0.0.1',hostPort:44001,containerPort:44001}]}
test('MP-10: public binding and proxy tuple prove foreign ownership despite empty ps ports',()=>{
 const confirmed=confirmedForeignProxyPids([listener],[container],'our-run',args)
 assert.deepEqual([...confirmed],[21])
 const result=classifyOwnedListeners([listener],new Set([44001]),['foreign|chariox-slice-foreign|Created|'],'our-run',[],new Set(),new Set([21]),confirmed)
 assert.deepEqual(result.owned,[])
 assert.equal(result.reused.length,1)
})
test('MP-10: mismatched IP or port never excuses an orphan proxy',()=>{
 for(const changed of [{...container,ips:['172.18.0.8']},{...container,bindings:[{...container.bindings[0],hostPort:44002}]}])assert.equal(confirmedForeignProxyPids([listener],[changed],'our-run',args).size,0)
 assert.equal(classifyOwnedListeners([listener],new Set([44001]),[],'our-run',[],new Set(),new Set([21])).owned.length,1)
})
test('MP-10: known owned proxy remains a leak even after foreign port reuse',()=>{
 assert.equal(confirmedForeignProxyPids([listener],[{...container,name:'chariox-slice-our-run'}],'our-run',args).size,0)
 const result=classifyOwnedListeners([listener],new Set([44001]),[],'our-run',[],new Set([21]),new Set([21]),new Set([21]))
 assert.deepEqual(result.owned,[listener])
})

test('MP-10: a proxy that ceased after ss is a stale snapshot, not a live leak',()=>{
 const result=classifyOwnedListeners([listener],new Set([44001]),[],'our-run',[],new Set(),new Set(),new Set(),new Set([21]))
 assert.deepEqual(result.owned,[])
 assert.equal(result.reused.length,1)
})
test('MP-10: confirmed foreign proxy may share its inherited socket with dockerd',()=>{
 const shared=listener.replace('pid=21,fd=7))','pid=21,fd=7),("dockerd",pid=99,fd=8))')
 const result=classifyOwnedListeners([shared],new Set([44001]),[],'our-run',[],new Set(),new Set(),new Set([21]))
 assert.deepEqual(result.owned,[])
 assert.equal(result.reused.length,1)
 // A known owned holder takes precedence over the foreign proof.
 assert.deepEqual(classifyOwnedListeners([shared],new Set([44001]),[],'our-run',[],new Set([99]),new Set(),new Set([21])).owned,[shared])
})
test('MP-10: ss column padding changes do not create extra listeners',async()=>{
 const {compareCycle}=await import('./room-repetition-gates.mjs')
 const base={containers:[],volumes:[],images:[],ownedPids:[],listeners:[listener],kernel:{fds:1,rss:1},relay:{fds:1,rss:1},diskBytes:1}
 assert.deepEqual(compareCycle(base,{...base,listeners:[listener.replaceAll(' ','   ')+'    ']}),[])
 assert.equal(compareCycle(base,{...base,listeners:[listener.replace(':44001',':44002')]}).length,1)
})

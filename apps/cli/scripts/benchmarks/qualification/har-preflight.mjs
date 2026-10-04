#!/usr/bin/env node
// MP-08 / MP-10 / MP-11 H1/H13: lane-owned first-party Chrome fixture only.
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtemp, readFile, rm, open, realpath } from 'node:fs/promises'
import { createServer } from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'
import { observePassiveHar } from './passive-har.mjs'
import { retainFinalHar } from './har-retention.mjs'
const [stateRoot, output]=process.argv.slice(2)
if (!stateRoot?.startsWith('/') || !output?.startsWith('/')) throw new Error('MP-10 absolute external state/output required')
const repository=await realpath(path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../../../../..'))
const relative=path.relative(repository,await realpath(stateRoot))
if(!relative || (!relative.startsWith(`..${path.sep}`)&&relative!=='..'&&!path.isAbsolute(relative)))throw new Error('MP-10 external state required')
const scratch=await mkdtemp(path.join(stateRoot,'har-chrome-'))
const server=createServer((req,res)=>{res.writeHead(200,{'Content-Type':'text/html; charset=utf-8'});res.end('<title>MP-08 MP-10 MP-11 fixture</title><p>Credential-free navigation</p>')})
let chrome, ws, observer, chromeExited, logs, receipt
try {
 server.listen(0,'127.0.0.1');await once(server,'listening')
 logs=await open(path.join(scratch,'chrome.log'),'wx',0o600)
 chrome=spawn('/usr/bin/google-chrome',['--headless=new','--no-sandbox','--disable-dev-shm-usage','--remote-debugging-port=0',`--user-data-dir=${scratch}/profile`,'about:blank'],{stdio:['ignore',logs.fd,logs.fd]})
 chromeExited=once(chrome,'exit')
 const deadline=Date.now()+20000;let port
 while(!port){
  try {port=Number((await readFile(path.join(scratch,'profile/DevToolsActivePort'),'utf8')).split('\n')[0])}catch{}
  if(chrome.exitCode!==null || Date.now()>deadline)throw new Error('MP-10 owned Chrome startup RED')
  if(!port)await delay(50)
 }
 const targets=await(await fetch(`http://127.0.0.1:${port}/json/list`)).json()
 ws=new WebSocket(targets.find(t=>t.type==='page').webSocketDebuggerUrl);await once(ws,'open')
 let nextId=0;const pending=new Map(),listeners=new Set()
 ws.addEventListener('message',({data})=>{
  const msg=JSON.parse(data)
  if(msg.id){const p=pending.get(msg.id);if(p){clearTimeout(p.timer);pending.delete(msg.id);msg.error?p.reject(new Error('MP-10 fixture CDP failure')):p.resolve(msg.result)}}
  else for(const listener of listeners)listener(msg)
 })
 const send=(method,params={})=>new Promise((resolve,reject)=>{const id=++nextId
  const timer=setTimeout(()=>{pending.delete(id);reject(new Error('MP-10 fixture CDP deadline'))},5000)
  pending.set(id,{resolve,reject,timer});ws.send(JSON.stringify({id,method,params}))})
 observer=await observePassiveHar({send,sessionIds:[''],subscribe(fn){listeners.add(fn);return()=>listeners.delete(fn)}})
 await send('Page.enable')
 // Fixture actor only; production collector never navigates or exposes CDP to provider.
 await send('Page.navigate',{url:`http://127.0.0.1:${server.address().port}/`})
 const snapshot=await observer.close({async drain(){
  const deadline=Date.now()+4000
  while(Date.now()<deadline){
   await delay(100)
   try {observer.har.assertComplete();if(observer.har.entries.length)return {settled:true}}catch{}
  }
  return {settled:false}
 }})
 receipt=await retainFinalHar(snapshot,output)

}finally{
 observer?.detach();if(ws){ws.close();await Promise.race([once(ws,'close'),delay(1000)])}
 if(chrome?.exitCode===null){chrome.kill('SIGTERM');await Promise.race([chromeExited,delay(5000)])
  if(chrome.exitCode===null){chrome.kill('SIGKILL');await chromeExited}}
 if(server.listening)await new Promise(resolve=>server.close(resolve))
 await logs?.close();await rm(scratch,{recursive:true})
}

console.log(JSON.stringify({mpItems:['MP-08','MP-10','MP-11'],status:'PASS',receipt,cleanup:{ownedChromeSettled:true,profileRemoved:true,loopbackServerClosed:true},limits:'Unscored first-party fixture; no provider or official full HAR/evaluator campaign.'}))

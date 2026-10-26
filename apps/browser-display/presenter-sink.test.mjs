// MP-08/MP-10/MP-11: main-thread contention must not delay track commits.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {BrowserDisplayPresenter} from './presenter.mjs';
const source=readFileSync(new URL('./decoder-worker.mjs',import.meta.url),'utf8');
test('MP-08/MP-10 60 synthetic frames commit within 6ms despite an 8ms busy main thread',async()=>{
 const prior={Worker:globalThis.Worker,VideoDecoder:globalThis.VideoDecoder,MediaStreamTrackGenerator:globalThis.MediaStreamTrackGenerator,MediaStream:globalThis.MediaStream};
 const latencies=[];let now=0;
 const image=(timestamp,decodedAt=now)=>({timestamp,decodedAt,displayWidth:1280,displayHeight:800,close(){},clone(){return image(timestamp,decodedAt)}});
 const context={clearRect(){},drawImage(frame){latencies.push(now-frame.decodedAt)}};
 class Scope{static[Symbol.hasInstance](){return true}}
 class W{
  constructor(){
   const scope=new Scope();Object.assign(scope,{WorkerGlobalScope:Scope,self:scope,performance:{timeOrigin:0,now:()=>now},EncodedVideoChunk:class{constructor(v){Object.assign(this,v)}},VideoDecoder:class{
    constructor(c){this.c=c;this.state='configured'}configure(){}close(){this.state='closed'}decode(v){queueMicrotask(()=>this.c.output(image(v.timestamp)))}
   }});
   scope.postMessage=data=>{setImmediate(()=>{now+=8;this.onmessage({data})})};
   vm.runInNewContext(source.slice(0,source.indexOf('export class')),scope);this.scope=scope;
  }
  postMessage(data){this.scope.onmessage({data})}terminate(){}
 }
 globalThis.Worker=W;globalThis.VideoDecoder=class{};
 globalThis.MediaStreamTrackGenerator=class{constructor(){this.writable={getWriter:()=>({ready:Promise.resolve(),write:async frame=>{latencies.push(now-frame.decodedAt)},abort:async()=>{}})}}stop(){}};
 globalThis.MediaStream=class{};
 const video={style:{},play:async()=>{},pause(){},remove(){}};
 const document={createElement:()=>video,defaultView:{getComputedStyle:()=>({position:'relative'})}};
 const canvas={width:1280,height:800,style:{},ownerDocument:document,parentElement:{style:{},insertBefore(){}},getContext:()=>context};
 const binding={subscription_id:'s',generation:1,tab_id:'t'},presenter=new BrowserDisplayPresenter(canvas,binding);
 try{
  for(let sequence=1;sequence<=60;sequence++)await presenter.present({...binding,sequence,document_id:'d',kind:'video',key:sequence===1,codec:'vp8',width:1280,height:800,css_width:1280,css_height:800,device_scale_factor:1,data:new Uint8Array([1])});
  assert.equal(latencies.length,60);latencies.sort((a,b)=>a-b);
  const p95=latencies[Math.ceil(latencies.length*.95)-1];console.log(JSON.stringify({MP:'MP-08/MP-10/MP-11',synthetic:true,frames:60,busyMainMs:8,decodeCommitP95:p95}));
  assert.ok(p95<=6,`decode→commit P95 ${p95}ms exceeds 6ms`);
  assert.ok(latencies.at(-1)<=1000/60,'no commit may miss the first rAF after decode');
 }finally{presenter.close();Object.assign(globalThis,prior)}
});

test('MP-11 decode-ahead rejects invalid moves and foreign bindings before worker dispatch',async()=>{
 const presenter=new BrowserDisplayPresenter({width:1280,height:800},{subscription_id:'s',generation:1,tab_id:'t'});
 let dispatched=0;presenter.getWorkerDecoder=()=>{dispatched++;throw Error('must not dispatch')};
 const base={subscription_id:'s',generation:1,tab_id:'t',sequence:1,document_id:'d',kind:'video',key:true,codec:'vp8',width:1280,height:800,css_width:1280,css_height:800,device_scale_factor:1,data:new Uint8Array([1])};
 try{
  for(const bad of [{...base,moves:[[0,0,1,1,0]]},{...base,generation:2},{...base,width:1279}]){
   presenter.prepareVideo(bad);await assert.rejects(presenter.prepared.get(bad));
  }
  assert.equal(dispatched,0);
 }finally{presenter.close();}
});

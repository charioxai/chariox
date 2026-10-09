// MD-DISPLAY-02/04: attestation is a private source fence, never pixel authority.
import test from 'node:test';
import {createHash} from 'node:crypto';
import {decodePng,displayMaskRegions} from './kernel-browser-pixels.mjs';
import assert from 'node:assert/strict';
import {CompositorSource,jpegDimensions} from './kernel-browser-compositor.mjs';
import {encodePng} from './kernel-browser-pixels.mjs';
function fixture(){
 const handlers=new Set(),calls=[];let data=encodePng(8,8,Buffer.alloc(8*8*4,255));
 const tab={target_id:'target',tab_id:'tab',document_id:'doc'},policy={values:[],targets:[],unknown:false};let current=policy;
 const emit=(method,params={},sessionId='session')=>{for(const h of handlers)h({method,params,sessionId})};
 const connection={subscribe:h=>{handlers.add(h);return()=>handlers.delete(h)},send:async(method,params)=>{calls.push(method);if(method==='Page.startScreencast')emit('Page.screencastFrame',{data,sessionId:1});if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame',loaderId:'doc'}}};return {}}};
 const source=new CompositorSource({connection,sessionId:'session',tab,scale:1,policy,width:8,height:8,hasher:{hash:async data=>({signature:createHash('sha256').update(decodePng(data).pixels).digest('hex'),width:8,height:8}),close:async()=>{}},screenshot:async()=>({data_base64:data}),allowed:p=>p===current&&!p.unknown&&!p.values.length&&!p.targets.length});
 return {source,emit,calls,handlers,policy,setPolicy:p=>current=p,setData:p=>data=p};
}
test('MD-DISPLAY compositor is attested, latest-only and ACKs without viewer credit',async()=>{
 const f=fixture();await f.source.start();assert.equal(f.source.sample().serial,1);assert.equal(f.source.sample(1),null);
 f.emit('Page.screencastFrame',{data:encodePng(8,8,Buffer.alloc(8*8*4,0)),sessionId:2});await new Promise(r=>setTimeout(r,0));assert.equal(f.source.sample(1).serial,2);
 assert.equal(f.calls.filter(x=>x==='Page.screencastFrameAck').length,2);await f.source.close();assert.equal(f.handlers.size,0);assert.equal(f.source.sample(),null);
});
test('MD-DISPLAY compositor navigation fences immediately even before a new capture',async()=>{
 const f=fixture();await f.source.start();f.emit('Page.frameNavigated',{frame:{id:'main',loaderId:'new'}});assert.equal(f.source.sample(),null);
 f.emit('Page.screencastFrame',{data:encodePng(8,8,Buffer.alloc(8*8*4,0)),sessionId:2});assert.equal(f.source.sample(),null);await f.source.close();assert.equal(f.handlers.size,0);
});
test('MD-DISPLAY compositor policy insertion/unknown/targets deny raw observations',async()=>{
 for(const policy of [{values:['synthetic-secret'],targets:[],unknown:false},{values:[],targets:[],unknown:true},{values:[],targets:[{}],unknown:false}]){
  const f=fixture();await f.source.start();f.setPolicy(policy);assert.equal(f.source.sample(),null);f.emit('Page.screencastFrame',{data:'AAAA',sessionId:2});assert.equal(f.source.latest,null);await f.source.close();
 }
});
test('MD-DISPLAY compositor rejects mismatched attestation and unwinds ownership',async()=>{
 const f=fixture();f.source.screenshot=async()=>({data_base64:encodePng(8,8,Buffer.alloc(8*8*4,0))});await assert.rejects(f.source.start(),/attestation differed/);assert.equal(f.handlers.size,0);assert.equal(f.source.sample(),null);assert(f.calls.includes('Page.stopScreencast'));
});

test('MD-DISPLAY screenshot restoration damage does not start compositor motion',async()=>{
 const f=fixture();await f.source.start();f.source.pause();f.emit('Page.screencastFrame',{data:encodePng(8,8,Buffer.alloc(8*8*4,0)),sessionId:2});assert.equal(f.source.sample().serial,1);f.source.resume();f.emit('Page.screencastFrame',{data:encodePng(8,8,Buffer.alloc(8*8*4,0)),sessionId:3});assert.equal(f.source.sample().serial,1);await f.source.close();
});

test('MD-DISPLAY JPEG geometry parsing is bounded and rejects truncation',()=>{
 const bytes=Buffer.from([255,216,255,192,0,8,8,6,64,10,0,3]);
 assert.deepEqual(jpegDimensions(bytes),{width:2560,height:1600});
 assert.equal(jpegDimensions(bytes.subarray(0,9)),null);assert.equal(jpegDimensions(Buffer.from('unsafe')),null);
});

test('MD-DISPLAY a late fingerprint cannot revive a closed source',async()=>{
 const f=fixture();await f.source.start();let resolve,started;
 const pending=new Promise(r=>resolve=r),entered=new Promise(r=>started=r);
 f.source.hasher.hash=async()=>{started();return pending};
 f.emit('Page.screencastFrame',{data:encodePng(8,8,Buffer.alloc(8*8*4,0)),sessionId:2});await entered;
 await f.source.close();resolve({signature:'changed',width:8,height:8});await new Promise(r=>setTimeout(r,0));
 assert.equal(f.source.latest,null);assert.equal(f.source.sample(),null);assert.equal(f.handlers.size,0);
});

test('MD-DISPLAY source closed during render lease admission releases the late lease',async()=>{
 const f=fixture();let admit,releases=0;const held=new Promise(r=>admit=r);
 f.source.acquire=()=>held;const started=f.source.start();await f.source.close();
 admit(async()=>{releases++});await assert.rejects(started,/retired/);
 assert.equal(releases,1);assert.equal(f.handlers.size,0);
});

test('MP-11 protection retirement keeps the renderer lease and rejects a late old capture',async()=>{
 const f=fixture();await f.source.start();let release,enter,captures=0;
 const held=new Promise(r=>release=r),entered=new Promise(r=>enter=r);
 const black=encodePng(8,8,Buffer.alloc(8*8*4,0));
 f.source.protect=async()=>{if(++captures===1){enter();return held;}return {data_base64:black};};
 f.emit('Page.screencastFrame',{data:encodePng(8,8,Buffer.alloc(8*8*4,255)),sessionId:2});await entered;
 f.emit('DOM.attributeModified',{name:'data-chariox-observation-protected'});
 assert.equal(f.source.sample(),null);assert.equal(f.source.closed,false);assert.equal(f.source.regionRevision,1);
 assert(!f.calls.includes('Page.stopScreencast'),'retirement cannot interrupt mouse press/release');
 release({data_base64:encodePng(8,8,Buffer.alloc(8*8*4,255))});await new Promise(r=>setTimeout(r,0));
 assert.equal(f.source.sample().data_base64,black);assert.equal(captures,2);
 await f.source.close();
});

test('MP-11 DOM churn keeps publishing bound captures; only a changed mask set retires frames',async()=>{
 const f=fixture();await f.source.start();let captures=0,release;const held=new Promise(r=>release=r);
 const grey=encodePng(8,8,Buffer.alloc(8*8*4,128)),black=encodePng(8,8,Buffer.alloc(8*8*4,0)),mask=[{x:0,y:0,width:2,height:2}];
 f.source.protect=async()=>{
  if(++captures===1){f.emit('DOM.childNodeInserted',{});return {data_base64:grey,[displayMaskRegions]:[]};}
  if(captures===2){await held;return {data_base64:black,[displayMaskRegions]:mask};}
  return {data_base64:black,[displayMaskRegions]:mask};
 };
 f.emit('DOM.childNodeInserted',{});await new Promise(r=>setTimeout(r,0));
 assert.equal(f.source.sample()?.data_base64,grey,'a capture bound before/after its own masks survives later tree churn');
 assert.equal(f.source.regionRevision,0);
 release();await new Promise(r=>setTimeout(r,0));
 assert.equal(f.source.sample().data_base64,black);assert.equal(f.source.regionRevision,1,'a changed mask set retires older frames');
 await f.source.close();
});

test('MP-11 style/class churn wakes captures only while protected regions exist',async()=>{
 const f=fixture();await f.source.start();let captures=0;
 f.source.protect=async()=>{captures++;return {data_base64:encodePng(8,8,Buffer.alloc(8*8*4,255)),[displayMaskRegions]:[]};};
 f.emit('DOM.attributeModified',{name:'style'});await new Promise(r=>setTimeout(r,0));
 assert.equal(captures,0);assert.equal(f.source.sample().serial,1);
 f.emit('DOM.attributeModified',{name:'data-chariox-secret'});await new Promise(r=>setTimeout(r,0));
 assert.equal(captures,1,'declared protection still recaptures');assert.equal(f.source.regionRevision,1);
 await f.source.close();
});

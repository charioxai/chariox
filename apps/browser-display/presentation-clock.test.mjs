// MP-08/MP-10/MP-11: diagnostics must name real compositor presentation.
import test from 'node:test';
import assert from 'node:assert/strict';
import {VideoSink} from './video-sink.mjs';
function fixture(){
 const saved={MediaStreamTrackGenerator:globalThis.MediaStreamTrackGenerator,MediaStream:globalThis.MediaStream};
 globalThis.MediaStreamTrackGenerator=class{constructor(){this.writable={}}stop(){}};globalThis.MediaStream=class{};
 const callbacks=new Map();let id=0;
 const video={style:{},dataset:{},play:async()=>{},pause(){},remove(){},requestVideoFrameCallback(fn){callbacks.set(++id,fn);return id},cancelVideoFrameCallback(id){callbacks.delete(id)}};
 const document={hidden:false,createElement:()=>video,defaultView:{getComputedStyle:()=>({position:'relative'})},addEventListener(){},removeEventListener(){}};
 const canvas={dataset:{},style:{},width:16,height:16,parentElement:{style:{},insertBefore(){}},ownerDocument:document,getContext:()=>({clearRect(){}})};
 const sink=new VideoSink(canvas,()=>{});sink.setup({setSink(){}});
 const image=timestamp=>({timestamp,close(){},clone(){return image(timestamp)}});
 return {sink,canvas,video,image,callbacks,fire(metadata){const next=callbacks.entries().next().value;assert(next,'presentation observer armed');callbacks.delete(next[0]);next[1](32,metadata)},close(){sink.close();Object.assign(globalThis,saved)}};
}
test('MP-08/MP-10/MP-11 track write is not committedAt; rVFC joins the actual video timestamp',async()=>{
 const f=fixture();try{
  const outcome=await f.sink.commit({image:f.image(1234567),decodedAt:8,writtenAt:9},{sequence:7,width:16,height:16});
  assert.equal(outcome.committedAt,undefined,'writer completion cannot claim presentation');assert.equal(outcome.writtenAt,9);
  f.fire({mediaTime:1.234567,presentationTime:31,expectedDisplayTime:33,presentedFrames:1});
  assert.equal(outcome.committedAt,performance.timeOrigin+31);assert.equal(Number(f.canvas.dataset.displayPresentedSequence),7);
 }finally{f.close()}
});
test('MP-08/MP-10/MP-11 adjacent microsecond timestamps join exactly and a retired sink ignores late callbacks',async()=>{
 const f=fixture();try{
  const a=await f.sink.commit({image:f.image(1000000),decodedAt:8,writtenAt:9},{sequence:1,width:16,height:16});
  const b=await f.sink.commit({image:f.image(1000001),decodedAt:8,writtenAt:9},{sequence:2,width:16,height:16});
  f.fire({mediaTime:1,presentationTime:31,expectedDisplayTime:33,presentedFrames:1});
  assert.equal(a.committedAt,performance.timeOrigin+31);assert.equal(b.committedAt,undefined);
  const late=f.callbacks.entries().next().value[1];f.sink.stopTrack();late(35,{mediaTime:1.000001,presentationTime:35});
  assert.equal(b.committedAt,undefined);
  assert.equal(f.canvas.dataset.displayPresentedSequence,'1');
 }finally{f.close()}
});

test('MP-08/MP-10/MP-11 rVFC arriving before worker reply retains its exact frame clock',async()=>{
 const f=fixture();try{
  f.fire({mediaTime:1.234567,presentationTime:31,expectedDisplayTime:33,presentedFrames:1});
  const outcome=await f.sink.commit({image:f.image(1234567),decodedAt:8,writtenAt:9},{sequence:7,width:16,height:16});
  assert.equal(outcome.committedAt,performance.timeOrigin+31);assert.equal(f.canvas.dataset.displayPresentedSequence,'7');
 }finally{f.close()}
});

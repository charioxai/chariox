// MP-08/MP-10/MP-11: whole-video and repair-overlay probe regressions.
import test from 'node:test';import assert from 'node:assert/strict';
import {sampleVisibleFrame} from './probe-surface.mjs';
function surface(kind,luminance,{overlay=false,ready=true}={}){
 const rgba=value=>new Uint8ClampedArray([value,value,value,255]);
 const context={pixel:0,drawImage(image){this.pixel=image.pixel},getImageData(){return {data:rgba(this.pixel)}}};
 const base={tagName:kind==='track'?'VIDEO':'CANVAS',dataset:{displaySink:kind},pixel:luminance,readyState:ready?2:0,videoWidth:ready?1280:0,videoHeight:ready?800:0,width:1280,height:800,hidden:false};
 const canvas={pixel:overlay?80:0,width:1280,height:800,style:{opacity:overlay?'':'0'},previousElementSibling:base,getContext:()=>({getImageData:()=>({data:rgba(0)})}),ownerDocument:{createElement:()=>({getContext:()=>context})}};
 return canvas;
}
for(const kind of ['track','canvas'])for(const dark of [false,true])test(`MP-10 ${kind} samples ${dark?'dark':'light'} whole video rather than cleared overlay`,()=>{
 const canvas=surface(kind,dark?30:230),frames=[{sequence:9,drawn:200}];
 assert.equal(sampleVisibleFrame(canvas,frames,100,dark)?.frame.sequence,9);
 assert.equal(sampleVisibleFrame(canvas,frames,100,!dark),null,'transparent overlay cannot invent a dark transition');
});
test('MP-10 repair overlay composes above the video base',()=>{
 assert.equal(sampleVisibleFrame(surface('canvas',230,{overlay:true}),[{sequence:9,drawn:200}],100,true)?.mean,80);
});
test('MP-10 unavailable track pixels and pre-input metadata cannot be joined',()=>{
 assert.equal(sampleVisibleFrame(surface('track',230,{ready:false}),[{sequence:9,drawn:200}],100,false),null);
 assert.equal(sampleVisibleFrame(surface('canvas',230),[{sequence:9,drawn:50}],100,false),null);
});

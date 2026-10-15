// MP-08/MP-10/MP-11: media is content inside a bound frame, never an identity proof.
import test from 'node:test';
import assert from 'node:assert/strict';
import {attestationPlan,assertAttestationPixels} from './kernel-browser-region-attestation.mjs';

const page={left:4,top:87,width:100,height:80};
const region={pageLeft:20,pageTop:20,width:40,height:20,contentRect:page};
const media=[{id:'n3',box:[20,20,40,20]}];
function rasters(width,height){
 const pixels=Buffer.alloc(width*height*4);
 for(let y=0;y<height;y++)for(let x=0;x<width;x++){
  const i=(y*width+x)*4;pixels[i]=(x*7+y)%256;pixels[i+1]=(y*11+x)%256;pixels[i+2]=(x*3+y*5)%256;pixels[i+3]=255;
 }
 const reference={width,height,pixels},raw={width,height,pixels:Buffer.from(pixels)};
 for(let i=0;i<pixels.length;i+=4){raw.pixels[i]=pixels[i+2];raw.pixels[i+2]=pixels[i];}
 return {reference,raw};
}
test('MP-11 a media-filled crop uses surrounding page anchors with exact origin',()=>{
 const p=attestationPlan(region,media,1);
 assert.deepEqual(p.rect,{left:4,top:87,width:100,height:80});
 assert.deepEqual(p.media,[{x:20,y:20,width:40,height:20}]);
});
test('MP-11 native/CDP media raster differences are allowed only inside reported boxes',()=>{
 const p=attestationPlan(region,media,1),{raw,reference}=rasters(100,80);
 raw.pixels[(25*100+25)*4]++;
 assert.doesNotThrow(()=>assertAttestationPixels(raw,reference,p.media,[]));
 assert.throws(()=>assertAttestationPixels(raw,reference,[],[]),/mismatch/,'unreported media stays exact');
 raw.pixels[(25*100+19)*4]++;
 assert.throws(()=>assertAttestationPixels(raw,reference,p.media,[]),/mismatch/,'no padding excludes adjacent non-media pixels');
});
test('MP-11 shifted origin, wrong raster/window and non-media differences refuse',()=>{
 const {raw,reference}=rasters(100,80);
 assert.throws(()=>assertAttestationPixels({...raw,width:98},reference,[],[]),/geometry/);
 const shifted={...raw,pixels:Buffer.from(raw.pixels)};raw.pixels.copy(shifted.pixels,0,4);
 assert.throws(()=>assertAttestationPixels(shifted,reference,[{x:20,y:20,width:40,height:20}],[]),/mismatch/);
 raw.pixels[0]++;
 assert.throws(()=>assertAttestationPixels(raw,reference,[{x:20,y:20,width:40,height:20}],[]),/mismatch/);
});
test('MP-11 full-bleed media without surrounding identity pixels refuses at both densities',()=>{
 for(const scale of [1,2]){
  const {raw,reference}=rasters(100,80);
  assert.throws(()=>assertAttestationPixels(raw,reference,[{x:0,y:0,width:100,height:80}],[]),/anchors/);
  assert.throws(()=>attestationPlan(region,[{id:'n3',box:[NaN,0,1,1]}],scale),/geometry/);
 }
});
test('MP-11 no media leaves the exact crop as its own attestation',()=>{
 const p=attestationPlan(region,[],1);
 assert.deepEqual(p.rect,{left:24,top:107,width:40,height:20});
 const {raw,reference}=rasters(40,20);assertAttestationPixels(raw,reference,p.media,[]);
});

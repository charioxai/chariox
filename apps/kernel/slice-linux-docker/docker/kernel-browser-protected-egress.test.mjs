// MP-08/MP-10/MP-11 safe fallback must honour egress without stranding its exact tiles.
import test from 'node:test';import assert from 'node:assert/strict';import {randomBytes} from 'node:crypto';
import {DisplayStream} from './kernel-browser-display.mjs';import {encodePng,decodePng} from './kernel-browser-pixels.mjs';
test('MP-11 bounded safe lossless fallback progressively repairs its admitted base',async()=>{
 const pixels=randomBytes(512*512*4);for(let i=3;i<pixels.length;i+=4)pixels[i]=255;
 const source={generation:1,force_lossless:true,data_base64:encodePng(512,512,pixels)};
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',device_scale_factor:1,bitrate:8000000,codec:'avc1.420033'},{encoder:{close:async()=>{}},now:()=>0,wait:async()=>{}});
 stream.sequence=1;stream.document_id='d';stream.previous={signature:'lossy'};
 const actual=Buffer.alloc(pixels.length);
 try{for(let n=0;n<50;n++){
  const frame=await stream.frame(source,'d',stream.sequence);if(!frame)break;
  assert.equal(frame.kind,'tiles');assert.equal(frame.base_sequence,frame.sequence-1);assert(Buffer.byteLength(JSON.stringify(frame))*4/3+1024<1024*1024);
  for(const tile of frame.tiles){const p=decodePng(tile.data_base64).pixels;for(let y=0;y<tile.height;y++)p.copy(actual,((tile.y+y)*512+tile.x)*4,y*tile.width*4,(y+1)*tile.width*4)}
 }assert.equal(stream.exact,true);assert.deepEqual(actual,pixels);
 const bootstrap=await stream.frame(source,'other',stream.sequence);
 assert.equal(bootstrap.kind,'png');assert.equal(stream.exact,false);
 const black=decodePng(bootstrap.data_base64).pixels;
 for(let n=0;n<black.length;n+=4)assert.deepEqual(black.subarray(n,n+4),Buffer.from([0,0,0,255]));
 assert.ok(stream.repair?.length,'a new document starts bounded exact recovery');
 }finally{await stream.close()}
});

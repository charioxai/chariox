import test from 'node:test';
import assert from 'node:assert/strict';
import { DisplayCapture, changedClip } from './kernel-browser-display-capture.mjs';
import { encodePng } from './kernel-browser-pixels.mjs';
test('MD-DISPLAY crop pixels stay native; unchanged preview verifies missed fine detail', async () => {
  let pixels=Buffer.alloc(1280*800*4,255), thumbnail=Buffer.alloc(160*100*4,255);
  const calls=[], tab={tab_id:'t',document_id:'d'}, policy={};
  const capture = new DisplayCapture(async clip => {
    calls.push(clip?.scale < 1?'preview':clip?'crop':'full');
    const width=clip?(clip.scale<1?160:clip.width):1280, height=clip?(clip.scale<1?100:clip.height):800;
    const data=clip?.scale<1?thumbnail:Buffer.alloc(width*height*4);
    if(!clip||clip.scale===1) for(let row=0;row<height;row++)pixels.copy(data,row*width*4,((clip?.y??0)+row)*1280*4+(clip?.x??0)*4,((clip?.y??0)+row)*1280*4+(clip?.x??0)*4+width*4);
    return {...tab,generation:1,data_base64:encodePng(width,height,data)};
  },1);
  await capture.next(tab,policy,false);calls.length=0;
  pixels[(104*1280+104)*4]=0;thumbnail[(13*160+13)*4]=0;
  const patch=await capture.next(tab,policy,true);
  assert.deepEqual(calls,['preview','crop']);assert.deepEqual(patch.pixels.pixels,pixels);
  // A one-pixel change not represented in the thumbnail must be read back.
  pixels[(700*1280+1000)*4]=0;calls.length=0;
  const verified=await capture.next(tab,policy,true);
  assert.deepEqual(calls,['full']);assert.deepEqual(verified.pixels.pixels,pixels);
  calls.length=0;await capture.next(tab,{},true);assert.deepEqual(calls,['preview','full'],'policy replacement fences cached pixels');
  calls.length=0;await capture.next(tab,policy,false);assert.deepEqual(calls,['preview','full'],'lost credit base fences cache');
});
test('MD-DISPLAY large preview damage falls back; wrong document is rejected', async () => {
 const before={width:160,height:100,pixels:Buffer.alloc(160*100*4)}, after={...before,pixels:Buffer.alloc(160*100*4,255)};
 assert.equal(changedClip(before,after,.125),null);
 const capture=new DisplayCapture(async()=>({tab_id:'t',document_id:'other',data_base64:''}),1);
 await assert.rejects(capture.next({tab_id:'t',document_id:'d'},{},false),/binding changed/);
 assert.equal(capture.previous,null);
});

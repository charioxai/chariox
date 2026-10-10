import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserDisplayPresenter} from './presenter.mjs';

// Public presentation boundary, with only OS image/canvas APIs replaced.
test('M2 native protected window presents exact PNG and clipped tiles without a browser tab', async () => {
  const originalCanvas=globalThis.OffscreenCanvas, originalBitmap=globalThis.createImageBitmap;
  let draws=0;
  class Canvas {constructor(w,h){this.width=w;this.height=h;}getContext(){return {drawImage(){draws++;}}}}
  globalThis.OffscreenCanvas=Canvas;
  globalThis.createImageBitmap=async blob=>{const data=new Uint8Array(await blob.arrayBuffer());return {width:data[0],height:data[1],close(){}};};
  try {
    const canvas=new Canvas(1,1), binding={display_kind:'native',surface_id:'macos-seat',generation:'epoch',subscription_id:'s',width:130,height:80,device_scale_factor:2};
    const viewer=new BrowserDisplayPresenter(canvas,binding);
    const full={...binding,sequence:1,css_width:65,css_height:40,document_id:'source-1',kind:'png',data:new Uint8Array([130,80])};
    assert.equal(await viewer.present(full),true);
    assert.equal(canvas.width,130);assert.equal(draws,2);
    assert.equal(await viewer.present({...full,surface_id:'other',sequence:2}),false);
    assert.equal(await viewer.present({...full,generation:'old',sequence:2}),false);
    assert.equal(await viewer.present({...full,display_kind:undefined,sequence:2}),false);
    assert.equal(await viewer.present({...full,sequence:2,kind:'tiles',base_sequence:1,tiles:[{x:128,y:64,width:2,height:16,format:'png',data:new Uint8Array([2,16])}]}),true);
    await assert.rejects(viewer.present({...full,sequence:3,kind:'tiles',base_sequence:1,tiles:[]}),/base lost/);
    await assert.rejects(viewer.present({...full,sequence:3,width:132,css_width:66}),/geometry/);
    assert.equal(await viewer.present({...full,sequence:3,document_id:'source-2'}),true);
    viewer.close();assert.equal(canvas.width,1);
  }finally{globalThis.OffscreenCanvas=originalCanvas;globalThis.createImageBitmap=originalBitmap;}
});

test('M2 native attachment uses the encrypted shared event route and never submits native IDs as tabs', async () => {
  const {attachNativeDisplay}=await import('./native-presenter.mjs');
  let listener, registered, finish;
  const requests=[], receipt=new Promise(r=>finish=r);
  const target={surface_id:'macos-seat',generation:'epoch'};
  const transport={kernelProtocolVersion:497,relayProtocolVersion:96,displayEventEncoding:'CXD1',
    onEvent(fn){listener=fn;return()=>{};},
    async subscribeDisplay(binding){registered=binding;},async unsubscribeDisplay(){},
    async request(request){requests.push(request);const c=request.KernelBrowser.command;assert.equal(c.op,'computer');
      assert.deepEqual(c.command.target,target);
      return {KernelBrowser:{result:c.command.op==='display_subscribe'?{...target,display_kind:'native',subscription_id:'s',width:130,height:80,device_scale_factor:2}:c.command.op==='display_next'?await receipt:{closed:true}}};}
  };
  const stream=await attachNativeDisplay({width:1,height:1},transport,target);
  assert.equal(registered.generation,'native:epoch');assert.equal(stream.presenter.binding.generation,'epoch');
  stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true;};
  const pending=stream.next();
  await assert.rejects(stream.next(),/credit outstanding/);
  listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence:1}});finish({frame_sent:true});
  await pending;assert.equal(stream.presenter.sequence,1);
  await stream.close();assert.equal(requests.at(-1).KernelBrowser.command.command.op,'display_close');
  let attempts=0;
  await assert.rejects(attachNativeDisplay({}, {...transport,kernelProtocolVersion:491,request(){attempts++;}},target),/497/);
  assert.equal(attempts,0);
});

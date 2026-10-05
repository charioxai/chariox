import test from 'node:test';
import assert from 'node:assert/strict';
import { attachBrowserDisplay } from './presenter.mjs';

test('MD-DISPLAY credit stays occupied through event-before-receipt and presentation', async () => {
  let listener, finishRequest, finishPresentation, presentationStarted;
  const receipt = new Promise(resolve => { finishRequest = resolve; });
  const presenting = new Promise(resolve => { presentationStarted = resolve; });
  const presentation = new Promise(resolve => { finishPresentation = resolve; });
  let calls = 0;
  const transport = {
    onEvent: callback => { listener = callback; return () => {}; },
    request: async ({ KernelBrowser: { command } }) => ({ KernelBrowser: { result:
      command.op === 'display_subscribe' ? { subscription_id: 's' } :
      command.op === 'display_next' && ++calls === 1 ? await receipt : { frame_sent: false },
    } }),
  };
  const stream = await attachBrowserDisplay({ width: 1, height: 1 }, transport, { tab_id: 't', generation: 1 });
  stream.presenter.present = async () => { presentationStarted(); await presentation; return true; };
  const first = stream.next();
  try {
    listener({ event: 'kernel_browser_frame', subscription_id: 's', frame: { sequence: 1 } });
    await assert.rejects(stream.next(), /credit outstanding/);
    await Promise.race([presenting,new Promise((_,reject)=>setTimeout(()=>reject(Error('MD-DISPLAY: receipt stalls presentation')),100))]);
    finishRequest({ frame_sent: true });
    await assert.rejects(stream.next(), /credit outstanding/);
  } finally {
    finishRequest({ frame_sent: true }); finishPresentation();
    await first; await stream.close();
  }
});

test('MD-DISPLAY bounded window sends four credits before any receipt and drains before stop', async () => {
 let listener; const credits=[];
 const transport={onEvent:fn=>{listener=fn;return()=>{}},request:async({KernelBrowser:{command}})=>({KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s'}:command.op==='display_next'?await new Promise(resolve=>credits.push({command,resolve})):{}}})};
 const stream=await attachBrowserDisplay({width:1,height:1},transport,{tab_id:'t',generation:1});
 stream.presenter.present=async frame=>{stream.presenter.sequence=frame.sequence;return true};
 stream.start(); await new Promise(resolve=>setTimeout(resolve,10));
 assert.equal(credits.length,4);
 const stopped=stream.stop();
 for(let i=0;i<4;i++){listener({event:'kernel_browser_frame',subscription_id:'s',frame:{sequence:i+1}});credits[i].resolve({frame_sent:true})}
 await stopped;assert.equal(credits.length,4);assert.equal(stream.presenter.sequence,4);
 await stream.close();
});

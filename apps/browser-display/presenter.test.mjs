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

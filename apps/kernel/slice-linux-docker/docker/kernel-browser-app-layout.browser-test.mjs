// MP-08 / MP-10: real App target, CDP emulation and initial mirror geometry.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { inflateRawSync, constants as zlib } from 'node:zlib';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

test('MP-08 / MP-10 an App viewport matches its first capped mirror packet', {timeout:30000}, async () => {
  assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT,'explicit disposable drill root required');
  const root=await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT,'app-layout-'));
  const host=new KernelBrowserHost(root);
  try {
    const opened=await host.handle({id:1,method:'browser.app.open',params:{
      origin_label:'layout-app',installation_id:'layout-app',
      assets:[{path:'index.html',content_type:'text/html',body_base64:Buffer.from('<!doctype html><button style="position:fixed;right:0;bottom:0">Save</button>').toString('base64')}],
    }});
    assert.equal(opened.ok,true,JSON.stringify(opened));
    const {target_id}=opened.result;
    const tab=[...host.tabs.values()].find(tab=>tab.target_id===target_id);
    assert([...host.browser.appTabs.apps.values()].some(app=>app.targetId===target_id));
    const {connection,sessionId}=await host.browser.resolvePageTarget(target_id);
    const viewport=async()=>{
      const reply=await connection.send('Runtime.evaluate',{expression:'({width:innerWidth,height:innerHeight})',returnByValue:true},sessionId);
      assert(!reply.exceptionDetails);
      return reply.result.value;
    };
    const native=await host.browser.nativeViewport(target_id);
    assert(native.width<1920&&native.height<1080,'requested layout must exceed the real native page');
    const expected={width:2*Math.floor(native.width/2),height:2*Math.floor(native.height/2)};
    const subscribed=await host.request({op:'mirror_subscribe',tab_id:tab.tab_id,generation:host.generation,device_scale_factor:1,wire:2,layout_width:1920,layout_height:1080});
    // Read directly after subscription, before any unrelated reconcile can repair the App.
    assert.deepEqual(await viewport(),expected,'subscription must resize the App immediately');
    const reply=await host.request({op:'mirror_next',subscription_id:subscribed.subscription_id,generation:host.generation,after_sequence:0,drift_nodes:[],wait_ms:1000});
    const packet=reply.encoding==='deflate'
      ? JSON.parse(inflateRawSync(Buffer.from(reply.packet_base64,'base64'),{finishFlush:zlib.Z_SYNC_FLUSH}).toString()) : reply;
    assert.deepEqual({width:packet.css_width,height:packet.css_height},expected);
    assert.deepEqual(await viewport(),expected);
    const smaller={width:900,height:500};
    await host.request({op:'mirror_subscribe',tab_id:tab.tab_id,generation:host.generation,device_scale_factor:1,wire:2,layout_width:smaller.width,layout_height:smaller.height});
    assert.deepEqual(await viewport(),smaller,'smaller App viewers apply through the same CDP path');
  } finally {
    await host.stop();
    await rm(root,{recursive:true,force:true});
  }
});

// MP-08/MP-10/MP-11: public default crop activation and allocated floor.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {localProtocolVersion,relayPeerProtocolVersion} from './lib/protocol-versions.mjs';
import {KernelBrowserHost} from '../apps/kernel/slice-linux-docker/docker/kernel-browser-host.mjs';
import * as region from '../apps/browser-display/region-binding.mjs';
test('MP-08/MP-10/MP-11: default native subscribe reaches visible-page admission before legacy viewport fitting',async()=>{
 const previous=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 const host=Object.assign(Object.create(KernelBrowserHost.prototype),{
  generation:1,start:async()=>{},protection:{unknown:false},streams:new Map(),displays:new Map(),chromium:{desktop:{binding:()=>({})}},
  target:async()=>assert.fail('default subscribe must never reach the resizing legacy capture'),
 });
 try{await assert.rejects(host.request({op:'display_subscribe',tab_id:'t',generation:0,codecs:['png','avc1.420033'],bitrate:8000000,device_scale_factor:1}),e=>e.code==='user_domain_stale_reference');}
 finally{if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=previous;}
});
test('MP-08/MP-10/MP-11: visible page has allocated536/144 snapshot and client activation floor',()=>{
 assert(localProtocolVersion>=536);assert(relayPeerProtocolVersion>=144);
 assert.equal(region.visiblePageMinimumProtocolVersion,536);
 const root=new URL('../apps/kernel/src/local/api/tests/protocol_shapes/',import.meta.url);
 const bytes=readFileSync(new URL('browser-visible-page-536.json',root));
 const fixture=JSON.parse(bytes);assert.equal(fixture.local,536);assert.equal(fixture.relay_peer,144);
 assert.equal(fixture.requests[0].KernelBrowser.command.target,undefined);
 assert.equal(fixture.response.KernelBrowser.result.source_kind,'region');
 assert.equal(fixture.response.KernelBrowser.result.css_height,712);
 const rust=readFileSync(new URL('kernel_browser.rs',root),'utf8');
 assert(rust.includes(createHash('sha256').update(bytes).digest('hex')));
});

test('MP-08/MP-10/MP-11: a headless Linux host without a live desktop keeps its full-viewport path',async()=>{
 const previous=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 const legacy=Error('headless legacy capture reached');
 const host=Object.assign(Object.create(KernelBrowserHost.prototype),{generation:1,start:async()=>{},protection:{unknown:false},streams:new Map(),displays:new Map(),chromium:{desktop:{binding:()=>null}},target:async()=>{throw legacy}});
 try{await assert.rejects(host.request({op:'display_subscribe',tab_id:'t',generation:0,codecs:['png','avc1.420033'],bitrate:8000000,device_scale_factor:1}),error=>error===legacy);}
 finally{if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=previous;}
});

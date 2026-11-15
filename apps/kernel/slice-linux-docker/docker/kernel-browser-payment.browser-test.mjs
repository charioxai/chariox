// Opt-in regression through the production browser host and real Chromium.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { decodePng } from './kernel-browser-pixels.mjs';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

for (const dpr of [1, 2]) test(`saved-card checkout commits once through real browser input DPR${dpr}`, async () => {
  assert.ok(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, 'Explicit installed Chromium required');
  const root = await mkdtemp(path.join(tmpdir(), 'vault-payment-'));
  let paid = 0;
  const server = createServer((request, response) => {
    if (request.url === '/paid') { paid++; response.end('Test payment received'); return; }
    response.setHeader('Content-Type', 'text/html');
    response.end('<!doctype html><form onsubmit="event.preventDefault();fetch(\'/paid\',{method:\'POST\'}).then(()=>document.querySelector(\'p\').textContent=\'Test receipt\')"><label>Test card<input id="card" type="text" style="background:magenta"></label><button id="edit" type="button">Continue editing</button><button id="pay">Pay $12.50</button></form><p>Test-mode checkout</p>'.replace('Pay $12.50',dpr===2?'Subscribe':'Pay $12.50'));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`, url = origin + '/';
  const host = new KernelBrowserHost(root, {appStorageRoot:path.join(root,'apps')});
  try {
    const opened = await host.request({op:'open',url});
    await host.request({op:'resize',tab_id:opened.tab_id,generation:opened.generation,width:1280,height:900,device_scale_factor:dpr});
    const tab = host.tabs.get(opened.tab_id), binding = {tab_id:tab.tab_id,generation:host.generation,document_id:tab.document_id};
    const snapshot = await host.request({op:'snapshot',...binding});
    const node = id => snapshot.snapshot.dom_nodes.find(node => node.attributes?.id === id);
    assert.ok(node('card') && node('pay'));
    const secret = '4242424242424242'; // Published test card, never a real one.
    await host.protect({values:[secret],targets:[],unknown:false});
    const fill = await host.handle({id:'fill',method:'host.secret',params:{...binding,plain_target:true,node_ref:node('card').node_ref,observed_by:'agent:buyer',_agent_input:true,_vault_payment:{id:'one-payment',origin,expires_at_ms:Date.now()+900000},action:{kind:'fill',text:secret,expected_document_url:url}}});
    assert.equal(fill.ok,true,JSON.stringify(fill));
    const full=await host.handle({id:'protected-full-page',method:'host.browser',params:{op:'screenshot',...binding,observed_by:'agent:buyer',_agent_input:true,full_page:true}});
    assert.equal(full.ok,true,JSON.stringify(full));
    const fullImage=decodePng(full.result.data_base64,1,{width:full.result.width,height:full.result.height}), fullField=node('card').bounds;
    const fullPixel=(Math.round(fullField.y+fullField.height/2)*fullImage.width+Math.round(fullField.x+fullField.width/2))*4;
    assert.deepEqual([...fullImage.pixels.subarray(fullPixel,fullPixel+3)],[0,0,0],'full-page pixels keep the plain card masked');
    const visible=await host.request({op:'screenshot',...binding,observed_by:'agent:buyer',_agent_input:true});
    assert.ok(visible.protected_regions.length>0,'the agent can still inspect a masked viewport');
    const image=decodePng(visible.data_base64,dpr), mask=visible.protected_regions[0];
    const pixel=(Math.round(mask.y+mask.height/2)*image.width+Math.round(mask.x+mask.width/2))*4;
    assert.deepEqual([...image.pixels.subarray(pixel,pixel+3)],[0,0,0],'the actual plain card field is black in model-bound pixels');
    const {connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);
    await connection.send('Runtime.evaluate',{expression:'document.querySelector("form").style.marginTop="2000px"'},sessionId);
    const offscreen=await host.handle({id:'offscreen-full-page',method:'host.browser',params:{op:'screenshot',...binding,observed_by:'agent:buyer',_agent_input:true,full_page:true}});
    assert.equal(offscreen.ok,false);assert.match(offscreen.error.message,/viewport screenshot.*access is still held/i);
    await connection.send('Runtime.evaluate',{expression:'document.querySelector("form").style.marginTop="";document.querySelector("input").type="password"'},sessionId);
    const dots=await host.request({op:'screenshot',...binding,observed_by:'agent:buyer',_agent_input:true,full_page:true});
    assert.equal(dots.mime_type,'image/png','password dots do not need masking');
    await connection.send('Runtime.evaluate',{expression:'document.querySelector("input").type="text"'},sessionId);
    const input = (id, action, approvals) => host.handle({id,method:'host.browser',params:{op:'input',...binding,observed_by:'agent:buyer',_agent_input:true,_vault_payment_approvals:approvals,input:action}});
    const enter=await input('enter-without-approval',{kind:'key',key:'Enter'},[]);
    assert.equal(enter.ok,false);assert.match(enter.error.message,/payment.*approval/);
    const edit=node('edit').bounds;
    const editing=await input('ordinary-checkout-click',{kind:'click',x:Math.round(edit.x+edit.width/2),y:Math.round(edit.y+edit.height/2)},[]);
    assert.equal(editing.ok,true,'ordinary editing stays available without a payment approval');
    const box=node('pay').bounds;
    const click = {kind:'click',x:Math.round(box.x+box.width/2),y:Math.round(box.y+box.height/2)};
    const denied = await input('no-live-turn',click,[]);
    assert.equal(denied.ok,false); assert.match(denied.error.message,/payment.*approval/);
    assert.equal(paid,0);
    const authority = [{id:'one-payment',origin}];
    const allowed = await input('pay',click,authority);
    assert.equal(allowed.ok,true,JSON.stringify(allowed));
    const end = Date.now()+5000;
    while (paid === 0 && Date.now()<end) await delay(50);
    assert.equal(paid,1);
    const repeated = await input('lost-reply-retry',click,authority);
    assert.equal(repeated.ok,false); assert.match(repeated.error.message,/already sent/);
    assert.equal(paid,1);
    const observed = await host.request({op:'snapshot',...binding});
    assert.ok(!JSON.stringify(observed).includes(secret),'filled card stays out of model observations');
    // A receipt no longer renders the filled field; its retained registration must not block a full-page read.
    await connection.send('Runtime.evaluate',{expression:'document.querySelector("form").remove()'},sessionId);
    const receipt=await host.request({op:'screenshot',...binding,observed_by:'agent:buyer',_agent_input:true,full_page:true});
    assert.equal(receipt.mime_type,'image/png');assert.ok(receipt.data_base64.length>100);
  } finally {
    await host.stop();
    if (host.chromium.browserUser) await rm(host.chromium.browserUser.root,{recursive:true,force:true});
    await new Promise(resolve => server.close(resolve));
    await rm(root,{recursive:true,force:true});
  }
});

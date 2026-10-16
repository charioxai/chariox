// MP-08/MP-10/MP-11: real renderer event-path review regressions, synthetic pages only.
import assert from 'node:assert/strict';
import {performance} from 'node:perf_hooks';
export async function rendererReview({source,viewer,originUrl,viewerUrl,receipt,mode,resource}) {
  const opened=await source.request({op:'open',url:`${originUrl}/forms`,observed_by:'drill'});
  const tab=source.tabs.get(opened.tab_id),connection=await source.browser.ensureConnection();
  const sessionId=await source.browser.ensureTargetSession(connection,tab.target_id);
  const evaluate=async expression=>(await connection.send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true},sessionId)).result.value;
  const next=()=>viewer.evaluate(()=>window.mirror.next().then(()=>{}));
  try {
    await evaluate(`(()=>{const a=document.querySelector('#ordinary');a.placeholder='Field A';a.focus();const b=document.createElement('input');b.placeholder='Field B';a.after(b);})()`);
    await viewer.goto(viewerUrl);await viewer.waitForFunction(()=>typeof window.start==='function');
    await viewer.evaluate(binding=>window.start(binding),{tab_id:tab.tab_id,generation:opened.generation,device_scale_factor:1});
    await next();
    if(mode!=='documents') {
      const fields=viewer.frameLocator('#mirror > iframe').locator('input');
      // A normal credit captures A; transport then holds those exact bytes for
      // 400ms. B is clicked/typed while it travels, and typed again immediately
      // after apply, without waiting for any post-click refresh.
      let capturedResolve,releaseResolve,heldPacket;
      const captured=new Promise(resolve=>capturedResolve=resolve),release=new Promise(resolve=>releaseResolve=resolve);
      const route=async route=>{
        const command=route.request().postDataJSON().KernelBrowser.command;
        if(command.op!=='mirror_next'){await route.continue();return;}
        const response=await route.fetch();heldPacket=(await response.json()).KernelBrowser.result;
        capturedResolve();await release;await route.fulfill({response});
      };
      await viewer.route('**/kernel',route);
      await viewer.evaluate(()=>{window.delayedReviewPacket=window.mirror.next()});await captured;
      const started=performance.now();let expectedFocus;
      await fields.nth(1).click();expectedFocus=await viewer.evaluate(()=>{const r=window.mirror.renderer;return r.ids.get(r.frame.contentDocument.activeElement)});await fields.nth(1).pressSequentially('Q');
      await viewer.evaluate(()=>window.mirror.renderer.inputChain);
      const remaining=400-(performance.now()-started);if(remaining>0)await new Promise(resolve=>setTimeout(resolve,remaining));
      releaseResolve();await viewer.evaluate(()=>window.delayedReviewPacket);await viewer.unroute('**/kernel',route);
      const active=await viewer.evaluate(()=>(()=>{const r=window.mirror.renderer;return r.ids.get(r.frame.contentDocument.activeElement)})());
      await viewer.keyboard.type('R');await viewer.evaluate(()=>window.mirror.renderer.inputChain);
      const values=await evaluate(`Array.from(document.querySelectorAll('input')).filter(n=>n.type!=='password').map(n=>n.value)`);
      receipt.security.push({check:'MP-08/MP-11 delayed pre-click credit (400ms), B click/immediate typing and typing after apply',active,expectedFocus,values,captured_focus:heldPacket.focused,delay_ms:performance.now()-started,passed:active===expectedFocus&&values[0]===''&&values[1]==='QR'});
      assert.equal(active,expectedFocus,'MP-08: delayed packet cannot restore field A over newer local focus');
      assert.deepEqual(values,['','QR'],'MP-11: printable input intended for B never dispatches to A');
      // A post-input observation must still restore kernel-driven focus (Tab).
      await evaluate(`document.querySelector('#ordinary').focus()`);await next();await next();
      assert.equal(await viewer.evaluate(()=>{const r=window.mirror.renderer;return r.ids.get(r.frame.contentDocument.activeElement)}),heldPacket.focused,'MP-08: source-driven focus moves from B back to A');
      await viewer.keyboard.press('Tab');await viewer.evaluate(()=>window.mirror.renderer.inputChain);
      await next();await next();
      const focusedSource=await evaluate(`document.activeElement?.placeholder`);
      const focusedClient=await viewer.evaluate(()=>{const r=window.mirror.renderer;return r.ids.get(r.frame.contentDocument.activeElement)});
      assert.equal(focusedSource,'Field B','MP-08: source Tab advances to the unprotected B field');
      assert.equal(focusedClient,expectedFocus,'MP-08: settled Tab focus advances to the exact B node');
      receipt.security.push({check:'MP-08 source focus advances after local input fence settles',passed:true});
      // Local input can also advance while apply awaits packet hash/resource
      // validation. Capture A, pause that await, then focus/type into B.
      await evaluate(`document.querySelector('#ordinary').focus()`);await next();await next();
      await viewer.evaluate(()=>{
        const digest=crypto.subtle.digest.bind(crypto.subtle);let once=true;
        window.reviewHashStarted=false;
        crypto.subtle.digest=async(...args)=>{if(once){once=false;window.reviewHashStarted=true;await new Promise(resolve=>window.resumeReviewHash=resolve)}return digest(...args)};
        window.restoreReviewHash=()=>crypto.subtle.digest=digest;
        window.delayedReviewPacket=window.mirror.next();
      });
      await viewer.waitForFunction(()=>window.reviewHashStarted);
      await fields.nth(1).click();await fields.nth(1).press('End');await fields.nth(1).pressSequentially('S');await viewer.evaluate(()=>window.mirror.renderer.inputChain);
      await viewer.evaluate(()=>{window.resumeReviewHash();window.restoreReviewHash()});await viewer.evaluate(()=>window.delayedReviewPacket);
      assert.equal(await viewer.evaluate(()=>{const r=window.mirror.renderer;return r.ids.get(r.frame.contentDocument.activeElement)}),expectedFocus,'MP-11: focus/input during asynchronous apply cannot roll back');
      await viewer.keyboard.type('T');await viewer.evaluate(()=>window.mirror.renderer.inputChain);
      assert.deepEqual(await evaluate(`Array.from(document.querySelectorAll('input')).filter(n=>n.type!=='password').map(n=>n.value)`),['','QRST']);
      receipt.security.push({check:'MP-08/MP-11 local focus/input during async apply preserves B and exact text targets',passed:true});
      // A reset (including protection reset) must discard local intent even
      // on the same source Document; it cannot carry focus to a new authority.
      await evaluate(`document.querySelector('#ordinary').focus()`);
      const reset=await source.request({op:'mirror_next',subscription_id:source.mirror.streams.keys().next().value,generation:opened.generation,after_sequence:0,drift_nodes:[],observed_by:'drill'});
      assert(reset.reset,'MP-11 reset fixture must use a reset packet');
      await viewer.evaluate(serial=>window.mirror.renderer.apply(JSON.parse(serial)),JSON.stringify(reset));
      assert.equal(await viewer.evaluate(()=>window.mirror.renderer.localFocus),null);
      assert.equal(await viewer.evaluate(()=>{const r=window.mirror.renderer;return r.ids.get(r.frame.contentDocument.activeElement)}),reset.focused);
      await next();
      await fields.nth(1).click();await viewer.evaluate(()=>window.mirror.renderer.inputChain);
      await source.request({op:'navigate',tab_id:tab.tab_id,generation:opened.generation,url:`${originUrl}/forms`,observed_by:'drill'});await next();
      assert.equal(await viewer.evaluate(()=>window.mirror.renderer.localFocus),null,'MP-11 navigation discards old focus');
      receipt.security.push({check:'MP-11 same-document reset and navigation discard local focus without retargeting',passed:true});
      await resource();
    }
    if(mode!=='focus') {
      await viewer.evaluate(()=>{
        const r=window.mirror.renderer,bind=r.bindEvents;window.reviewDocuments=[];
        r.bindEvents=function(doc){
          if(!window.reviewDocuments.some(p=>p.doc===doc)){
            const p={doc,listeners:0,removed:0},add=doc.addEventListener,remove=doc.removeEventListener;
            doc.addEventListener=function(...args){p.listeners++;return add.apply(this,args)};
            doc.removeEventListener=function(...args){p.listeners--;p.removed++;return remove.apply(this,args)};
            window.reviewDocuments.push(p);
          }
          return bind.call(this,doc);
        };
      });
      for(let cycle=0;cycle<12;cycle++) {
        await evaluate(`new Promise(resolve=>{document.querySelector('iframe')?.remove();const frame=document.createElement('iframe');frame.src='/nested?cycle=${cycle}';frame.onload=resolve;document.querySelector('main').append(frame);})`);
        await next();
        // Reset the same source document as well as replacing/removing frames.
        if(cycle%3===0) {
          const credit=await source.request({op:'mirror_next',subscription_id:source.mirror.streams.keys().next().value,generation:opened.generation,after_sequence:0,drift_nodes:[],observed_by:'drill'});
          await viewer.evaluate(serial=>window.mirror.renderer.apply(JSON.parse(serial)),JSON.stringify(credit));
          // attach's credit cursor is deliberately reconciled with a normal next.
          await next();
        }
        await evaluate(`document.querySelector('iframe').remove()`);await next();
        const bounds=await viewer.evaluate(()=>{
          const r=window.mirror.renderer,live=new Set([r.frame.contentDocument]);
          for(const doc of live)for(const frame of doc.querySelectorAll('iframe'))if(frame.contentDocument)live.add(frame.contentDocument);
          const retired=window.reviewDocuments.filter(p=>!live.has(p.doc));
          const leaked=retired.filter(p=>p.listeners!==0);
          return {retained_dom_docs:[...r.dom.values()].filter(n=>!live.has(n.ownerDocument)).length,retired:retired.length,leaked:leaked.length,removed:retired.reduce((n,p)=>n+p.removed,0),bound:r.documentBindings?.size,live:live.size};
        });
        receipt.security.push({check:`MP-11 retired nested Documents released before close, cycle${cycle}`, ...bounds,passed:bounds.leaked===0&&bounds.bound===bounds.live});
        assert.equal(bounds.leaked,0,'MP-11: removed/replaced/reset nested Document has zero listeners before close');
        assert.equal(bounds.bound,bounds.live,'MP-11: renderer retains bindings only for live Documents');
        assert.equal(bounds.retained_dom_docs,0,'MP-11: node map also releases retired Documents');
        if(cycle%3===0)await resource();
      }
    }
  } finally {
    await viewer.evaluate(()=>window.mirror?.close()).catch(()=>{});
    await source.request({op:'close',tab_id:tab.tab_id,generation:opened.generation,observed_by:'drill'});
  }
}

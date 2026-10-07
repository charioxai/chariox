// MP-08/MP-10/MP-11: real renderer events and live source admission regressions.
import assert from 'node:assert/strict';
export async function rendererInputReview({source,viewer,originUrl,viewerUrl,receipt,mode,resource}) {
  for(const seam of mode==='input'?['caret','fallback','coordinate','tab','native-keys','controls']:[mode]) {
    const opened=await source.request({op:'open',url:`${originUrl}/forms`,observed_by:'drill'});
    const tab=source.tabs.get(opened.tab_id),connection=await source.browser.ensureConnection();
    const sessionId=await source.browser.ensureTargetSession(connection,tab.target_id);
    const evaluate=async expression=>(await connection.send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true},sessionId)).result.value;
    const next=()=>viewer.evaluate(()=>window.mirror.next().then(()=>{}));
    const drain=()=>viewer.evaluate(()=>window.mirror.renderer.inputChain);
    const read=source.mirror.evaluate.bind(source.mirror);
    try {
      if(seam==='caret')await evaluate(`(()=>{document.querySelector('main').innerHTML='<input value="abcdef" style="appearance:none;width:240px;font:20px monospace;padding:0;border:0"><div contenteditable="true" style="font:20px monospace;width:240px">abcdef</div>';})()`);
      if(seam==='coordinate')await evaluate(`(()=>{document.querySelector('main').innerHTML='<button id="one" style="width:160px;height:50px">One</button><button id="two" style="width:160px;height:50px">Two</button>';window.clicked=[0,0];document.querySelector('#one').onclick=()=>window.clicked[0]++;document.querySelector('#two').onclick=()=>window.clicked[1]++;})()`);
      if(seam==='tab')await evaluate(`(()=>{document.querySelector('main').innerHTML='<input placeholder="A"><input placeholder="B">';const a=document.querySelector('input');a.onkeydown=e=>{if(e.key==='Enter')document.querySelectorAll('input')[1].focus()};a.focus();})()`);
      if(seam==='native-keys')await evaluate(`(()=>{document.querySelector('main').innerHTML='<input placeholder="A"><input placeholder="B" value="abc"><button>Commit</button>';window.keyClicks=0;document.querySelector('button').onclick=()=>window.keyClicks++;document.querySelector('input').focus();})()`);
      if(seam==='controls')await evaluate(`document.querySelector('main').innerHTML='<input style="box-sizing:content-box;width:200px;height:30px;padding:5px;border:3px solid">'`);
      await viewer.goto(viewerUrl);await viewer.waitForFunction(()=>typeof window.start==='function');
      await viewer.evaluate(binding=>window.start(binding),{tab_id:tab.tab_id,generation:opened.generation,device_scale_factor:1});
      await next();
      const frame=viewer.frameLocator('#mirror > iframe');
      if(seam==='controls') {
        const layout=await viewer.evaluate(()=>window.mirror.renderer.layoutFidelity());
        assert(layout.max_error_css_px<=0.5,'MP-10: native control tile preserves border-box geometry with source content-box sizing');
        receipt.security.push({check:'MP-08/MP-10 native control source content-box width/height + padding/border preserves sampled geometry',layout,passed:true});
      }
      if(seam==='tab')for(const key of ['Tab','Enter'])for(const delayed of [false,true]) {
        await evaluate(`(()=>{for(const e of document.querySelectorAll('input'))e.value='';document.querySelector('input').focus();})()`);await next();await next();
        let release=()=>{},capturedResolve;const captured=new Promise(resolve=>capturedResolve=resolve),held=new Promise(resolve=>release=resolve);
        const route=async route=>{if(route.request().postDataJSON().KernelBrowser.command.op!=='mirror_next'){await route.continue();return;}const response=await route.fetch();capturedResolve();await held;await route.fulfill({response});};
        try {
          if(delayed){await viewer.route('**/kernel',route);await viewer.evaluate(()=>{window.tabPacket=window.mirror.next()});await captured;}
          else await viewer.evaluate(()=>window.tabPoll=setInterval(()=>{if(!window.tabPollBusy){window.tabPollBusy=true;window.tabPollingCredit=window.mirror.next().catch(()=>{}).finally(()=>window.tabPollBusy=false)}},25));
          await viewer.keyboard.press(key);await viewer.keyboard.type('Q');await drain();
          assert.deepEqual(await evaluate(`Array.from(document.querySelectorAll('input'),e=>e.value)`),['','Q'],`MP-11: ${key} then immediate printable input targets the native B focus`);
          if(delayed){await new Promise(resolve=>setTimeout(resolve,400));release();await viewer.evaluate(()=>window.tabPacket);await viewer.keyboard.type('R');await drain();assert.deepEqual(await evaluate(`Array.from(document.querySelectorAll('input'),e=>e.value)`),['','QR'],'MP-11: a delayed pre-Tab focus packet never retargets subsequent text to A');}
          else{await viewer.evaluate(()=>clearInterval(window.tabPoll));await viewer.evaluate(()=>window.tabPollingCredit);}
          receipt.security.push({check:`MP-08/MP-11 ${key} then immediate typing with ${delayed?'400ms delayed pre-key packet + typing after apply':'normal 25ms polling'}`,passed:true});
        } finally{release();await viewer.unroute('**/kernel',route);await viewer.evaluate(()=>clearInterval(window.tabPoll));}
      }
      if(seam==='tab') {
        await evaluate(`(()=>{const fields=document.querySelectorAll('input');fields[1].type='password';fields[1].value='MASK-ME';fields[0].value='';fields[0].focus();})()`);await next();await next();
        await viewer.evaluate(()=>{window.inputFailures=[];window.mirror.renderer.failure=e=>window.inputFailures.push(e.message)});
        await viewer.keyboard.press('Tab');await viewer.keyboard.type('T');await drain();
        assert.equal(await evaluate(`document.querySelector('input[type=password]').value`),'MASK-ME');
        assert.equal(await evaluate(`document.querySelector('input').value`),'');
        assert.equal((await viewer.evaluate(()=>window.inputFailures)).length,1,'MP-11: protected post-Tab native focus refuses before inserting');
        assert.equal(source.generation,opened.generation,'MP-11: refusal dispatches no text and keeps healthy browser generation');
        await evaluate(`new Promise(resolve=>{const main=document.querySelector('main');main.replaceChildren();const f=document.createElement('iframe');f.style.cssText='display:block;margin-left:150px;margin-top:80px;border:3px solid;padding:7px;width:400px;height:160px';f.onload=()=>{f.contentDocument.querySelector('input').focus();resolve()};f.srcdoc='<html><body style="margin:0"><input><input></body></html>';main.append(f);})`);await next();await next();
        await frame.frameLocator('iframe').locator('input').nth(0).click();await drain();
        await viewer.keyboard.press('Tab');await viewer.keyboard.type('N');await drain();
        assert.deepEqual(await evaluate(`Array.from(document.querySelector('iframe').contentDocument.querySelectorAll('input'),e=>e.value)`),['','N'],'MP-08: native post-key focus resolves inside an observed frame');
        await viewer.keyboard.press('ArrowLeft');await viewer.keyboard.type('K');await drain();
        assert.deepEqual(await evaluate(`Array.from(document.querySelector('iframe').contentDocument.querySelectorAll('input'),e=>e.value)`),['','KN'],'MP-08/MP-11: subsequent native key and text preserve nested field focus/caret');
        await source.request({op:'navigate',tab_id:tab.tab_id,generation:opened.generation,url:`${originUrl}/forms`,observed_by:'drill'});await next();
        assert.equal(await viewer.evaluate(()=>window.mirror.renderer.nativeFocus),null,'MP-11: navigation/reset drops native focus mode');
        receipt.security.push({check:'MP-08/MP-11 post-key protected focus refuses without generation change; nested focus types into B; navigation clears mode',passed:true});
      }
      if(seam==='native-keys') {
        for(const key of ['Backspace','ArrowLeft','Tab']) {
          await evaluate(`(()=>{const fields=document.querySelectorAll('input');fields[0].value='';fields[1].value='abc';fields[0].focus();window.keyClicks=0;})()`);await next();await next();
          await viewer.evaluate(()=>{window.inputFailures=[];window.mirror.renderer.failure=e=>window.inputFailures.push(e.message)});
          await viewer.keyboard.press('Tab');await drain();
          assert.equal(await evaluate(`document.activeElement.placeholder`),'B','MP-11: first Tab has native B focus');
          assert.deepEqual(await evaluate(`(()=>{const b=document.activeElement;return [b.selectionStart,b.selectionEnd]})()`),[0,3],'MP-08: native Tab selects the existing field text, without a caret workaround');
          const painted=await viewer.evaluate(()=>({sequence:window.mirror.renderer.sequence,focused:window.mirror.renderer.records.get(window.mirror.renderer.ids.get(window.mirror.renderer.frame.contentDocument.activeElement))?.id}));
          let release=()=>{},capturedResolve;const captured=new Promise(resolve=>capturedResolve=resolve),held=new Promise(resolve=>release=resolve);let capturedPacket;
          const route=async route=>{if(route.request().postDataJSON().KernelBrowser.command.op!=='mirror_next'){await route.continue();return;}const response=await route.fetch();capturedPacket=(await response.json()).KernelBrowser.result;capturedResolve();await held;await route.fulfill({response});};
          try {
            await viewer.route('**/kernel',route);
            await viewer.evaluate(()=>window.keyPoll=setInterval(()=>{if(!window.keyPollBusy){window.keyPollBusy=true;window.keyPollingCredit=window.mirror.next().catch(e=>window.inputFailures.push(e.message)).finally(()=>window.keyPollBusy=false)}},25));
            await captured;
            assert.notEqual(capturedPacket.focused,painted.focused,'MP-11: normal polling captures B while the viewer still paints A');
            assert.equal(capturedPacket.sequence,painted.sequence+1);
            await viewer.keyboard.press(key);
            if(key==='Tab')await viewer.keyboard.press('Enter');else {
              await drain();
              if(key==='Backspace')assert.equal(await evaluate(`document.querySelectorAll('input')[1].value`),'','MP-08: native Backspace deletes the selected B text before any insert');
              else assert.deepEqual(await evaluate(`(()=>{const b=document.querySelectorAll('input')[1];return [b.selectionStart,b.selectionEnd]})()`),[0,0],'MP-08: native ArrowLeft collapses B selection before any insert');
              await viewer.keyboard.type('X');
            }
            await drain();
            assert.deepEqual(await viewer.evaluate(()=>window.inputFailures),[],`MP-11: post-Tab ${key} admits the real native focus despite an in-flight newer epoch`);
            if(key==='Tab')assert.equal(await evaluate('window.keyClicks'),1,'MP-08: post-Tab Tab/Enter reaches the native button exactly once');
            else assert.deepEqual(await evaluate(`Array.from(document.querySelectorAll('input'),e=>e.value)`),['',key==='Backspace'?'X':'Xabc'],`MP-08: post-Tab ${key} then immediate text has the correct effect`);
            await new Promise(resolve=>setTimeout(resolve,400));
            await viewer.evaluate(()=>clearInterval(window.keyPoll));release();await viewer.evaluate(()=>window.keyPollingCredit);
            assert.deepEqual(await viewer.evaluate(()=>window.inputFailures),[],'MP-11: delayed response never closes the mirror or replays a key');
            receipt.security.push({check:`MP-08/MP-11 Tab then ${key}${key==='Tab'?'/Enter':'/text'} with normal25ms polling and400ms delayed B focus packet`,passed:true});
          } finally {await viewer.evaluate(()=>clearInterval(window.keyPoll));release();await viewer.evaluate(()=>window.keyPollingCredit);await viewer.unroute('**/kernel',route);}
        }
      }
      if(seam==='native-keys') {
        await evaluate(`(()=>{const fields=document.querySelectorAll('input');fields[0].focus();fields[1].value='MASK-ME';})()`);await next();await next();
        await viewer.evaluate(()=>{window.inputFailures=[];window.mirror.renderer.failure=e=>window.inputFailures.push(e.message)});
        await viewer.keyboard.press('Tab');await drain();
        await evaluate(`(()=>{const b=document.querySelectorAll('input')[1];b.type='password';window.protectedKeys=0;b.addEventListener('keydown',()=>window.protectedKeys++)})()`);
        await viewer.keyboard.press('Backspace');await drain();
        assert.equal(await evaluate(`document.querySelector('input[type=password]').value`),'MASK-ME');
        assert.equal(await evaluate('window.protectedKeys'),0,'MP-11: newly protected native focus dispatches no key');
        const failures=await viewer.evaluate(()=>window.inputFailures);assert.equal(failures.length,1);assert(!failures[0].includes('stale mirror input epoch'),'MP-11: protected key refusal cannot be retried as a stale sequence');
        assert.equal(source.generation,opened.generation,'MP-11: pre-dispatch refusal preserves the healthy generation');
        receipt.security.push({check:'MP-11 native keyboard newly protected focus refuses before keyDown with no retry marker or generation change',passed:true});
      }
      if(seam==='caret') {
        const field=frame.locator('input');
        // Click the boundary after character 1, never Home/End. Both the viewer
        // and source must retain this exact caret when printable text arrives.
        const fieldPoint=await viewer.evaluate(()=>{const e=window.mirror.renderer.frame.contentDocument.querySelector('input'),b=e.getBoundingClientRect();const canvas=e.ownerDocument.createElement('canvas'),c=canvas.getContext('2d');c.font=e.ownerDocument.defaultView.getComputedStyle(e).font;return {x:c.measureText('a').width,y:b.height/2}});
        await field.click({position:fieldPoint});await viewer.keyboard.type('X');await drain();
        const fieldValue=await evaluate(`document.querySelector('input').value`);
        assert.equal(fieldValue,'aXbcdef','MP-08: appearance:none field click preserves character boundary');
        await next();await next();
        const edit=frame.locator('[contenteditable]');
        const offset=await viewer.evaluate(()=>{const e=window.mirror.renderer.frame.contentDocument.querySelector('[contenteditable]'),r=e.ownerDocument.createRange();r.setStart(e.firstChild,1);r.setEnd(e.firstChild,2);const b=r.getBoundingClientRect(),box=e.getBoundingClientRect();return {x:b.x-box.x,y:b.y-box.y+b.height/2}});
        await edit.click({position:offset});await viewer.keyboard.type('Y');await drain();
        const editorValue=await evaluate(`document.querySelector('[contenteditable]').textContent`);
        assert.equal(editorValue,'aYbcdef','MP-08: contenteditable click preserves collapsed caret');
        receipt.security.push({check:'MP-08/MP-11 actual clicked character boundary + immediate typing in DOM field/editor, no Home/End',fieldValue,editorValue,passed:true});
        await evaluate(`new Promise(resolve=>{const frame=document.createElement('iframe');frame.style.cssText='display:block;margin-left:150px;margin-top:80px;border:3px solid;padding:7px;width:400px;height:160px';frame.onload=resolve;frame.srcdoc='<html><body style="margin:0;font:20px monospace"><input value="abcdef" style="appearance:none;width:240px;font:20px monospace;padding:0;border:0"><div contenteditable="true">abcdef</div></body></html>';document.querySelector('main').append(frame);})`);
        await next();await next();
        const nested=frame.frameLocator('iframe');
        await nested.locator('input').click({position:fieldPoint});await viewer.keyboard.type('N');await drain();
        assert.equal(await evaluate(`document.querySelector('iframe').contentDocument.querySelector('input').value`),'aNbcdef','MP-08: nested field root-coordinate caret includes frame border/padding');
        await nested.locator('[contenteditable]').click({position:offset});await viewer.keyboard.type('M');await drain();
        assert.equal(await evaluate(`document.querySelector('iframe').contentDocument.querySelector('[contenteditable]').textContent`),'aMbcdef','MP-08: nested rich editor retains the clicked boundary');
        receipt.security.push({check:'MP-08/MP-11 nested field/editor clicked caret includes offset frame border and padding',passed:true});
      }
      if(seam==='fallback') {
        source.mirror.evaluate=async(world,expression)=>{if(expression.includes('.read('))throw Error('MP-10 forced observer failure');return read(world,expression)};
        await next();
        const box=await evaluate(`(()=>{const b=document.querySelector('#ordinary').getBoundingClientRect();return {x:b.x+b.width/2,y:b.y+b.height/2}})()`);
        await frame.locator('body > div').click({position:box});await viewer.keyboard.type('fallback');await drain();
        const text=await evaluate(`document.querySelector('#ordinary').value`);
        assert.equal(text,'fallback','MP-08: real printable keys reach source field through full-frame fallback');
        await viewer.evaluate(()=>{const d=window.mirror.renderer.frame.contentDocument,e=d.activeElement;for(const [type,data]of [['compositionstart',''],['compositionupdate','日本'],['compositionend','日本']])e.dispatchEvent(new d.defaultView.CompositionEvent(type,{bubbles:true,data}))});await drain();
        assert.equal(await evaluate(`document.querySelector('#ordinary').value`),'fallback日本','MP-08: full-frame IME commit uses protected display text input');
        await evaluate(`document.querySelector('input[type=password]').focus()`);
        await assert.rejects(viewer.evaluate(()=>window.mirror.input({kind:'coordinate',input:{kind:'text',text:'REFUSED'}})),/secret field input/);
        assert.equal(await evaluate(`document.querySelector('input[type=password]').value`),'MASK-ME','MP-11: fallback printable input preserves Vault-only secret input');
        receipt.security.push({check:'MP-08/MP-11 forced full-frame observer failure: real typing, IME commit, protected-field refusal',passed:true});
      }
      if(seam==='coordinate') {
        await viewer.evaluate(()=>{window.inputFailures=[];window.mirror.renderer.failure=e=>window.inputFailures.push(e.message)});
        await evaluate(`document.querySelector('#one').before(document.querySelector('#two'))`);
        // Keep the last observed bytes; no credit between the live swap and click.
        await frame.locator('button').nth(0).click();await drain();
        assert.deepEqual(await evaluate('window.clicked'),[0,0],'MP-11: stale displayed native tile never dispatches the swapped live target');
        const failures=await viewer.evaluate(()=>window.inputFailures);
        assert.equal(failures.length,1);assert(!failures[0].includes('stale mirror input epoch'),'MP-11: changed target is never a sequence-only retry');
        await next();await next();await frame.locator('button').nth(1).click();await drain();
        assert.deepEqual(await evaluate('window.clicked'),[1,0],'MP-08: refreshed geometry clicks the observed button normally');
        await next();await next();await viewer.evaluate(()=>window.inputFailures=[]);
        const capture=source.browser.inputCapture,run=capture.run.bind(capture);
        capture.run=async(...args)=>{await evaluate(`document.querySelector('#two').before(document.querySelector('#one'))`);return run(...args)};
        try{await frame.locator('button').nth(1).click();await drain();}finally{capture.run=run;}
        assert.deepEqual(await evaluate('window.clicked'),[1,0],'MP-11: layout change after resolution, during focus capture, refuses before dispatch');
        assert.equal((await viewer.evaluate(()=>window.inputFailures)).length,1);
        await next();await next();await viewer.evaluate(()=>window.inputFailures=[]);
        await evaluate(`document.querySelector('#one').style.marginLeft='8px'`);
        await frame.locator('button').nth(0).click();await drain();
        assert.deepEqual(await evaluate('window.clicked'),[1,0],'MP-11: the same hit ID with changed live geometry also refuses before dispatch');
        assert.equal((await viewer.evaluate(()=>window.inputFailures)).length,1);
        receipt.security.push({check:'MP-11 native tiles swap before resolution or during focus capture; same-ID geometry change refuses; refreshed click succeeds',passed:true});
        await evaluate(`new Promise(resolve=>{const frame=document.createElement('iframe');frame.style.cssText='display:block;margin-left:150px;margin-top:80px;border:3px solid;padding:7px;width:400px;height:160px';frame.onload=()=>{frame.contentDocument.querySelector('button').onclick=()=>window.nestedClicked++;resolve()};frame.srcdoc='<html><body style="margin:0"><button style="width:160px;height:50px">Nested</button></body></html>';window.nestedClicked=0;document.querySelector('main').append(frame);})`);
        await next();await next();await viewer.evaluate(()=>window.inputFailures=[]);
        await evaluate(`document.querySelector('iframe').style.marginLeft='154px'`);
        await frame.frameLocator('iframe').locator('button').click();await drain();
        assert.equal(await evaluate('window.nestedClicked'),0,'MP-11: changed frame ancestor refuses unchanged nested tile geometry');
        assert.equal((await viewer.evaluate(()=>window.inputFailures)).length,1);
        await next();await next();await frame.frameLocator('iframe').locator('button').click();await drain();
        assert.equal(await evaluate('window.nestedClicked'),1,'MP-08: refreshed nested geometry admits normal input');
        await next();await next();await viewer.evaluate(()=>window.inputFailures=[]);
        await evaluate(`document.querySelector('main').setAttribute('data-chariox-observation-protected','')`);
        await frame.frameLocator('iframe').locator('button').click();await drain();
        assert.equal(await evaluate('window.nestedClicked'),1,'MP-11: newly protected outer ancestor fences nested input before another observation');
        assert.equal((await viewer.evaluate(()=>window.inputFailures)).length,1);
        receipt.security.push({check:'MP-11 live frame geometry and newly protected outer ancestor refuse nested tile before refresh',passed:true});
      }
      await resource();
    } finally {
      source.mirror.evaluate=read;
      await viewer.evaluate(()=>window.mirror?.close()).catch(()=>{});
      await source.request({op:'close',tab_id:tab.tab_id,generation:opened.generation,observed_by:'drill'});
    }
  }
}

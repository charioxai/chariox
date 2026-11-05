import assert from 'node:assert/strict';

const persistenceScript = `
globalThis.checks={};
const attempt=async(k,f)=>{try{checks[k]=await f()}catch(e){checks[k]=e.name}};
await attempt('parent',()=>parent.document.body.innerText);
await attempt('conversation',()=>top.document.querySelector('#conversation').dataset.private);
await attempt('cookie',()=>{const before=document.cookie;document.cookie='av1=present;path=/';return before});
await attempt('local',()=>{const before=localStorage.getItem('draft');localStorage.setItem('draft','present');return before});
await attempt('session',()=>{const before=sessionStorage.getItem('draft');sessionStorage.setItem('draft','present');return before});
await attempt('idb',()=>new Promise((resolve,reject)=>{
  const r=indexedDB.open('av1-drafts',1);r.onupgradeneeded=()=>r.result.createObjectStore('drafts');r.onerror=()=>reject(r.error);
  r.onsuccess=()=>{const db=r.result,tx=db.transaction('drafts','readwrite'),store=tx.objectStore('drafts'),get=store.get('draft');
    get.onsuccess=()=>{const before=get.result??null;store.put('present','draft');tx.oncomplete=()=>{db.close();resolve(before)}};tx.onerror=()=>reject(tx.error)};
}));
await attempt('cache',async()=>{const c=await caches.open('av1-drafts'),before=await c.match('/av1-value');await c.put('/av1-value',new Response('present'));return before?await before.text():null});
await attempt('sw',async()=>{const before=await navigator.serviceWorker.getRegistration('/av1-worker/');await navigator.serviceWorker.register('/av1-worker/sw.js');return before?'present':null});
checks.origin=origin;checks.done=true;
`;

export async function isolationCases({ browser, server, pack }) {
  const results = { persistence: [] };
  for (const control of [true, false]) {
    server.setBoundary(control ? 'all' : null); server.setPhase(`persistence-${control ? 'control' : 'strict'}`);
    const context = await browser.newContext(); const page = await context.newPage();
    const visits = [];
    for (let visit = 0; visit < 2; visit++) {
      await server.open(page, await pack(persistenceScript));
      const frame = await server.frame(page);
      await frame.waitForFunction(() => globalThis.checks?.done, null, { timeout: 8000 });
      visits.push(await frame.evaluate(() => globalThis.checks));
    }
    results.persistence.push({ control, visits });
    await context.close();
  }
  server.setBoundary(null);
  const context = await browser.newContext(), page = await context.newPage();
  const compile = `globalThis.checks={};const attempt=(k,f)=>{try{checks[k]=f()}catch(e){checks[k]=e.name}};
    attempt('eval',()=>eval('2+2'));attempt('function',()=>new Function('return 4')());
    checks.rtc=typeof RTCPeerConnection;checks.rtcDescriptor=Object.getOwnPropertyDescriptor(globalThis,'RTCPeerConnection');
    attempt('replaceRtc',()=>{Object.defineProperty(globalThis,'RTCPeerConnection',{value:function(){}});return true});
    checks.wasm=await WebAssembly.compile(new Uint8Array([0,97,115,109,1,0,0,0])).then(()=>true,e=>e.name);`;
  await server.open(page, await pack(compile));
  let frame = await server.frame(page);
  await frame.waitForFunction(() => globalThis.checks?.wasm !== undefined);
  results.compilation = await frame.evaluate(() => globalThis.checks);
  server.setBoundary('wasm');
  await server.open(page, await pack(compile)); frame = await server.frame(page);
  await frame.waitForFunction(() => globalThis.checks?.wasm !== undefined);
  results.compilation.withoutWasmPermission = await frame.evaluate(() => globalThis.checks.wasm);
  server.setBoundary(null);

  await server.open(page, await pack(`document.getElementById('root').innerHTML='<input id="copy" value="native clipboard proof"><input id="paste"><input id="pick" type="file">';
    document.getElementById('pick').onchange=async e=>{const f=e.target.files[0];globalThis.picked={name:f.name,text:await f.text()}};`));
  frame = await server.frame(page);
  await frame.locator('#copy').click(); await page.keyboard.press('Control+A'); await page.keyboard.press('Control+C');
  await frame.locator('#paste').click(); await page.keyboard.press('Control+V');
  results.clipboard = await frame.locator('#paste').inputValue();
  const chooser = page.waitForEvent('filechooser'); await frame.locator('#pick').click();
  await (await chooser).setFiles({ name: 'native-picked.txt', mimeType: 'text/plain', buffer: Buffer.from('native file proof') });
  await frame.waitForFunction(() => globalThis.picked !== undefined);
  results.filePicker = await frame.evaluate(() => globalThis.picked);
  await context.close();

  const [red, green] = results.persistence.map(row => row.visits);
  results.passed = green.every(visit => visit.origin === 'null' && visit.parent === 'SecurityError' && visit.conversation === 'SecurityError'
    && visit.cookie === 'SecurityError' && visit.local === null && visit.session === null
    && ['TypeError', 'SecurityError'].includes(visit.idb) && visit.cache === 'SecurityError' && visit.sw === 'SecurityError')
    && red[1].local === 'present' && red[1].session === 'present' && red[1].cookie.includes('av1=present')
    && red[1].idb === 'present' && red[1].cache === 'present' && red[1].sw === 'present'
    && red[0].conversation === 'adjacent-private-canary';
  results.passed &&= results.compilation.eval === 'EvalError' && results.compilation.function === 'EvalError'
    && results.compilation.wasm === true && results.compilation.withoutWasmPermission === 'CompileError'
    && results.compilation.rtc === 'undefined' && results.compilation.rtcDescriptor.configurable === false
    && results.compilation.replaceRtc === 'TypeError' && results.clipboard === 'native clipboard proof'
    && results.filePicker.text === 'native file proof';
  return results;
}

export async function sourceBindingCases({ browser, server, pack }) {
  const rows = [];
  for (const control of [true, false]) {
    server.setBoundary(control ? 'source' : null); server.setPhase(`source-binding-${control ? 'control' : 'strict'}`);
    const context = await browser.newContext(), page = await context.newPage();
    await page.goto(server.origin + '/probe.html');
    await page.evaluate(() => {
      const host = document.createElement('iframe');host.id='host';globalThis.hostReady=false;
      addEventListener('message',event=>{if(event.source===host.contentWindow&&event.data?.type==='chariox.app-host.ready')globalThis.hostReady=true});
      host.src='/assets/app-view-host/host.html';document.body.append(host);
      const sibling=document.createElement('iframe');sibling.id='sibling';document.body.append(sibling);
    });
    await page.waitForFunction(() => globalThis.hostReady);
    const sibling = page.frames().find(f => f.parentFrame() === page.mainFrame() && f.url() === 'about:blank');
    assert(sibling);
    await sibling.evaluate(packed => {
      const channel=new MessageChannel();globalThis.reply=null;channel.port1.onmessage=e=>{globalThis.reply=e.data;channel.port1.close()};
      parent.document.querySelector('#host').contentWindow.postMessage({type:'chariox.app-host.attach',...packed},parent.location.origin,[channel.port2]);
    }, await pack('globalThis.publisherRan=true;'));
    await page.waitForTimeout(300);
    const reply=await sibling.evaluate(()=>globalThis.reply);
    const host=page.frames().find(f=>f.url().endsWith('/assets/app-view-host/host.html'));
    rows.push({control,reply,appFrames:host.childFrames().length});
    await context.close();
  }
  server.setBoundary(null);
  return { rows, passed: rows[0].reply?.type === 'chariox.app-host.mounted' && rows[1].reply === null && rows[1].appFrames === 0 };
}

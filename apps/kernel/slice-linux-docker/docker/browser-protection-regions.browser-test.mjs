// Opt-in MP-08/MP-11: real Chromium, every frame incl. isolated ones, DPR 1 and 2.
// Oracle: CDP viewport pixels. Protected content (magenta) is fully covered;
// ordinary content (cyan: consent dialog, cross-site captcha frame, article)
// stays visible. Desktop placement is proven by the Xvfb drill.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { decodePng, encodePng, maskPng } from './kernel-browser-pixels.mjs';
import { ProtectionGate, awaitPresented, fenceBrowserCapture, measureBrowserProtection } from './browser-protection-regions.mjs';
import { locateBrowserRegions } from './browser-observation-regions.mjs';
import { REORDERED, VAULT_VALUE, census, launchChromium, openFixture, serveFixture } from './browser-protection-fixture.mjs';
const executable = process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE;
assert.ok(executable, 'Explicit installed Chromium required; never download a browser');

async function withFixture(dpr, run, query = '') {
  const root = await mkdtemp(path.join(tmpdir(), 'cx-protection-'));
  const fixture = await serveFixture();
  const chromium = await launchChromium({ executable, dpr, root, headless: process.env.CHARIOX_PROTECTION_TEST_HEADED !== '1' });
  try {
    const opened = await openFixture(chromium.browser, fixture.url + query);
    const shot = async () => decodePng((await opened.connection.send('Page.captureScreenshot', { format: 'png' }, opened.sessionId)).data, dpr);
    await run({ ...chromium, ...opened, shot });
  } finally { await chromium.close(); await fixture.close(); await rm(root, { recursive: true, force: true }); }
}
const policy = (values = []) => ({ values, targets: [], unknown: false });

for (const dpr of [1, 2]) for (const collector of ['documentProtection', 'locateBrowserRegions']) {
  test(`MP-08/MP-11 DPR ${dpr} ${collector}: inherited RTL keeps ordinary controls visible`, () => withFixture(dpr, async ({ browser, connection, sessionId, targetId, shot }) => {
    await connection.send('Runtime.evaluate', { expression: `{
      document.documentElement.dir='rtl'; document.documentElement.lang='ar';
      document.body.innerHTML='<button id="control" onclick="this.textContent=&quot;Continued&quot;" style="position:fixed;left:20px;top:20px;width:180px;height:80px;background:#00ffff;border:0">Continue</button><h1 style="position:fixed;left:550px;top:40px;width:350px;font:24px sans-serif">مرحبا שלום</h1>';
      document.body.style.margin='0';
    }` }, sessionId);
    let targets = [];
    const collect = collector === 'documentProtection'
      ? async values => (await measureBrowserProtection(browser, policy(values))).pages[0].regions
      : values => locateBrowserRegions(targets, browser, values, { contentTarget: targetId, contentScale: dpr });
    const records = [];
    const check = async (stage, values, protectedPixels) => {
      const pixels = await shot(), start = performance.now(), regions = await collect(values), elapsed = performance.now()-start;
      const raw = census(pixels), after = census(pixels, regions.map(([x,y,w,h]) => [Math.floor(x),Math.floor(y),Math.ceil(x+w)-Math.floor(x),Math.ceil(y+h)-Math.floor(y)]));
      records.push({ stage, elapsed, raw, after, regions });
      if (process.env.CHARIOX_PROTECTION_TEST_EVIDENCE) {
        const root = process.env.CHARIOX_PROTECTION_TEST_EVIDENCE, data = encodePng(pixels.width,pixels.height,pixels.pixels);
        await writeFile(path.join(root,`rtl-dpr${dpr}-${collector}-${stage}-raw.png`),Buffer.from(data,'base64'));
        await writeFile(path.join(root,`rtl-dpr${dpr}-${collector}-${stage}-masked.png`),Buffer.from(maskPng(data,regions,dpr),'base64'));
        await writeFile(path.join(root,`rtl-dpr${dpr}-${collector}.json`),JSON.stringify({items:['MP-08','MP-11'],dpr,collector,records}));
      }
      assert(raw.cyan>10000*dpr*dpr,'MP-08 ordinary control is rendered');
      assert(after.cyan>=raw.cyan*.95,`MP-08 ${stage}: ordinary control stays visible (${after.cyan}/${raw.cyan})`);
      if (protectedPixels === true) assert.equal(after.magenta,0,'MP-11 field, direct echo and local reordered run stay covered');
      if (protectedPixels === false) assert(after.magenta>100*dpr*dpr,'MP-11 unrelated registration leaves the direct echo visible');
    };
    await check('ordinary', ['unrelated-value']);
    await connection.send('Runtime.evaluate', { expression: `document.body.insertAdjacentHTML('beforeend', ${JSON.stringify('<input id="password" type="password" value="'+VAULT_VALUE+'" style="position:fixed;left:550px;top:160px;width:280px;height:40px;background:#ff00ff;border:0"><p style="position:fixed;left:550px;top:230px;margin:0;color:#ff00ff;font:bold 20px monospace">'+VAULT_VALUE+'</p><p style="position:fixed;left:550px;top:300px;width:350px;margin:0;color:#ff00ff;font:bold 20px monospace"><bdo dir="rtl">'+[...VAULT_VALUE].reverse().join('')+'</bdo></p>')})` }, sessionId);
    // The production observation path receives the registered field reference.
    const { root } = await connection.send('DOM.getDocument', {}, sessionId);
    const { nodeId } = await connection.send('DOM.querySelector', { nodeId:root.nodeId, selector:'#password' }, sessionId);
    const { node } = await connection.send('DOM.describeNode', { nodeId }, sessionId);
    targets = [{ target_id:targetId, node_ref:`backend:${node.backendNodeId}` }];
    await check('unrelated', ['unrelated-value'], false);
    await check('protected', [VAULT_VALUE], true);
    await connection.send('Input.dispatchMouseEvent', { type:'mousePressed', x:100, y:50, button:'left', clickCount:1 }, sessionId);
    await connection.send('Input.dispatchMouseEvent', { type:'mouseReleased', x:100, y:50, button:'left', clickCount:1 }, sessionId);
    const { result } = await connection.send('Runtime.evaluate', { returnByValue:true, expression:`document.getElementById('control').textContent` }, sessionId);
    assert.equal(result.value,'Continued');
    await check('after-control', [VAULT_VALUE], true);
  }));
}

// MP-08/MP-11: field-reference coverage is independent of matching value echoes.
for (const dpr of [1, 2]) test(`MP-08/MP-11 DPR ${dpr}: registered field quads cover content pixels`, () => withFixture(dpr, async ({ browser, connection, sessionId, targetId, shot }) => {
  await connection.send('Runtime.evaluate', { expression:`document.body.innerHTML='<input id="password" type="password" style="position:fixed;left:550px;top:160px;width:280px;height:40px;background:#ff00ff;border:0"><button style="position:fixed;left:20px;top:20px;width:180px;height:80px;background:#00ffff;border:0">Continue</button>'` }, sessionId);
  const { root } = await connection.send('DOM.getDocument', {}, sessionId);
  const { nodeId } = await connection.send('DOM.querySelector', { nodeId:root.nodeId, selector:'#password' }, sessionId);
  const { node } = await connection.send('DOM.describeNode', { nodeId }, sessionId);
  const regions = await locateBrowserRegions([{target_id:targetId,node_ref:`backend:${node.backendNodeId}`}],browser,[],{contentTarget:targetId,contentScale:dpr});
  const pixels = await shot(), raw = census(pixels), after = census(pixels,regions);
  if (process.env.CHARIOX_PROTECTION_TEST_EVIDENCE) {
    const out=process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,data=encodePng(pixels.width,pixels.height,pixels.pixels);
    await writeFile(path.join(out,`field-dpr${dpr}-raw.png`),Buffer.from(data,'base64'));
    await writeFile(path.join(out,`field-dpr${dpr}-masked.png`),Buffer.from(maskPng(data,regions,dpr),'base64'));
    await writeFile(path.join(out,`field-dpr${dpr}.json`),JSON.stringify({items:['MP-08','MP-11'],dpr,raw,after,regions}));
  }
  assert(raw.magenta>10000*dpr*dpr,'MP-11 empty protected field renders canary pixels');
  assert(after.cyan>=raw.cyan*.95,'MP-08 ordinary control remains visible');
  assert.equal(after.magenta,0,'MP-11 registered field is covered at its capture coordinates');
}));

for (const dpr of [1, 2]) for (const kind of ['huge', 'overlap']) for (const collector of ['documentProtection', 'locateBrowserRegions']) {
  test(`MP-08/MP-11 DPR ${dpr} ${collector}: ${kind} glyph grid has a deadline`, { timeout: 15000 }, () => withFixture(dpr, async ({ browser, connection, sessionId, targetId, shot }) => {
    await connection.send('Runtime.evaluate', { expression: `{
      document.body.replaceChildren(); document.body.style.margin='0';
      const control=document.createElement('div'); control.style.cssText='position:fixed;left:20px;top:20px;width:180px;height:80px;background:#00ffff'; document.body.append(control);
      const box=document.createElement('div'); box.style.cssText='position:fixed;left:550px;top:100px;width:350px;height:100px;color:#ff00ff;font:16px sans-serif;transform:translateX(1px)'; document.body.append(box);
      if (${JSON.stringify(kind)}==='huge') { const glyph=document.createElement('span'); glyph.style.cssText='display:inline-block;transform-origin:top left;transform:scale(100000)'; glyph.textContent='X';box.append(glyph); }
      else { box.style.lineHeight='0'; box.innerHTML='<span>X</span><br>'.repeat(2000); }
    }` }, sessionId);
    const collectors = { documentProtection: async () => (await measureBrowserProtection(browser, policy(['unrelated-value']))).pages[0].regions,
      locateBrowserRegions: () => locateBrowserRegions([], browser, ['unrelated-value'], { contentTarget: targetId, contentScale: dpr }) };
    const pixels = await shot();
    for (const [name, collect] of [[collector, collectors[collector]]]) {
      const start = performance.now(), regions = await collect(), elapsed = performance.now() - start;
      assert(elapsed < 2000, `MP-11 ${name} took ${elapsed} ms`);
      if (kind === 'overlap') assert(regions.some(([x,y,w,h]) => x <= 552*dpr && y <= 100*dpr && x+w >= 900*dpr && y+h >= 200*dpr), `MP-11 ${name} covers just the uncertain container`);
      assert(census(pixels, regions).cyan >= 180*80*dpr*dpr*.95, `MP-08 ${name} leaves unrelated content visible`);
      if (process.env.CHARIOX_PROTECTION_TEST_EVIDENCE) {
        const root = process.env.CHARIOX_PROTECTION_TEST_EVIDENCE, data = encodePng(pixels.width, pixels.height, pixels.pixels);
        await writeFile(path.join(root,`grid-${kind}-dpr${dpr}-raw.png`),Buffer.from(data,'base64'));
        await writeFile(path.join(root,`grid-${kind}-dpr${dpr}-${name}.png`),Buffer.from(maskPng(data,regions,dpr),'base64'));
        await writeFile(path.join(root,`grid-${kind}-dpr${dpr}-${name}.json`),JSON.stringify({items:['MP-08','MP-11'],elapsed,regions}));
      }
    }
  }));
}

for(const dpr of [1,2])test(`MP-08/MP-11 DPR ${dpr}: reversed flex overflow and zero-sized containers are covered by both collectors`,()=>withFixture(dpr,async({browser,connection,sessionId,targetId,shot})=>{
  const a=VAULT_VALUE.slice(0,17),b=VAULT_VALUE.slice(17);
  const {result}=await connection.send('Runtime.evaluate',{returnByValue:true,expression:`{const crops=[];for(const [top,w,h] of [[100,1,24],[Math.min(innerHeight-40,650),0,0]]){const box=document.createElement('div');box.style.cssText='position:fixed;left:550px;top:'+top+'px;width:'+w+'px;height:'+h+'px;display:flex;flex-direction:row-reverse;color:#ff00ff;white-space:nowrap;font:bold 18px sans-serif;z-index:2147483647';for(const text of ${JSON.stringify([b,a])}){const span=document.createElement('span');span.style.cssText='flex-shrink:0;background:#fff';span.textContent=text;box.append(span);}document.body.append(box);const range=document.createRange();range.selectNodeContents(box);const r=range.getBoundingClientRect();crops.push([r.left,r.top,r.width,r.height]);}crops}`},sessionId);
  const pixels=await shot();
  const crops=result.value.map(([x,y,w,h])=>[Math.floor(x*dpr),Math.floor(y*dpr),Math.ceil((x+w)*dpr)-Math.floor(x*dpr),Math.ceil((y+h)*dpr)-Math.floor(y*dpr)]);
  const crop=([x,y,w,h])=>({width:w,pixels:Buffer.concat(Array.from({length:h},(_,row)=>pixels.pixels.subarray(((y+row)*pixels.width+x)*4,((y+row)*pixels.width+x+w)*4)))});
  const collectors={transform:async values=>(await measureBrowserProtection(browser,policy(values))).pages[0].regions,
    panel:values=>locateBrowserRegions([],browser,values,{contentTarget:targetId,contentScale:dpr})};
  const round=regions=>regions.map(([x,y,w,h])=>[Math.floor(x),Math.floor(y),Math.ceil(x+w)-Math.floor(x),Math.ceil(y+h)-Math.floor(y)]);
  const inBox=([x,y],regions)=>regions.map(([rx,ry,w,h])=>[rx-x,ry-y,w,h]);
  const exposed=[];
  for(const [name,collect]of Object.entries(collectors)){
    const without=round(await collect([])),withValue=round(await collect([VAULT_VALUE]));
    if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE){
      const data=encodePng(pixels.width,pixels.height,pixels.pixels),root=process.env.CHARIOX_PROTECTION_TEST_EVIDENCE;
      await writeFile(path.join(root,`overflow-dpr${dpr}-raw.png`),Buffer.from(data,'base64'));
      await writeFile(path.join(root,`overflow-dpr${dpr}-${name}.png`),Buffer.from(maskPng(data,withValue,dpr),'base64'));
    }
    for(const box of crops){
      assert(census(crop(box),inBox(box,without)).magenta>100*dpr*dpr,'MP-11 overflow canary pixels are visible without a saved value');
      const left=census(crop(box),inBox(box,withValue)).magenta;if(left)exposed.push(`${name}: ${left} px`);
    }
    const control=[20,20,180,100].map(v=>v*dpr);
    const ordinary=census(crop(control),inBox(control,without)),masked=census(crop(control),inBox(control,withValue));
    assert(masked.cyan>ordinary.cyan*.95,`MP-08 ${name} unrelated ordinary content stays visible (${masked.cyan}/${ordinary.cyan})`);
  }
  assert.deepEqual(exposed,[],'MP-11 both collectors cover all overflowing value pixels');
}));

for (const dpr of [1, 2]) for (const collector of ['documentProtection', 'locateBrowserRegions']) {
  test(`MP-08/MP-11 DPR ${dpr} ${collector}: scaled 2px text and ordinary text share value protection`, () => withFixture(dpr, async ({ browser, connection, sessionId, targetId, shot }) => {
    const [first, second] = [VAULT_VALUE.slice(0, 12), VAULT_VALUE.slice(12)];
    const { result } = await connection.send('Runtime.evaluate', { returnByValue: true, expression: `{
      document.body.replaceChildren(); document.body.style.margin='0';
      const control=document.createElement('div'); control.style.cssText='position:fixed;left:20px;top:20px;width:180px;height:80px;background:#00ffff'; document.body.append(control);
      const line=document.createElement('div'); line.style.cssText='position:fixed;left:550px;top:100px;width:350px;height:80px;display:flex;flex-direction:row-reverse;justify-content:flex-end;align-items:flex-start;color:#ff00ff;white-space:nowrap;font:20px monospace';
      const ordinary=document.createElement('span');ordinary.textContent=${JSON.stringify(second)};
      const small=document.createElement('span'); small.textContent=${JSON.stringify(first)};
      small.style.cssText='font-size:2px;transform:scale(10);transform-origin:top left;margin-right:'+(${first.length}*12-${first.length}*1.2)+'px';
      line.append(ordinary,small);document.body.append(line);
      const bounds=node=>{const range=document.createRange();range.selectNodeContents(node);const r=range.getBoundingClientRect();return [r.left,r.top,r.width,r.height];};
      const r=line.getBoundingClientRect(); ({line:[r.left,r.top,r.width,r.height],pieces:[bounds(small),bounds(ordinary)]});
    }` }, sessionId);
    const pixels = await shot(), box = result.value.line.map(v => Math.round(v*dpr));
    const [x,y,w,h] = box;
    const crop = { width:w, pixels:Buffer.concat(Array.from({length:h}, (_,row) => pixels.pixels.subarray(((y+row)*pixels.width+x)*4,((y+row)*pixels.width+x+w)*4))) };
    const collect = collector === 'documentProtection'
      ? async values => (await measureBrowserProtection(browser,policy(values))).pages[0].regions
      : values => locateBrowserRegions([],browser,values,{contentTarget:targetId,contentScale:dpr});
    const without = await collect([]), start = performance.now(), regions = await collect([VAULT_VALUE]), elapsed = performance.now()-start;
    const local = masks => masks.map(([rx,ry,rw,rh]) => [Math.floor(rx)-x,Math.floor(ry)-y,Math.ceil(rx+rw)-Math.floor(rx),Math.ceil(ry+rh)-Math.floor(ry)]);
    const before=census(crop,local(without)).magenta, exposed=census(crop,local(regions)).magenta;
    const pieces=result.value.pieces.map(([left,top,width,height])=>{
      const x0=Math.floor(left*dpr),y0=Math.floor(top*dpr),x1=Math.ceil((left+width)*dpr),y1=Math.ceil((top+height)*dpr);
      const part={width:x1-x0,pixels:Buffer.concat(Array.from({length:y1-y0},(_,row)=>pixels.pixels.subarray(((y0+row)*pixels.width+x0)*4,((y0+row)*pixels.width+x1)*4)))};
      return census(part).magenta;
    });
    if (process.env.CHARIOX_PROTECTION_TEST_EVIDENCE) {
      const root=process.env.CHARIOX_PROTECTION_TEST_EVIDENCE, data=encodePng(pixels.width,pixels.height,pixels.pixels);
      await writeFile(path.join(root,`scaled-dpr${dpr}-${collector}-raw.png`),Buffer.from(data,'base64'));
      await writeFile(path.join(root,`scaled-dpr${dpr}-${collector}-masked.png`),Buffer.from(maskPng(data,regions,dpr),'base64'));
      await writeFile(path.join(root,`scaled-dpr${dpr}-${collector}.json`),JSON.stringify({items:['MP-08','MP-11'],dpr,collector,before,pieces,exposed,elapsed,regions}));
    }
    assert(pieces.every(count=>count>50*dpr*dpr),'MP-11 scaled and ordinary value pieces are each visible');
    assert(before>100*dpr*dpr,'MP-11 value is rendered without protection');
    assert(census(pixels,regions).cyan>=180*80*dpr*dpr*.95,'MP-08 unrelated content remains visible');
    assert.equal(exposed,0,'MP-11 scaled and ordinary value pieces are covered');
  }));
}

// Vault policy: whole-page DOMSnapshot path (echoes, markers incl. frame owners).
// No policy on a fields-only page: selector-search path, no DOMSnapshot.
for (const dpr of [1, 2]) for (const [label, values, query] of [['snapshot', [VAULT_VALUE], ''], ['search', [], '?novault&nomarkers']]) {
  test(`DPR ${dpr} ${label}: every protected frame region is covered; ordinary content stays visible`, () => withFixture(dpr, async ({ browser, connection, sessionId, shot }) => {
    for (const scroll of [0, 300]) {
      await connection.send('Runtime.evaluate', { expression: `scrollTo(0, ${scroll})` }, sessionId);
      let snapshots = 0;
      const send = connection.send.bind(connection);
      connection.send = (method, ...rest) => { if (method === 'DOMSnapshot.captureSnapshot') snapshots++; return send(method, ...rest); };
      const { pages } = await measureBrowserProtection(browser, policy(values)).finally(() => { connection.send = send; });
      assert.equal(snapshots > 0, label === 'snapshot', `${label} path`);
      assert.equal(pages.length, 1);
      const [page] = pages;
      assert.equal(page.dpr, dpr);
      const pixels = await shot();
      assert.deepEqual(page.viewport, [pixels.width, pixels.height]);
      const before = census(pixels), after = census(pixels, page.regions);
      assert.ok(before.magenta > 1000 * dpr * dpr, 'fixture shows protected pixels');
      assert.equal(after.magenta, 0, `scroll ${scroll}: protected pixels remain`);
      // Every frame is inspected; only the scaled, mirrored and rotated frames are withheld whole
      // (the selector search places the rotated in-process frame's field from its transformed quad).
      assert.ok(page.withheld.every(reason => reason === 'transformed_frame') && (scroll || page.withheld.length === (values.length ? 3 : 2)), String(page.withheld));
      // Ordinary content incl. the cross-site captcha frame is not withheld.
      assert.ok(after.cyan > before.cyan * 0.95, `scroll ${scroll}: ordinary content masked (${after.cyan}/${before.cyan})`);
    }
  }, query));
}

// The canvas draws the value, an adopted-stylesheet ::before/::after around
// nested inline text generates it, and sibling spans split it; the scripts are
// gone: no DOM string echoes it, only the rendered layout text.
for (const dpr of [1, 2]) test(`DPR ${dpr}: Vault echoes and opaque media are protected only while values are registered`, () => withFixture(dpr, async ({ browser, connection, sessionId, shot }) => {
  const { result } = await connection.send('Runtime.evaluate', { returnByValue: true,
    expression: `[...document.querySelectorAll('script,style')].some(source => source.text?.includes(${JSON.stringify(VAULT_VALUE)}) || source.textContent.includes(${JSON.stringify(VAULT_VALUE.slice(0, 12))})) ? null : ['drawn', 'media', 'generated', 'siblings'].map(id => document.getElementById(id).getBoundingClientRect()).map(b => [b.left, b.top, b.width, b.height])` }, sessionId);
  assert.ok(result.value, 'the drawing and styling scripts are gone');
  const [drawn, media, generated, siblings] = result.value.map(box => box.map(v => Math.round(v * dpr)));
  const pixels = await shot();
  const without = (await measureBrowserProtection(browser, policy())).pages[0];
  const withValue = (await measureBrowserProtection(browser, policy([VAULT_VALUE]))).pages[0];
  const crop = ([x, y, w, h]) => ({ width: w, pixels: Buffer.concat(Array.from({ length: h }, (_, row) => pixels.pixels.subarray(((y + row) * pixels.width + x) * 4, ((y + row) * pixels.width + x + w) * 4))) });
  const inBox = ([x, y, w, h], regions) => regions.map(([rx, ry, rw, rh]) => [rx - x, ry - y, rw, rh]);
  assert.ok(census(crop(drawn), inBox(drawn, without.regions)).magenta > 50 * dpr * dpr, 'the drawn value is ordinary without a Vault value');
  assert.ok(census(crop(media), inBox(media, without.regions)).cyan === media[2] * media[3], 'media stay visible without a Vault value');
  assert.ok(census(crop(generated), inBox(generated, without.regions)).magenta > 50 * dpr * dpr, 'the generated value is ordinary without a Vault value');
  assert.ok(census(pixels, without.regions).magenta > 0, 'the echo paragraph is ordinary without a Vault value');
  assert.ok(census(crop(siblings), inBox(siblings, without.regions)).magenta > 50 * dpr * dpr, 'the split value is ordinary without a Vault value');
  assert.equal(census(crop(generated), inBox(generated, withValue.regions)).magenta, 0, 'the generated value is masked');
  assert.equal(census(crop(siblings), inBox(siblings, withValue.regions)).magenta, 0, 'the split value is masked');
  assert.equal(census(pixels, withValue.regions).magenta, 0, 'echoes, the canvas-drawn, generated and split values are masked');
  assert.equal(census(crop(media), inBox(media, withValue.regions)).cyan, 0, 'every medium is masked while a value is registered');
}));

// Flex row-reverse, CSS order, grid placement, absolute positioning, a bidi
// override, a right-to-left line and a font-size:0 separator show the value
// although DOM order never spells it. Both collectors (desktop transform and
// Browser-panel masks) mask each line only while a value is registered.
for (const dpr of [1, 2]) test(`DPR ${dpr}: containers whose visual order differs from DOM order are masked by both collectors`, () => withFixture(dpr, async ({ browser, connection, sessionId, targetId, shot }) => {
  const { result } = await connection.send('Runtime.evaluate', { returnByValue: true,
    expression: `${JSON.stringify(REORDERED)}.map(id => document.getElementById('order-' + id).getBoundingClientRect()).map(b => [b.left, b.top, b.width, b.height])` }, sessionId);
  const lines = result.value.map(box => box.map(v => Math.round(v * dpr)));
  const pixels = await shot();
  const crop = ([x, y, w, h]) => ({ width: w, pixels: Buffer.concat(Array.from({ length: h }, (_, row) => pixels.pixels.subarray(((y + row) * pixels.width + x) * 4, ((y + row) * pixels.width + x + w) * 4))) });
  // Outward-rounded, as masks are painted.
  const inBox = ([x, y], regions) => regions.map(([rx, ry, rw, rh]) => [Math.floor(rx) - x, Math.floor(ry) - y, Math.ceil(rx + rw) - Math.floor(rx), Math.ceil(ry + rh) - Math.floor(ry)]);
  const panel = values => locateBrowserRegions([], browser, values, { contentTarget: targetId, contentScale: dpr });
  const collectors = { transform: async values => (await measureBrowserProtection(browser, policy(values))).pages[0].regions, panel };
  const hidden = [], exposed = [];
  for (const [name, collect] of Object.entries(collectors)) {
    const without = await collect([]), withValue = await collect([VAULT_VALUE]);
    lines.forEach((line, i) => {
      if (census(crop(line), inBox(line, without)).magenta <= 100 * dpr * dpr) hidden.push(`${name} ${REORDERED[i]}`);
      const left = census(crop(line), inBox(line, withValue)).magenta;
      if (left) exposed.push(`${name} ${REORDERED[i]}: ${left} px`);
    });
  }
  assert.deepEqual(hidden, [], 'ordinary without a Vault value');
  assert.deepEqual(exposed, [], 'reordered values are masked');
}));

test('a captcha frame stays visible; its region is not protected', () => withFixture(1, async ({ browser, connection, sessionId }) => {
  const { result } = await connection.send('Runtime.evaluate', { returnByValue: true, expression: 'JSON.stringify(document.getElementById("captcha").getBoundingClientRect())' }, sessionId);
  const box = JSON.parse(result.value);
  const { pages } = await measureBrowserProtection(browser, policy());
  const overlap = pages[0].regions.filter(([x, y, w, h]) => x < box.right && x + w > box.left && y < box.bottom && y + h > box.top);
  assert.deepEqual(overlap, []);
}));

test('layout moved between measurement and capture re-measures, then fails closed', () => withFixture(1, async ({ browser, connection, sessionId }) => {
  const calls = [];
  const once = await fenceBrowserCapture(browser, policy(), async protection => {
    calls.push(protection);
    if (calls.length === 1) await connection.send('Runtime.evaluate', { expression: 'scrollBy(0, 120)' }, sessionId);
    return calls.length;
  });
  assert.equal(once, 2, 'the moved capture is discarded and re-captured under a fresh measurement');
  assert.notDeepEqual(calls[1].pages[0].regions, calls[0].pages[0].regions);
  const moving = [];
  const result = await fenceBrowserCapture(browser, policy(), async protection => {
    moving.push(protection);
    if (protection) await connection.send('Runtime.evaluate', { expression: 'scrollBy(0, 40)' }, sessionId);
    return protection;
  });
  assert.equal(result, null, 'continuously moving layout is captured fail-closed');
  assert.deepEqual(moving.map(Boolean), [true, true, true, false]);
}));

// A failed DOM.getBoxModel (error or timeout) is not proof of absent layout:
// with repeated failures for the password fields or for the frame owners
// (isolated login frame included), no protected pixel is released by a fenced
// capture or by a stream frame.
for (const failing of ['input', 'iframe']) test(`failed ${failing} box lookups never release protected pixels`, () => withFixture(1, async ({ browser, connection, sessionId, shot }) => {
  const { root } = await connection.send('DOM.getDocument', { depth: -1 }, sessionId);
  const { nodeIds } = await connection.send('DOM.querySelectorAll', { nodeId: root.nodeId, selector: failing }, sessionId);
  const failed = new Set(await Promise.all(nodeIds.map(async nodeId => (await connection.send('DOM.describeNode', { nodeId }, sessionId)).node.backendNodeId)));
  assert.ok(failed.size >= 3);
  const send = connection.send.bind(connection);
  connection.send = (method, params, ...rest) => (method === 'DOM.getBoxModel' && failed.has(params?.backendNodeId) ? Promise.reject(new Error('injected box failure')) : send(method, params, ...rest));
  try {
    const fenced = await fenceBrowserCapture(browser, policy(), async protection => protection && census(await shot(), protection.pages[0].regions));
    assert.equal(fenced?.magenta, 0, 'fenced capture releases protected pixels');
    const gate = new ProtectionGate(async () => { await awaitPresented(browser, gate.protection?.pages ?? [], 1); return measureBrowserProtection(browser, policy()); });
    await gate.step(); await gate.step();
    const serial = gate.protectionSerial, frame = await shot();
    assert.ok(serial > 0, 'stable protection adopted');
    assert.equal((await gate.step()).verified, serial, 'frame released');
    assert.equal(census(frame, gate.protection.pages[0].regions).magenta, 0, 'released stream frame shows protected pixels');
  } finally { connection.send = send; }
}, '?novault&nomarkers'));

test('unknown protection fails closed', () => withFixture(1, async ({ browser }) => {
  await assert.rejects(measureBrowserProtection(browser, { values: [], targets: [], unknown: true }));
  assert.equal(await fenceBrowserCapture(browser, { values: [], targets: [], unknown: true }, async protection => protection), null);
}));

test('a visible browser-internal page (settings, extensions, DevTools) fails closed', () => withFixture(1, async ({ browser, connection, sessionId }) => {
  await connection.send('Page.navigate', { url: 'chrome://version/' }, sessionId);
  for (let i = 0; i < 50 && (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree.frame.url !== 'chrome://version/'; i++) await new Promise(resolve => setTimeout(resolve, 50));
  await assert.rejects(measureBrowserProtection(browser, policy()), /browser-internal page/);
}));

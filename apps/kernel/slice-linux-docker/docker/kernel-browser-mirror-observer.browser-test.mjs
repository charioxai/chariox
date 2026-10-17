// MP-08/MP-10/MP-11: the v1 mirror observer on real Chromium. A read that finds nothing changed does no
// computed-style work, and every incremental read equals a fresh full read of the same page.
// Opt-in: set CHARIOX_KERNEL_BROWSER_EXECUTABLE and CHARIOX_MDACCESS_DRILL_ROOT (a disposable directory).
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { launchChromium } from './browser-protection-fixture.mjs';
import { mirrorObserverExpression } from './kernel-browser-mirror-observer.mjs';

const executable = process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, drillRoot = process.env.CHARIOX_MDACCESS_DRILL_ROOT;
const options = { skip: !executable || !drillRoot };

const page = `<style>
body{font:14px Arial;margin:8px}.a{padding:4px}.a.on{color:#c00}.a.on .b{color:#0a0}
button:hover{background:#9f9}button:active{color:#c00}input:focus{outline:2px solid #f0f}.tip{display:none}.card:hover .tip{display:block}.card{width:100px;height:30px}
.e{display:block;width:10px;height:10px}.box{appearance:none;width:14px;height:14px;border:1px solid #000}.box:checked{background:#f00}
@media (max-width:500px){p{color:#a0a}}
</style>
<div class=a id=a><span class=b>b</span></div><div class=c id=c>c</div>
<button id=btn>go</button><input id=inp><input type=checkbox class=box id=chk>
<div class=card id=card>card<div class=tip>tip</div></div>
<div id=wrap><div id=grow>g</div></div><div class=e id=e></div><p id=p1>one</p><p>two</p><ul id=list><li>1</li><li>2</li></ul><div id=host></div><div id=sticky style="position:sticky;top:0">st</div><div style="position:relative"><div id=filler style="height:20px"></div><div id=abs style="position:absolute">ab</div></div><div id=fixed style="position:fixed;right:0;bottom:0">fx</div><div style="height:1500px">tall</div>
<iframe id=fr width=200 height=60 srcdoc="<style>p{color:#c00}.on{color:#00c}</style><p id=q>framed</p>"></iframe>
<script>host.attachShadow({mode:'open'}).innerHTML='<style>b{color:#f60}</style><b>shadow</b><slot></slot>'</script>`;

let chrome, root;
async function session(html) {
  const { connection } = chrome;
  const { targetId } = await connection.send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true });
  const send = (method, params = {}) => connection.send(method, params, sessionId);
  await send('Page.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 800, height: 600, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url: `data:text/html;charset=utf-8,${encodeURIComponent(html)}` });
  const evaluate = async (expression, contextId) => {
    const reply = await send('Runtime.evaluate', { expression, contextId, returnByValue: true, awaitPromise: true });
    assert(!reply.exceptionDetails, reply.exceptionDetails?.exception?.description ?? 'evaluation failed');
    return reply.result.value;
  };
  for (let i = 0; i < 200 && await evaluate("document.readyState==='complete'&&location.protocol==='data:'").catch(() => false) !== true; i++) await delay(25);
  const { frameTree } = await send('Page.getFrameTree');
  const { executionContextId: world } = await send('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: 'mirror-observer-test' });
  const initial = await evaluate("(()=>{const n=document.createElement('div');n.style.all='initial';document.body.append(n);const s=getComputedStyle(n);const o=Object.fromEntries([...s].map(k=>[k,s.getPropertyValue(k)]));n.remove();return o})()");
  assert.equal(await evaluate(mirrorObserverExpression(initial), world), true);
  // The isolated world has its own prototypes: this counts only the observer's property reads.
  await evaluate("(()=>{const p=CSSStyleDeclaration.prototype,get=p.getPropertyValue;globalThis.__styleReads=0;p.getPropertyValue=function(...a){globalThis.__styleReads++;return get.apply(this,a)};return true})()", world);
  const records = new Map();
  let top;
  const read = async () => {
    await evaluate('globalThis.__styleReads=0', world);
    const packet = await evaluate("globalThis.__charioxMirror.read([],[],'test',false)", world);
    const reads = await evaluate('globalThis.__styleReads', world);
    for (const id of packet.removed) records.delete(id);
    for (const { style_index, ...node } of packet.nodes) records.set(node.id, style_index === undefined ? node : { ...node, style: packet.styles[style_index] });
    top = packet.root;
    return { packet, reads };
  };
  // The page as the viewer would rebuild it: ids stripped, so two loads of the same document compare equal.
  const tree = (id = top) => { const { id: _id, parent: _parent, children, ...rest } = records.get(id); return { ...rest, children: children.map(child => tree(child)) }; };
  return {
    read, tree, evaluate: expression => evaluate(expression), world: expression => evaluate(expression, world),
    // Chromium applies :hover on the next frame.
    move: async (x, y) => { await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y }); await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))'); },
    press: async (selector, type) => { const { x, y } = await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`); await send('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons: type === 'mousePressed' ? 1 : 0, clickCount: 1 }); },
    hover: async (selector, at = [0.5, 0.5]) => { const { x, y } = await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width*${at[0]},y:r.y+r.height*${at[1]}}})()`); await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y }); await evaluate('new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))'); },
    resize: width => send('Emulation.setDeviceMetricsOverride', { width, height: 600, deviceScaleFactor: 1, mobile: false }),
    close: () => connection.send('Target.closeTarget', { targetId }),
  };
}
const sessions = [];
const open = async html => { const s = await session(html); sessions.push(s); return s; };
test.before(async () => { if (!options.skip) { root = await mkdtemp(path.join(drillRoot, 'observer-')); chrome = await launchChromium({ executable, root }); } });
test.after(async () => { for (const s of sessions) await s.close().catch(() => {}); await chrome?.close(); if (root) await rm(root, { recursive: true, force: true }); });

// `act` runs against a page that has already been read once (incremental) and against a fresh page (full).
// `settle` is a read in between on the warm page only.
async function matchesFreshRead(html, act, { delayMs = 0 } = {}) {
  const warm = await open(html);
  await warm.read();
  await act(warm, () => warm.read());
  if (delayMs) await delay(delayMs);
  const incremental = await warm.read();
  const cold = await open(html);
  await act(cold, async () => {});
  if (delayMs) await delay(delayMs);
  await cold.read();
  assert.deepEqual(warm.tree(), cold.tree());
  return incremental;
}

test('a read that finds nothing changed does no computed-style work', options, async () => {
  const s = await open(page);
  const first = await s.read();
  assert(first.reads > 0);
  const second = await s.read();
  assert.equal(second.reads, 0, 'every element kept its previous style');
  assert.equal(second.packet.nodes.length, 0);
  assert.deepEqual(second.packet.removed, []);
});

// No author selector uses :hover/:active/:focus: only the user-agent rules for those states apply.
const plain = page.replace('button:hover{background:#9f9}button:active{color:#c00}', '').replace('input:focus{outline:2px solid #f0f}', '').replace('.card:hover .tip{display:block}', '');

test('hover and focus restyle only the elements themselves when no author selector depends on them', options, async () => {
  const first = await open(plain);
  const full = (await first.read()).reads;
  const hovered = await matchesFreshRead(plain, s => s.hover('#btn'));
  assert(hovered.reads < full / 4, `${hovered.reads} of ${full} property reads`);
  await matchesFreshRead(plain, s => s.evaluate('inp.focus()'));
  await matchesFreshRead(plain, async (s, settle) => { await s.hover('#btn'); await s.press('#btn', 'mousePressed'); await settle(); await s.press('#btn', 'mouseReleased'); await s.move(700, 500); });
});

test('scrolling re-evaluates only the positioned elements', options, async () => {
  const first = await open(page);
  const full = (await first.read()).reads;
  const scrolled = await matchesFreshRead(page, s => s.evaluate('scrollTo(0,400)'));
  assert(scrolled.reads < full / 4, `${scrolled.reads} of ${full} property reads`);
});

test('a text change re-evaluates only what it touched', options, async () => {
  const first = await open(page);
  const full = (await first.read()).reads;
  const incremental = await matchesFreshRead(page, s => s.evaluate("document.querySelector('#p1').firstChild.data='changed'"));
  assert(incremental.reads < full / 4, `${incremental.reads} of ${full} property reads`);
});

for (const [name, act, html] of [
  ['a class change restyles descendants and following siblings', s => s.evaluate("a.classList.add('on')")],
  ['an attribute on a following sibling', s => s.evaluate("c.setAttribute('style','margin-left:20px')")],
  ['a class change reaching a following sibling through a sibling selector', s => s.evaluate("a.classList.add('on')"), `${page}<style>.a.on + .c{color:#00c}</style>`],
  ['elements appended to a list', s => s.evaluate("list.append(document.createElement('li'));list.lastChild.textContent='3'")],
  ['an element removed from a list', s => s.evaluate('list.firstElementChild.remove()')],
  ['hover over an element whose :hover reveals a child', s => s.hover('#card')],
  ['hover entering then leaving a button', async s => { await s.hover('#btn'); await s.move(700, 500); }],
  ['hover entering a button', s => s.hover('#btn')],
  ['hover leaving a button after a read', async (s, settle) => { await s.hover('#btn'); await settle(); await s.move(700, 500); }],
  ['focus leaving a field after a read', async (s, settle) => { await s.evaluate('inp.focus()'); await settle(); await s.evaluate('inp.blur()'); }],
  ['a checkbox unchecked after a read', async (s, settle) => { await s.evaluate('chk.checked=true'); await settle(); await s.evaluate('chk.checked=false'); }],
  ['a button pressed and released after a read', async (s, settle) => { await s.hover('#btn'); await s.press('#btn', 'mousePressed'); await settle(); await s.press('#btn', 'mouseReleased'); }],
  ['a button held pressed', async s => { await s.hover('#btn'); await s.press('#btn', 'mousePressed'); }],
  ['a class change inside a same-origin frame', s => s.evaluate("fr.contentDocument.querySelector('#q').className='on'")],
  ['hover moving between elements', async s => { await s.hover('#btn'); await s.hover('#card'); }],
  ['focus moving into a field', s => s.evaluate('inp.focus()')],
  ['a checkbox checked through its property', s => s.evaluate('chk.checked=true')],
  ['an absolutely positioned element moved by what precedes it', s => s.evaluate("filler.style.height='60px'")],
  ['the page scrolled', s => s.evaluate('scrollTo(0,400)')],
  ['the page scrolled and back after a read', async (s, settle) => { await s.evaluate('scrollTo(0,400)'); await settle(); await s.evaluate('scrollTo(0,0)'); }],
  ['a sibling growing and shifting the layout', s => s.evaluate("c.style.height='120px'")],
  ['a viewport narrow enough to flip a media query', s => s.resize(400)],
  ['a rule inserted through the CSSOM', s => s.evaluate("document.styleSheets[0].insertRule('#p1{color:#0a0}')")],
  ['a style element whose text changes but not its rule count', s => s.evaluate("const st=document.querySelector('style');st.textContent=st.textContent.replace('.a{padding:4px}','.a{padding:4px;color:#609}')")],
  ['a child growing resizes its parent', s => s.evaluate("grow.style.height='80px'")],
  ['text added to an element a selector matches while empty', s => s.evaluate("e.textContent='x'"), `${page}<style>.e:empty{color:#c00}</style>`],
  ['the pointer leaving the page', async (s, settle) => { await s.hover('#btn'); await settle(); await s.move(-5, -5); }],
  ['a stylesheet disabled through the CSSOM', s => s.evaluate('document.styleSheets[0].disabled=true')],
  ['a shadow tree stylesheet edited', s => s.evaluate("host.shadowRoot.querySelector('style').textContent='b{color:#06f}'")],
  ['slotted light DOM added to a shadow host', s => s.evaluate("host.append(document.createElement('i'));host.lastChild.textContent='slotted'")],
]) test(`incremental read equals a fresh read: ${name}`, options, async () => { await matchesFreshRead(html ?? page, act); });

test('a rule property edited through the CSSOM is picked up by the next full verification', options, async () => {
  await matchesFreshRead(page, s => s.evaluate("document.styleSheets[0].cssRules[0].style.fontSize='20px'"), { delayMs: 1700 });
});

test('a stylesheet using :has() is served from the cache, and the host of a :has() is restyled with what changed below it', options, async () => {
  const has = `${page}<style>.holder:has(.on){color:#c00}.holder:has(.on) .tail{color:#06c}</style><div class=holder id=holder><i id=inner>x</i><b class=tail>t</b></div>`;
  const s = await open(has);
  const first = await s.read();
  assert.equal((await s.read()).reads, 0);
  await matchesFreshRead(has, w => w.evaluate("inner.className='on'"));
  await matchesFreshRead(has, async (w, settle) => { await w.evaluate("inner.className='on'"); await settle(); await w.evaluate("inner.className=''"); });
  const small = await matchesFreshRead(has, w => w.evaluate("document.querySelector('#p1').firstChild.data='changed'"));
  assert(small.reads < first.reads / 4);
});

test('a :has() with a sibling argument restyles the preceding sibling that hosts it', options, async () => {
  const sibling = `${page}<style>.holder:has(+ .on){color:#c00}.holder:has(~ .late .on){color:#06c}</style><div class=holder id=holder>h</div><i id=after>a</i><div class=late><b id=deep>d</b></div>`;
  const s = await open(sibling);
  const first = await s.read();
  assert.equal((await s.read()).reads, 0);
  await matchesFreshRead(sibling, w => w.evaluate("after.className='on'"));
  await matchesFreshRead(sibling, w => w.evaluate("deep.className='on'"));
  await matchesFreshRead(sibling, async (w, settle) => { await w.evaluate("after.className='on'"); await settle(); await w.evaluate("after.className=''"); });
  const small = await matchesFreshRead(sibling, w => w.evaluate("document.querySelector('#p1').firstChild.data='changed'"));
  assert(small.reads < first.reads / 4);
});

test('a size container that changes size restyles what its @container rules reach', options, async () => {
  const container = `${page}<style>.row{display:flex;width:400px}.cq{container-type:inline-size;flex:1}@container (min-width:250px){.in{color:#c00}}</style><div class=row><div class=cq id=cq><span class=in>q</span></div><div id=side style="width:100px">s</div></div>`;
  const s = await open(container);
  await s.read();
  assert.equal((await s.read()).reads, 0);
  await matchesFreshRead(container, w => w.evaluate("side.style.width='200px'"));
});

test('animated elements are restyled each read, everything else keeps its style', options, async () => {
  const animated = `${page}<style>@keyframes go{from{color:#f00}to{color:#00f}}#anim{animation:go 100s linear -5s paused}</style><div id=anim>x</div>`;
  const s = await open(animated);
  const first = await s.read();
  const second = await s.read();
  assert(second.reads > 0 && second.reads < first.reads / 4, `${second.reads} of ${first.reads} property reads`);
  await matchesFreshRead(animated, w => w.evaluate('document.getAnimations()[0].currentTime=60000'));
  await matchesFreshRead(animated, async (w, settle) => { await w.evaluate('document.getAnimations()[0].currentTime=60000'); await settle(); await w.evaluate('document.getAnimations()[0].cancel()'); });
});

test('hover reaches a subtree only where an author selector puts :hover in front of descendants', options, async () => {
  const menu = `${page}<style>.menu{padding:0 0 30px}.menu:hover .sub{color:#c00}.menu:hover{outline:1px solid #c00}</style><div class=menu id=menu>m<span class=sub id=sub>s</span></div>`;
  const first = await (await open(menu)).read();
  await matchesFreshRead(menu, w => w.hover('#menu', [0.5, 0.9]));
  await matchesFreshRead(menu, async (w, settle) => { await w.hover('#menu', [0.5, 0.9]); await settle(); await w.move(700, 500); });
  const elsewhere = await matchesFreshRead(menu, w => w.hover('#btn'));
  assert(elsewhere.reads < first.reads / 4, `${elsewhere.reads} of ${first.reads} property reads`);
});

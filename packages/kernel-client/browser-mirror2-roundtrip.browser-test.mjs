// MP-08/MP-10/MP-11 source and renderer: real Chromium source and renderer regression fixtures.
// Supplementary only: these anonymous local fixtures never establish real-service acceptance.
import test,{before,after} from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createRequire} from 'node:module';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {inflateRawSync,constants as zlib} from 'node:zlib';
import {execFileSync} from 'node:child_process';
import {KernelBrowserHost} from '../../apps/kernel/slice-linux-docker/docker/kernel-browser-host.mjs';
const tools=process.env.MIRROR_LAB_NODE_MODULES;
assert(tools&&path.isAbsolute(tools),'MP-10: public Playwright/esbuild tool directory required');
const {chromium}=createRequire(path.join(tools,'x.js'))('playwright-core');
let scratch,browser,viewerServer,viewerUrl;
before(async()=>{
  scratch=await mkdtemp(path.join(tmpdir(),'mirror2-roundtrip-'));
  const outfile=path.join(scratch,'renderer.js');
  execFileSync(path.join(tools,'.bin/esbuild'),[new URL('./src/browser-mirror2.ts',import.meta.url).pathname,'--bundle','--format=iife','--global-name=Mirror2','--target=es2022',`--outfile=${outfile}`]);
  const bundle=await readFile(outfile);
  viewerServer=createServer((req,res)=>{res.setHeader('Content-Type',req.url==='/renderer.js'?'text/javascript':'text/html');res.end(req.url==='/renderer.js'?bundle:'<!doctype html><div id="viewer"></div><script src="/renderer.js"></script>');});
  await new Promise(r=>viewerServer.listen(0,'127.0.0.1',r));viewerUrl=`http://127.0.0.1:${viewerServer.address().port}`;
  browser=await chromium.launch({executablePath:process.env.MIRROR_LAB_CHROME??'/opt/google/chrome/chrome',headless:true,chromiumSandbox:true});
});
after(async()=>{await browser?.close();if(viewerServer)await new Promise(r=>viewerServer.close(r));if(scratch)await rm(scratch,{recursive:true,force:true});});
async function roundtrip(html,child,run){
  const root=await mkdtemp(path.join(scratch,'source-'));
  const childSource=html.includes('CHILDPORT')?createServer((req,res)=>{res.setHeader('Content-Type','text/html');res.end(child);}):null;
  if(childSource)await new Promise(r=>childSource.listen(0,'127.0.0.1',r));
  const source=createServer((req,res)=>{res.setHeader('Content-Type','text/html');res.end((req.url.startsWith('/child')?child:html).replaceAll('CHILDPORT',String(childSource?.address().port)).replaceAll('PORT',String(source.address().port)));});
  await new Promise(r=>source.listen(0,'127.0.0.1',r));
  const host=new KernelBrowserHost(root);let context,page,sequence=0;
  try{
    const opened=await host.request({op:'open',url:`http://127.0.0.1:${source.address().port}/source`});
    const evaluate=async expression=>{const tab=host.tabs.get(opened.tab_id);const {connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);const reply=await connection.send('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true},sessionId);assert(!reply.exceptionDetails,'MP-10: source fixture evaluation');return reply.result.value;};
    for(let i=0;i<200&&!(await evaluate("location.pathname==='/source'&&document.readyState==='complete'").catch(()=>false));i++)await new Promise(r=>setTimeout(r,25));
    await host.request({op:'state'});
    const {subscription_id}=await host.request({op:'mirror_subscribe',tab_id:opened.tab_id,generation:opened.generation,device_scale_factor:1,wire:2});
    let compression=Buffer.alloc(0),base;
    // The product client restores the compact delta's binding and unchanged header.
    const next=async(reset=false)=>{
      let packet=await host.request({op:'mirror_next',subscription_id,generation:opened.generation,after_sequence:reset?0:sequence,drift_nodes:[],wait_ms:1000});
      sequence=packet.sequence;
      if(packet.reset)compression=Buffer.alloc(0);
      if(packet.encoding==='deflate'){
        compression=Buffer.concat([compression,Buffer.from(packet.packet_base64,'base64')]);
        const bytes=inflateRawSync(compression,{finishFlush:zlib.Z_SYNC_FLUSH});
        packet={...JSON.parse(bytes.subarray(bytes.length-packet.packet_bytes)),resources:packet.resources??[],tiles:packet.tiles??[]};
      }
      if(!packet.reset&&!packet.subscription_id){
        const header=name=>Object.hasOwn(packet,name)?packet[name]:base[name]??null;
        packet={...packet,wire:2,resources:packet.resources??[],tiles:packet.tiles??[],subscription_id,
          tab_id:base.tab_id,generation:base.generation,document_id:base.document_id,base_sequence:packet.sequence-1,reset:false,
          css_width:base.css_width,css_height:base.css_height,device_scale_factor:base.device_scale_factor,
          scroll:header('scroll'),focused:header('focused'),selection:header('selection'),fragment:header('fragment')};
      }
      base=packet;return packet;
    };
    context=await browser.newContext({viewport:{width:1400,height:900}});await context.grantPermissions(['clipboard-read','clipboard-write'],{origin:viewerUrl});page=await context.newPage();await page.goto(viewerUrl);
    const actions=[],errors=[];
    await page.exposeFunction('sourceAction',async action=>{actions.push(action);try{return await host.request({op:'input',tab_id:opened.tab_id,generation:opened.generation,document_id:host.tabs.get(opened.tab_id).document_id,input:{kind:'mirror',subscription_id,sequence,action}});}catch(error){errors.push(error.message);throw error;}});
    await page.evaluate(async()=>{window.r=new Mirror2.BrowserMirror2Renderer(document.getElementById('viewer'),action=>window.sourceAction(action));await window.r.ready();});
    let applied=0;const apply=async packet=>{if(packet.sequence<=applied)return;await page.evaluate(packet=>window.r.apply(Mirror2.decodeMirror2Packet(packet)),packet);applied=packet.sequence;};
    await run({page,host,next,apply,evaluate,actions,errors});
  }finally{await context?.close();await host.stop();await new Promise(r=>source.close(r));if(childSource)await new Promise(r=>childSource.close(r));await rm(root,{recursive:true,force:true});}
}
test('MP-08/MP-10: native clipboard paste replaces a selected contenteditable range in the kernel',()=>roundtrip(
  '<!doctype html><div id="editor" contenteditable="true">hello world</div>','',async({page,next,apply,evaluate,actions,errors})=>{
    await apply(await next());
    await page.evaluate(async()=>{await navigator.clipboard.writeText('Chariox');const d=window.r.frame.contentDocument,e=d.getElementById('editor');window.pasteFields=[];e.addEventListener('beforeinput',v=>{if(v.inputType==='insertFromPaste')window.pasteFields.push({data_null:v.data===null,plain_text:v.dataTransfer?.types.includes('text/plain')===true});});e.focus();const range=d.createRange();range.setStart(e.firstChild,6);range.setEnd(e.firstChild,11);d.getSelection().removeAllRanges();d.getSelection().addRange(range);});
    await page.waitForTimeout(100);await page.keyboard.press('Control+v');
    for(let i=0;i<60&&await evaluate("editor.textContent!=='hello Chariox'");i++)await page.waitForTimeout(25);
    assert.equal(await evaluate('editor.textContent'),'hello Chariox','MP-08: the normal guarded text action replaces the kernel range');
    assert.deepEqual(await page.evaluate(()=>window.pasteFields),[{data_null:true,plain_text:true}]);
    assert(actions.some(a=>a.kind==='selection'));assert(actions.some(a=>a.kind==='text'&&a.text==='Chariox'));assert.deepEqual(errors,[]);
  }));
// Review #941@531ba0343 P2-3: `:target` was rewritten in every admitted sheet but only the top document's fragment
// travelled, so a section a frame document targets stayed hidden (initially and after its hash changes).
for(const [label,hostname] of [['same-origin','127.0.0.1'],['foreign','localhost']])for(const resnapshot of [false,true])test(`MP-10: protocol 503 a ${label} frame document's fragment targets its own section (initial, then a hash change by ${resnapshot?'snapshot':'delta'})`,()=>roundtrip(
  `<!doctype html><p id="section">top section</p><iframe src="http://${hostname}:PORT/child#section" style="width:320px;height:140px"></iframe>`,
  '<!doctype html><style>p{display:none}p:target{display:block}</style><p id="section">first target</p><p id="other">second target</p><script>onmessage=e=>{location.hash=e.data}</script>',
  async({page,host,next,apply,evaluate})=>{
    await apply(await next());
    const source=async()=>{const entry=[...host.mirror.streams.values()][0].frames.values().next().value;const expression="[getComputedStyle(document.getElementById('section')).display,getComputedStyle(document.getElementById('other')).display]";if(entry){const r=await entry.child.connection.send('Runtime.evaluate',{expression,contextId:entry.child.contextId,returnByValue:true},entry.child.sessionId);assert(!r.exceptionDetails);return r.result.value;}return evaluate(`frames[0].eval(${JSON.stringify(expression)})`);};
    const shown=()=>page.evaluate(()=>{const d=window.r.frame.contentDocument.querySelector('iframe').contentDocument;return ['section','other'].map(id=>getComputedStyle(d.getElementById(id)).display);});
    const native=[await source()],mirror=[await shown()];
    await evaluate("frames[0].postMessage('other','*');true");
    for(let i=0;i<20&&JSON.stringify(await shown())!==JSON.stringify(['none','block']);i++)await apply(await next(resnapshot&&i===0));
    native.push(await source());mirror.push(await shown());
    assert.deepEqual(native,[['block','none'],['none','block']],'MP-10: the real source activates each child target');
    assert.deepEqual(mirror,native,'MP-10: the frame document marks its own target, independent of the top document');
    assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('section').hasAttribute('chariox-target')),false,'MP-10: the top document has no fragment');
  }));
// Review #320@4f8b54237 P3: a paste over the kernel's 16384-unit text limit was refused whole and changed nothing.
test('MP-08: a native clipboard paste longer than one text action reaches the kernel whole',()=>roundtrip(
  '<!doctype html><textarea id="field"></textarea>','',async({page,next,apply,evaluate,errors})=>{
    await apply(await next());
    const text='x'.repeat(16383)+'\u{1F600}'+'y'.repeat(5000);
    await page.evaluate(async text=>{await navigator.clipboard.writeText(text);window.r.frame.contentDocument.getElementById('field').focus();},text);
    for(let i=0;i<40&&await evaluate('document.activeElement?.id')!=='field';i++){await apply(await next());await page.waitForTimeout(25);}
    await page.keyboard.press('Control+v');
    for(let i=0;i<120&&await evaluate('field.value.length')!==text.length;i++)await page.waitForTimeout(25);
    assert.equal(await evaluate('field.value'),text,'MP-08: every slice of the paste reaches the kernel field in order');assert.deepEqual(errors,[]);
  }));
for(const origin of ['localhost:PORT','127.0.0.1:CHILDPORT'])test(`MP-08/MP-10: ${origin} foreign child closed custom root presents its real tile pixels`,()=>roundtrip(
  `<!doctype html><iframe src="http://${origin}/child" style="width:320px;height:140px"></iframe>`,
  '<!doctype html><x-closed id="box" style="display:block;width:100px;height:60px"></x-closed><script>customElements.define("x-closed",class extends HTMLElement{constructor(){super();this.attachShadow({mode:"closed"}).innerHTML=\'<div style="width:100px;height:60px;background:rgb(3,41,207)">closed child</div>\'}});onmessage=e=>{if(e.data==="large"){box.style.width="900px";box.style.height="600px"}}</script>',
  async({page,host,next,apply,evaluate})=>{
    const packet=await next();assert.equal(packet.fallback,undefined);
    const entry=[...host.mirror.streams.values()][0].frames.values().next().value;
    assert(entry?.child,'MP-10: fixture foreign child is attached');
    const snap=await entry.child.connection.send('DOMSnapshot.captureSnapshot',{computedStyles:[]},entry.child.sessionId);
    if(origin.includes('CHILDPORT'))assert(snap.documents.length>1,'MP-10: same-site foreign child shares the top-level CDP snapshot');
    assert(snap.documents.some(d=>d.nodes.shadowRootType?.value.some(v=>snap.strings[v]==='closed')),'MP-10: trusted fixture precondition proves a real closed shadow root exists');
    await apply(packet);
    const color=await page.evaluate(async()=>{const d=window.r.frame.contentDocument.querySelector('iframe').contentDocument,box=d.getElementById('box'),image=getComputedStyle(box).backgroundImage;const match=image.match(/^url\("?(.*?)"?\)$/);if(!match)return null;const img=new Image();await new Promise((r,j)=>{img.onload=r;img.onerror=j;img.src=match[1];});const canvas=document.createElement('canvas');canvas.width=100;canvas.height=60;const ctx=canvas.getContext('2d');ctx.drawImage(img,0,0,100,60);return [...ctx.getImageData(75,30,1,1).data];});
    assert.deepEqual(color,[3,41,207,255],'MP-10: the real renderer displays the child closed region pixels');
    await evaluate("document.querySelector('iframe').src+='?replacement';true");await page.waitForTimeout(700);
    const replacement=await next();assert.equal(replacement.fallback,undefined);await apply(replacement);
    const replaced=()=>page.evaluate(()=>window.r.frame.contentDocument.querySelector('iframe').contentDocument.getElementById('box').style.backgroundImage.startsWith('url('));
    for(let i=0;i<6&&!await replaced();i++)await apply(await next());
    assert(await replaced(),'MP-10: replacement child snapshot also carries its closed region');
    await evaluate("(()=>{const f=document.querySelector('iframe');f.style.width='1100px';f.style.height='700px';f.contentWindow.postMessage('large','*');return true})()");
    let large=await next();for(let i=0;i<4&&large.fallback!=='region_area';i++)large=await next();
    assert.equal(large.fallback,'region_area','MP-11: the same closed region obeys the opaque-area video handoff');
  }));

// MP-08/MP-10/MP-11: review a44f1afce F1, layout and pixels belong to the same opaque box.
for(const tag of ['canvas','x-opaque'])test(`MP-10: opaque ${tag} preserves author position, display and transform across class/style deltas`,()=>roundtrip(
 `<!doctype html><style>.moved{left:420px!important;top:260px!important;display:block!important;transform:translate(7px,11px)!important}</style><${tag} id="box" width="80" height="40" style="position:absolute;left:200px;top:100px;display:block;width:80px;height:40px;${tag==='canvas'?'transform:translate(13px,7px)':''}"></${tag}><script>${tag==='canvas'?"const c=box.getContext('2d');c.fillStyle='rgb(19,31,47)';c.fillRect(0,0,80,40)":"customElements.define('x-opaque',class extends HTMLElement{constructor(){super();this.attachShadow({mode:'closed'}).innerHTML='<div style=width:80px;height:40px;background:rgb(19,31,47)></div>'}})"}</script>`,
 '',async({page,next,apply,evaluate})=>{
  const properties="(()=>{const e=document.getElementById('box'),r=e.getBoundingClientRect(),s=getComputedStyle(e);return {x:r.x,y:r.y,width:r.width,height:r.height,display:s.display,transform:s.transform}})()";
  const read=()=>page.evaluate(()=>{const e=window.r.frame.contentDocument.getElementById('box'),r=e.getBoundingClientRect(),s=e.ownerDocument.defaultView.getComputedStyle(e);return {x:r.x,y:r.y,width:r.width,height:r.height,display:s.display,transform:s.transform,paint:s.backgroundImage}});
  const agree=async()=>{const expected=await evaluate(properties),actual=await read();for(const name of ['x','y','width','height'])assert(Math.abs(actual[name]-expected[name])<=1,`MP-10: ${tag} ${name}: ${actual[name]} vs ${expected[name]}`);assert.equal(actual.display,expected.display);assert.equal(actual.transform,expected.transform);assert(actual.paint.startsWith('url('),'MP-10: tile paint survives layout reconciliation');};
  await apply(await next());await agree();
  for(const expression of ["box.className='moved';true","box.className='';box.setAttribute('style','position:absolute;left:600px;top:300px;display:block;width:80px;height:40px');true"]){
   await evaluate(expression);for(let i=0;i<3;i++){const packet=await next();assert(!packet.reset);await apply(packet);await page.waitForTimeout(30);}await agree();
  }
 }));

test('MP-08/MP-10: ordinary typing invokes the native page keyboard contract',()=>roundtrip(
  '<input id="field"><script>window.keys=[];document.addEventListener("keydown",e=>window.keys.push(e.key))</script>','',async({page,next,apply,evaluate})=>{
    await apply(await next()); const field=page.frameLocator('iframe[title="Mirrored kernel browser page"]').locator('#field');
    await field.click(); await page.waitForTimeout(150);await page.keyboard.type('Chariox',{delay:60});
    for(let i=0;i<30&&await evaluate('document.querySelector("#field").value')!=='Chariox';i++){await apply(await next());await page.waitForTimeout(25);}
    assert.deepEqual(await evaluate('window.keys'),[...'Chariox'],'ordinary keys must reach native keydown listeners');
    assert.equal(await evaluate('document.querySelector("#field").value'),'Chariox');
  }));

// MP-08/MP-10/MP-11 M2: real source + unmodified product renderer consume
// the ordinary wire incrementally. Anonymous fixture; not live acceptance.
test('MP-10 M2: the renderer paints the prefix and retains it while ordinary children packets finish the document',()=>roundtrip(
  '<!doctype html><h1 id="first">Viewport first</h1><main>'+'<p>Middle</p>'.repeat(10001)+'<p id="last">Last paragraph</p></main>','',async({page,next,apply})=>{
    const first=await next();assert(!first.fallback);assert(first.nodes.length<4096);await apply(first);
    assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('first')?.textContent),'Viewport first');
    assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('last')),null);
    let complete=false;
    for(let n=0;n<200&&!complete;n++){const packet=await next();assert(!packet.reset);assert(!packet.fallback);await apply(packet);complete=await page.evaluate(()=>window.r.frame.contentDocument.getElementById('last')?.textContent==='Last paragraph');}
    assert(complete);assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.querySelectorAll('p').length),10002);
    assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('first')?.textContent),'Viewport first');
  }));

// MP-08/MP-10/MP-11 M2: WHATWG's explanatory title attributes contain a
// javascript: literal away from the start; the strict client rejects them.
test('MP-10/MP-11 M2: descriptive scheme literals cannot poison a continuation',()=>roundtrip(
  '<!doctype html><h1>First viewport</h1><main>'+'<p>Middle</p>'.repeat(10001)+'<dfn id="scheme" title="An example javascript: scheme">javascript: stays visible text</dfn></main>','',async({page,next,apply})=>{
    await apply(await next());let done=false;
    for(let n=0;n<200&&!done;n++){await apply(await next());done=await page.evaluate(()=>window.r.frame.contentDocument.getElementById('scheme')?.textContent==='javascript: stays visible text');}
    assert(done);assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('scheme').getAttribute('title')),null);
  }));

import {probePaths} from './probe-config.mjs';
import {CpuSampler,cpuSpan} from '../drill-cpu.mjs';
// MP-08/MP-10/MP-11: real hosted web, public controls, numeric per-hop diagnostics.
const {chromium}=await import(process.env.VIDHOPS_PLAYWRIGHT??'/w/valenv/tools/node_modules/playwright-core/index.mjs');
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {sampleVisibleFrame} from './probe-surface.mjs';
import {PhysicalPixels} from './native-observer.mjs';
import {OwnedDisplay} from './viewer-display.mjs';
const {LocalIpcClient}=await import(process.env.VIDHOPS_IPC);
import {startViewerNetwork} from './viewer-network.mjs';
const [,,root,E]=process.argv,paths=probePaths(root,E),state=JSON.parse(await readFile(root+'/state.json','utf8'));
await mkdir(E,{recursive:true});await mkdir(paths.viewer,{recursive:true});await mkdir(paths.tmp,{recursive:true});
process.env.CHARIOX_HOME=paths.runtime;
const ipc=new LocalIpcClient(`ws://127.0.0.1:${state.ports[0]}/kernel`,{controlResponseStallMs:60000});
const display=new OwnedDisplay(root+'/viewer'),report={items:['MP-08','MP-10','MP-11'],oss:process.env.OSS_SOURCE??null,cloud:process.env.CLOUD_SOURCE??null,status:'RUNNING',recording:process.env.VIDHOPS_RECORD==='1',cells:[],viewer_gpu_mode:process.env.VIDLAT_VIEWER_DEFAULT_GPU==='1'?'chromium_default':'forced_software',qualification:'Engineering diagnostics; host recorded in command receipt; composed video/repair surface echo plus next RAF, not campaign X11 endpoint acceptance. Input driven only through real hosted web.'};
let browser,network,pixels;const cpu=new CpuSampler({root:paths.V,viewerRoot:paths.viewer});if(paths.ownerAB)await cpu.track(state.kernelPid,'pipeline');cpu.start();process.once('SIGTERM',async()=>{report.status='INTERRUPTED_SIGTERM';await cpu.close();await save();await browser?.close();await network?.close();await pixels?.close();await display.close();await ipc.close();process.exit(143)});
const save=()=>writeFile(E+'/results.json',JSON.stringify(report,null,2));
const request=async command=>(await ipc.send({KernelBrowser:{command}})).KernelBrowser.result;
const initialState=await request({op:'state'}).catch(e=>{if(!String(e.message).includes('browser_unavailable'))throw e;return {tabs:[]}});const initialTabIds=new Set((initialState.tabs??[]).map(t=>t.tab_id));
async function controls(){
 const current=await request({op:'state'});const tab=current.tabs?.find(t=>!initialTabIds.has(t.tab_id));if(!tab)throw Error('own viewer tab absent');
 if(!/^https:\/\/(en\.wikipedia\.org|www\.wikipedia\.org)\//.test(tab.url))throw Error('public source changed unexpectedly');
 const snapshot=(await request({op:'snapshot',tab_id:tab.tab_id,generation:current.generation})).snapshot;
 const ids=new Set(['searchInput','skin-client-pref-skin-theme-value-day','skin-client-pref-skin-theme-value-night','js-lang-list-button']);
 const html=(snapshot.dom_nodes??[]).find(n=>n.node_name==='HTML'&&n.bounds?.width>0),scale=Math.max(1,Math.round((html?.bounds.width??1280)/1280));
 const selected=(snapshot.dom_nodes??[]).filter(n=>ids.has(n.attributes?.id)&&n.bounds?.width>0&&n.bounds.height>0);
 const close=(snapshot.dom_nodes??[]).find(n=>n.node_name==='BUTTON'&&/(^|\s)(frb-close|cn-closeButton)(\s|$)/.test(n.attributes?.class??'')&&n.bounds?.width>0);
 return {url:tab.url,scale,bannerClose:close?Object.fromEntries(Object.entries(close.bounds).map(([k,v])=>[k,v/scale])):null,controls:selected.map(n=>({id:n.attributes.id,bounds:Object.fromEntries(Object.entries(n.bounds).map(([k,v])=>[k,v/scale])),expanded:n.attributes?.['aria-expanded']??null,value:(snapshot.accessibility_nodes??[]).find(a=>a.node_ref===n.node_ref)?.value??null}))};
}
function observe(){
 const at=()=>performance.timeOrigin+performance.now();
 globalThis.probe={inputs:[],arrivals:[],draws:[],frames:[],receives:[],stages:[],presented:[],longtasks:[],raf:[]};
 for(const kind of ['pointerdown','keydown','wheel','beforeinput'])window.addEventListener(kind,e=>{if(e.isTrusted){const time=at();probe.inputs.push({at:time,kind,dx:e.deltaX,dy:e.deltaY});globalThis.__vidlatPhysicalInput?.({at:time,kind});}},true);
 const NativeWorker=Worker;let workerId=0;globalThis.Worker=new Proxy(NativeWorker,{construct(T,args){const w=Reflect.construct(T,args),id=++workerId,decoder=String(args[0]).endsWith('/decoder-worker.mjs'),jobs=new Map();if(decoder){const send=w.postMessage.bind(w);w.postMessage=(m,t)=>{if(Number.isSafeInteger(m?.id)&&Number.isSafeInteger(m?.sequence)){jobs.set(m.id,m.sequence);probe.stages.push({stage:'worker_decode_send',worker:id,id:m.id,sequence:m.sequence,channel:m.channel??0,at:at()})}return send(m,t)};w.addEventListener('message',e=>{if(Number.isSafeInteger(e.data?.id)){probe.stages.push({stage:'worker_decode_reply',worker:id,id:e.data.id,sequence:jobs.get(e.data.id),channel:e.data.channel??null,at:at()});jobs.delete(e.data.id)}})}return w}});
 const NativeWS=WebSocket;globalThis.WebSocket=new Proxy(NativeWS,{construct(T,args){const ws=Reflect.construct(T,args);ws.addEventListener('message',e=>{if(e.data instanceof ArrayBuffer||e.data instanceof Blob)probe.arrivals.push({at:at(),bytes:e.data.byteLength??e.data.size});});return ws;}});
 const draw=CanvasRenderingContext2D.prototype.drawImage;CanvasRenderingContext2D.prototype.drawImage=function(...args){const t=at(),result=Reflect.apply(draw,this,args);if(this.canvas.getAttribute?.('aria-label')==='Kernel browser video')probe.draws.push({start:t,end:at()});return result;};
 const attached=new WeakSet();setInterval(()=>{for(const c of document.querySelectorAll('canvas[aria-label="Kernel browser video"]'))if(!attached.has(c)){attached.add(c);new MutationObserver(()=>{const seq=Number(c.dataset.displaySequence);if(seq!==probe.frames.at(-1)?.sequence){const f={sequence:seq,kind:c.dataset.displayKind,codec:c.dataset.streamCodec,drawn:at(),width:c.width,height:c.height};probe.frames.push(f);requestAnimationFrame(()=>f.raf=at());}}).observe(c,{attributes:true,attributeFilter:['data-display-sequence']});}},25);
 try{new PerformanceObserver(list=>{for(const e of list.getEntries())probe.longtasks.push({at:performance.timeOrigin+e.startTime,duration:e.duration});}).observe({entryTypes:['longtask']});}catch{}
 const tick=t=>{probe.raf.push(performance.timeOrigin+t);if(probe.raf.length>30000)probe.raf.splice(0,5000);requestAnimationFrame(tick)};requestAnimationFrame(tick);
}
try{
 const denv=await display.start();pixels=await new PhysicalPixels(denv).start();network=await startViewerNetwork(state.relayUrl,{httpsOrigins:[state.webOrigin],uplinkMbps:8,oneWayDelayMs:30});
 browser=await chromium.launch({headless:false,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox',...(process.env.VIDLAT_VIEWER_DEFAULT_GPU==='1'?[]:['--disable-gpu']),'--disable-dev-shm-usage','--js-flags=--max-old-space-size=1024','--window-size=1920,1080',`--disk-cache-dir=${root}/viewer/cache`,`--force-device-scale-factor=${Number((process.env.DPRS??'1').split(',')[0])}`,`--proxy-server=${network.proxyUrl}`],env:{...process.env,...denv,TMPDIR:paths.tmp}});
 for(const dpr of (process.env.DPRS??'1,2').split(',').map(Number)){
  const ctx=await browser.newContext({viewport:{width:1920,height:1080},deviceScaleFactor:dpr,...(process.env.VIDHOPS_RECORD==='1'?{recordVideo:{dir:E+'/recordings',size:{width:1920,height:1080}}}:{})});await ctx.addInitScript(observe);await ctx.addInitScript(`globalThis.__vidlatSampleVisibleFrame=${sampleVisibleFrame.toString()}`);
  // MP-08/MP-10 diagnostics: wrap existing exported attach callback only; the real built
  // app, presenter and transport execute unchanged. This instrumentation is not acceptance.
  if(process.env.VIDHOPS_INSTRUMENT==='1')await ctx.route('**/vendor/browser-display-*/presenter.mjs',async route=>{const response=await route.fetch();const source=await response.text();const suffix=`
const originalAttach=attachBrowserDisplay; attachBrowserDisplay=function(canvas,transport,tab,options={}){const receive=transport.onEvent,oldTiming=options.onTiming,oldPresented=options.onPresented;return originalAttach(canvas,{...transport,onEvent:fn=>receive(event=>{if(event.event==='kernel_browser_frame'&&event.frame){const f=event.frame;globalThis.probe.receives.push({at:performance.timeOrigin+performance.now(),sequence:f.sequence,kind:f.kind});}fn(event);})},tab,{...options,onTiming:(stage,start)=>{globalThis.probe.stages.push({stage,start,end:performance.timeOrigin+performance.now()});oldTiming?.(stage,start);},onPresented:f=>{globalThis.probe.presented.push({at:performance.timeOrigin+performance.now(),sequence:f.sequence,kind:f.kind});oldPresented?.(f);}});};
`;await route.fulfill({response,body:source+suffix});});
  const p=await ctx.newPage();p.setDefaultTimeout(15000);await p.exposeBinding('__vidlatPhysicalInput',(_,v)=>{if(v.kind==='pointerdown'||v.kind==='keydown'||(pixels.active?.mode==='frames'&&v.kind==='wheel'))pixels.begin(v.at)});network.attach(p);
  const cell={dpr,status:'RUNNING',clicks:[],scroll:[],shortcuts:[]};report.cells.push(cell);cell.resources=[];for(const [event,stage]of [['request','decoder_module_request'],['requestfinished','decoder_module_finished']])ctx.on(event,req=>{if(new URL(req.url()).pathname.endsWith('/decoder-worker.mjs'))cell.resources.push({stage,at:Date.now()})});
  try{
   await p.goto(state.webOrigin+'/waiting-room');await writeFile(E+'/progress.json',JSON.stringify({items:report.items,at:Date.now(),stage:'waiting-room-loaded'}));await p.getByText('Waiting Room Ready',{exact:true}).waitFor({timeout:60000});await p.waitForTimeout(6000);await writeFile(E+'/progress.json',JSON.stringify({items:report.items,at:Date.now(),stage:'waiting-room-ready'}));
   for(const [field,id]of [['Machine',state.machineId],['Kernel',state.daemonId]]){if(!await p.getByRole('button',{name:new RegExp('^'+field)}).count()&&field==='Kernel')continue;await p.getByRole('button',{name:new RegExp('^'+field)}).click();const option=p.locator(`[data-option-picker-value="${id}"]`);await option.waitFor({state:'visible',timeout:3000}).catch(()=>{});if(await option.count())await option.click();else if(field!=='Kernel'||!(await p.getByRole('button',{name:/^Kernel/}).innerText()).includes(state.alias))throw Error('MP-10 required kernel choice absent');await p.waitForTimeout(field==='Machine'?5000:1000);}
   await p.locator(`[data-waiting-session-id="${state.sessionId}"]`).click();await p.waitForTimeout(3000);
   await p.getByRole('button',{name:'Open Browser panel',exact:true}).click();await writeFile(E+'/progress.json',JSON.stringify({items:report.items,at:Date.now(),stage:'browser-panel-opened'}));
   await p.waitForTimeout(5000);
   const newTab=p.getByRole('button',{name:'New tab',exact:true});if(await newTab.count()){await p.getByRole('textbox',{name:'Browser address'}).fill('https://www.wikipedia.org/');await newTab.click();await p.waitForTimeout(5000);}
   const surface=()=>p.locator('canvas[aria-label="Kernel browser video"]');
   async function navigate(url){await p.getByRole('textbox',{name:'Browser address'}).fill(url);await p.getByRole('button',{name:/^(Open tab|Go)$/}).click();await surface().waitFor({timeout:60000});await p.waitForTimeout(10000);if(new URL((await controls()).url).href!==new URL(url).href)throw Error('MP-10 viewer navigation missed requested page');}
   async function point(bounds){const box=await surface().boundingBox(),k=Math.min(box.width/1280,box.height/800);return {x:box.x+(box.width-1280*k)/2+(bounds.x+bounds.width/2)*k,y:box.y+(box.height-800*k)/2+(bounds.y+bounds.height/2)*k};}
   await navigate('https://en.wikipedia.org/wiki/Web_browser');let first=await controls();if(first.bannerClose){const q=await point(first.bannerClose);await p.screenshot({path:E+`/dpr${dpr}-banner-before.png`});await p.mouse.click(q.x,q.y);await p.waitForTimeout(2000);}const dbArticle=await p.getByRole('dialog',{name:'Kernel browser',exact:true}).locator('.ds-modal-body').boundingBox();await p.mouse.move(dbArticle.x+22,dbArticle.y+110);await p.mouse.wheel(0,600);await p.waitForTimeout(500);cell.article=await controls();await p.screenshot({path:E+`/dpr${dpr}-article.png`});
   const day=cell.article.controls.find(c=>c.id.endsWith('-day')),night=cell.article.controls.find(c=>c.id.endsWith('-night'));
   if(day&&night&&Math.max(day.bounds.y,night.bounds.y)<780){
    const q=await point(day.bounds);await p.mouse.click(q.x,q.y);await p.waitForTimeout(2000);
    for(let n=0;n<Number(process.env.TRIALS??20);n++){
     const dark=n%2===0,q=await point((dark?night:day).bounds),b=await surface().boundingBox();const nativeGeometry=await pixels.arm(p,{x:b.x+2*b.width/1280,y:b.y+100*b.height/800,width:30,height:30},'latency',{luminance:dark?[0,100]:[180,255]});if(nativeGeometry.target_already_visible)throw Error('MP-10 target theme already visible before input');await p.mouse.click(q.x,q.y);
     const sample=await p.evaluate(async dark=>{const c=document.querySelector('canvas[aria-label="Kernel browser video"]'),input=probe.inputs.findLast(x=>x.kind==='pointerdown'),deadline=performance.now()+5000;while(performance.now()<deadline){const observed=globalThis.__vidlatSampleVisibleFrame(c,probe.frames,input.at,dark);if(observed){const {frame:match,mean}=observed;await new Promise(requestAnimationFrame);return {input:input.at,visible:performance.timeOrigin+performance.now(),latency:performance.timeOrigin+performance.now()-input.at,frame:match,mean};}await new Promise(requestAnimationFrame);}return {input:input.at,latency:null};},dark);
     const physical=await pixels.result();sample.canvas_raf_latency=sample.latency;sample.physical=physical;sample.native_geometry=nativeGeometry;sample.latency=physical.latency_ms;sample.visible=physical.presented_at;cell.clicks.push(sample);await save();await p.waitForTimeout(500);
    }
   }else cell.click_blocker='public theme controls absent/outside source viewport';
   await navigate('https://en.wikipedia.org/wiki/Web_browser');first=await controls();if(first.bannerClose){const q=await point(first.bannerClose);await p.mouse.click(q.x,q.y);await p.waitForTimeout(2000);}const box=await surface().boundingBox();await p.mouse.move(box.x+box.width*.7,box.y+box.height*.5);
   for(const [delta,hz]of [[120,20],[120,60],[16,60]]){
    const b=await surface().boundingBox();await pixels.arm(p,{x:b.x+b.width*.55,y:b.y+b.height*.5,width:128,height:128},'frames');const cpuBefore=await cpu.sample(),begin=Date.now(),pending=[];for(let n=0;n<hz*5;n++){const sent=p.mouse.wheel(0,(Math.floor(n/(hz/2))%2?-1:1)*delta);if(process.env.FAST_SCROLL==='1'&&hz===60)pending.push(sent);else await sent;await p.waitForTimeout(Math.max(0,begin+(n+1)*1000/hz-Date.now()));}await Promise.all(pending);await p.waitForTimeout(500);
    const row=await p.evaluate(({begin,delta,hz})=>({begin,end:begin+5000,delta,hz,input_count:probe.inputs.filter(x=>x.kind==='wheel'&&x.at>=begin&&x.at<begin+5000).length,frames:probe.frames.filter(f=>f.drawn>=begin&&f.drawn<begin+5000)}),{begin,delta,hz});row.cpu=cpuSpan(cpuBefore,await cpu.sample());row.dataset_fps=row.frames.length/5;row.native=await pixels.end(begin,begin+5000);row.fps=row.native.fps;cell.scroll.push(row);await save();
   }
   await navigate('https://www.wikipedia.org/');cell.portal=await controls();const input=cell.portal.controls.find(c=>c.id==='searchInput');
   if(input){const q=await point(input.bounds);await p.mouse.click(q.x,q.y);await p.keyboard.insertText('chariox');await p.waitForTimeout(1500);const typed=await controls();await p.screenshot({path:E+`/dpr${dpr}-shortcut-seed.png`});await p.keyboard.press('Control+A');await p.keyboard.press('Backspace');await p.waitForTimeout(1500);const cleared=await controls();await p.screenshot({path:E+`/dpr${dpr}-shortcut-cleared.png`});cell.shortcuts.push({typed:typed.controls.find(c=>c.id==='searchInput')?.value,after_ctrl_a_backspace:cleared.controls.find(c=>c.id==='searchInput')?.value,expected:''});}
   cell.typing=[];
   if(input){
    const b=await surface().boundingBox(),k=Math.min(b.width/1280,b.height/800),field=input.bounds;
    const roi={x:b.x+(b.width-1280*k)/2+(field.x+8)*k,y:b.y+(b.height-800*k)/2+(field.y+4)*k,width:Math.min(128,field.width*k-16),height:Math.min(40,field.height*k-8)};
    for(let n=0;n<Number(process.env.TYPING_TRIALS??process.env.TRIALS??20);n++){
     if((await controls()).controls.find(c=>c.id==='searchInput')?.value!=='')throw Error('MP-10 typing baseline not empty');
     await pixels.arm(p,roi,'latency',{threshold:4,columns:3});await p.keyboard.press('m');const physical=await pixels.result();
     await p.waitForTimeout(200);const observed=(await controls()).controls.find(c=>c.id==='searchInput')?.value;cell.typing.push({physical,source_echo_matches:observed==='m',expected_glyph:'m'});if(observed!=='m')throw Error('MP-10 source typing echo differs');
     await p.keyboard.press('Backspace');await p.waitForTimeout(500);await save();
    }
   }
   // Restore the real public input through supported keys before qualifying menu clicks.
   const still=(await controls()).controls.find(c=>c.id==='searchInput')?.value??'';if(still.length>20)throw Error('unexpected public input length');for(let n=0;n<still.length;n++)await p.keyboard.press('Backspace');await p.waitForTimeout(1500);cell.portal_clear=(await controls()).controls.find(c=>c.id==='searchInput')?.value;if(cell.portal_clear!=='')throw Error('public input not cleared');
   // A user scrolls the enclosing modal, outside the remote canvas, to expose its full viewport.
   const dialog=p.getByRole('dialog',{name:'Kernel browser',exact:true}).locator('.ds-modal-body'),db=await dialog.boundingBox();await p.mouse.move(db.x+22,db.y+110);await p.mouse.wheel(0,500);await p.waitForTimeout(500);
   const button=cell.portal.controls.find(c=>c.id==='js-lang-list-button');if(button){const q=await point(button.bounds);cell.portal_button={source_bounds:button.bounds,point:q,surface:await surface().boundingBox()};cell.portal_toggles=[];for(let n=0;n<4;n++){await p.mouse.click(q.x,q.y);await p.waitForTimeout(1000);const cs=await controls();cell.portal_toggles.push({url:cs.url,expanded:cs.controls.find(c=>c.id==='js-lang-list-button')?.expanded});await p.screenshot({path:E+`/dpr${dpr}-portal-toggle${n}.png`});}}
   await p.screenshot({path:E+`/dpr${dpr}-portal.png`});cell.metrics=await p.evaluate(()=>probe);cell.status='MEASURED_NOT_ACCEPTED';
  }catch(e){cell.status='FAIL';cell.error=e.message.split('\n')[0].replace(/https?:\/\/\S+/g,'[URL]').slice(0,300);await p.screenshot({path:E+`/dpr${dpr}-failure.png`}).catch(()=>{});}finally{await save();const close=p.getByRole('button',{name:'Close tab',exact:true});if(await close.count())await close.click().catch(()=>{});await ctx.close();}
 }
 report.network=await network.snapshot();report.status=report.cells.every(c=>c.status==='MEASURED_NOT_ACCEPTED')?'MEASURED_NOT_ACCEPTED':'FAIL';
}catch(e){report.status='FAIL';report.error=e.message.split('\n')[0].slice(0,300);process.exitCode=1;}finally{report.cpu_samples=cpu.samples;await cpu.close();await browser?.close();await network?.close();await pixels?.close();await display.close();await ipc.close();await save();if(report.status==='FAIL')process.exitCode=1;console.log(JSON.stringify({items:report.items,status:report.status,cells:report.cells.map(c=>({dpr:c.dpr,status:c.status,clicks:c.clicks.map(x=>x.latency),scroll:c.scroll.map(x=>({delta:x.delta,hz:x.hz,fps:x.fps})),shortcuts:c.shortcuts,error:c.error})),error:report.error}));}

import {probePaths,visibleSourcePage} from './probe-config.mjs';
import {CpuSampler,cpuSpan} from '../drill-cpu.mjs';
// MP-08/MP-10/MP-11: real hosted web, public controls, numeric per-hop diagnostics.
const {chromium}=await import(process.env.VIDHOPS_PLAYWRIGHT??'/w/valenv/tools/node_modules/playwright-core/index.mjs');
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {PhysicalPixels} from './native-observer.mjs';
import {OwnedDisplay} from './viewer-display.mjs';
const {default:WebSocket}=await import(process.env.VIDHOPS_WS);
import {startViewerNetwork} from './viewer-network.mjs';
const [,,root,E]=process.argv,paths=probePaths(root,E),state=JSON.parse(await readFile(root+'/state.json','utf8'));
if(paths.ownerAB){state.webOrigin=state.selkiesOrigin;state.relayUrl=state.selkiesOrigin.replace('https:','wss:');}
await mkdir(E,{recursive:true});await mkdir(paths.viewer,{recursive:true});await mkdir(paths.tmp,{recursive:true});
process.env.CHARIOX_HOME=paths.runtime;
const ipc={close:async()=>{}};
const display=new OwnedDisplay(root+'/viewer'),report={items:['MP-08','MP-10','MP-11'],oss:process.env.OSS_SOURCE??null,cloud:process.env.CLOUD_SOURCE??null,status:'RUNNING',recording:process.env.VIDHOPS_RECORD==='1',cells:[],viewer_gpu_mode:process.env.VIDLAT_VIEWER_DEFAULT_GPU==='1'?'chromium_default':'forced_software',qualification:'Pinned Selkies actual viewer + Decision G native pixel observer; source CDP is preparation/read-only oracle. Engineering diagnostics; host recorded in command receipt; source canvas pixel echo plus next RAF, not campaign X11 endpoint acceptance. Input driven only through real hosted web.'};
let browser,network,pixels;const cpu=new CpuSampler({root:paths.S,viewerRoot:paths.viewer});if(paths.ownerAB)await cpu.track(state.selkiesPid,'pipeline');cpu.start();process.once('SIGTERM',async()=>{report.status='INTERRUPTED_SIGTERM';await cpu.close();await save();await browser?.close();await network?.close();await pixels?.close();await display.close();await ipc.close();process.exit(143)});
const save=()=>writeFile(E+'/results.json',JSON.stringify(report,null,2));
async function sourceRead(target,method,params={}){
 const w=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,j)=>{w.once('open',r);w.once('error',j)});
 try{return await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('MP-10 source oracle deadline')),5000);const done=(error,value)=>{clearTimeout(timer);error?reject(error):resolve(value)};w.once('error',()=>done(Error('MP-10 source oracle failed')));w.once('message',b=>{const v=JSON.parse(b);v.error?done(Error('MP-10 source oracle failed')):done(null,v.result)});w.send(JSON.stringify({id:1,method,params}))})}finally{w.close()}
}
async function cdpRead(method,params={}){
 const pages=await(await fetch(`http://127.0.0.1:${state.cdpPort}/json/list`)).json();
 const target=await visibleSourcePage(pages,async p=>(await sourceRead(p,'Runtime.evaluate',{expression:'document.visibilityState === "visible"',returnByValue:true})).result?.value===true);
 return sourceRead(target,method,params);
}
async function controls(){
 const result=await cdpRead('Runtime.evaluate',{expression:`(()=>{if(!['en.wikipedia.org','www.wikipedia.org'].includes(location.hostname))throw Error('public source required');const ids=['searchInput','skin-client-pref-skin-theme-value-day','skin-client-pref-skin-theme-value-night','js-lang-list-button'];const controls=ids.map(id=>document.getElementById(id)).filter(Boolean).map(e=>{const b=e.getBoundingClientRect();return {id:e.id,value:e.value,expanded:e.getAttribute('aria-expanded'),bounds:{x:b.x,y:b.y+${paths.ownerAB?87:0},width:b.width,height:b.height}}});const close=document.querySelector('.frb-icon-close');const b=close?.getBoundingClientRect();return {url:location.href,controls,bannerClose:b?{x:b.x,y:b.y+${paths.ownerAB?87:0},width:b.width,height:b.height}:null}})()`,returnByValue:true});return result.result.value;
}
function observe(){
 const at=()=>performance.timeOrigin+performance.now();
 globalThis.probe={inputs:[],arrivals:[],draws:[],frames:[],receives:[],stages:[],presented:[],longtasks:[],raf:[]};
 for(const kind of ['pointerdown','keydown','wheel','beforeinput'])window.addEventListener(kind,e=>{if(e.isTrusted){const time=at();probe.inputs.push({at:time,kind,dx:e.deltaX,dy:e.deltaY});globalThis.__vidlatPhysicalInput?.({at:time,kind});}},true);
 const note=v=>{if(v?.type==='vidlat'&&Number.isFinite(v.at)&&Number.isFinite(v.frame_id))probe.stages.push({stage:v.stage,at:v.at,frame_id:v.frame_id,stamp:v.stamp})};
 const workerPrefix=`
 (()=>{const times=new Map();globalThis.__vidlatId=-1;const at=()=>performance.timeOrigin+performance.now();const note=(stage,id,stamp)=>self.postMessage({type:'vidlat',stage,frame_id:Number.isFinite(id)?id:-1,stamp,at:at()});
 if(typeof VideoDecoder!=='undefined'){const Native=VideoDecoder;globalThis.VideoDecoder=class extends Native{constructor(opts){super({...opts,output:f=>{const meta=times.get(f.timestamp);note('decode_done',meta?.id,f.timestamp);opts.output(f);times.delete(f.timestamp)}})}decode(chunk){times.set(chunk.timestamp,{id:globalThis.__vidlatId});note('decode_start',globalThis.__vidlatId,chunk.timestamp);return super.decode(chunk)}}}
 const proto=typeof OffscreenCanvasRenderingContext2D!=='undefined'?OffscreenCanvasRenderingContext2D.prototype:typeof OffscreenCanvas!=='undefined'?Object.getPrototypeOf(new OffscreenCanvas(1,1).getContext('2d')):null;if(proto){const draw=proto.drawImage;proto.drawImage=function(...a){const result=Reflect.apply(draw,this,a);const meta=times.get(a[0]?.timestamp);note('draw',meta?.id??globalThis.__vidlatId,a[0]?.timestamp);return result}}
 if(typeof WebSocket!=='undefined'){const Native=WebSocket;globalThis.WebSocket=new Proxy(Native,{construct(T,args){const ws=Reflect.construct(T,args);ws.addEventListener('message',e=>{if(e.data instanceof ArrayBuffer&&e.data.byteLength>10){const b=new Uint8Array(e.data,0,10);if(b[0]===4||b[0]===3){globalThis.__vidlatId=(b[2]<<8)|b[3];note('receive',globalThis.__vidlatId)}}});return ws}})}
 })();`;
 const mainTimes=new Map();globalThis.__vidlatId=-1;
 if(globalThis.VideoDecoder){const Native=VideoDecoder;globalThis.VideoDecoder=class extends Native{constructor(opts){super({...opts,output:f=>{const meta=mainTimes.get(f.timestamp);note({type:'vidlat',stage:'decode_done',at:at(),frame_id:meta?.id??-1,stamp:f.timestamp});opts.output(f);mainTimes.delete(f.timestamp)}})}decode(chunk){mainTimes.set(chunk.timestamp,{id:globalThis.__vidlatId});note({type:'vidlat',stage:'decode_start',at:at(),frame_id:globalThis.__vidlatId,stamp:chunk.timestamp});return super.decode(chunk)}}}
 const videoObservers=new WeakSet();setInterval(()=>{for(const v of document.querySelectorAll('video'))if(v.requestVideoFrameCallback&&!videoObservers.has(v)){videoObservers.add(v);const tick=(now,meta)=>{note({type:'vidlat',stage:'video_present',frame_id:-1,stamp:meta.mediaTime*1e6,at:performance.timeOrigin+meta.expectedDisplayTime});v.requestVideoFrameCallback(tick)};v.requestVideoFrameCallback(tick)}},25);
 const NativeBlob=Blob;globalThis.Blob=new Proxy(NativeBlob,{construct(T,args){const parts=args[0];if(Array.isArray(parts)&&parts.length&&parts.every(x=>typeof x==='string')){let code=parts.join('');if(code.includes('VideoDecoder')||code.includes('WebSocket')||code.includes('drawImage')){code=code.replaceAll('const frameId = (head[2] << 8) | head[3];','const frameId = (head[2] << 8) | head[3]; globalThis.__vidlatId=frameId;');args=[[workerPrefix+code],args[1]];}}return Reflect.construct(T,args)}});
 const NativeWorker=Worker;globalThis.Worker=new Proxy(NativeWorker,{construct(T,args){const w=Reflect.construct(T,args);const send=w.postMessage.bind(w);w.postMessage=(m,t)=>{if(m?.type==='chunk'||m?.frame)note({type:'vidlat',stage:'worker_feed',at:at(),frame_id:mainTimes.get(m.frame?.timestamp)?.id??globalThis.__vidlatId,stamp:m.timestamp??m.frame?.timestamp});return send(m,t)};w.addEventListener('message',e=>{if(e.data?.type==='message'&&e.data.data instanceof ArrayBuffer&&e.data.data.byteLength>10){const b=new Uint8Array(e.data.data,0,10);if(b[0]===4||b[0]===3){globalThis.__vidlatId=(b[2]<<8)|b[3];note({type:'vidlat',stage:'client_dispatch',frame_id:globalThis.__vidlatId,at:at()})}}if(e.data?.type==='vidlat'){note(e.data);e.stopImmediatePropagation()}});return w}});
 const NativeWS=WebSocket;globalThis.WebSocket=new Proxy(NativeWS,{construct(T,args){const ws=Reflect.construct(T,args);ws.addEventListener('message',e=>{if(e.data instanceof ArrayBuffer||e.data instanceof Blob)probe.arrivals.push({at:at(),bytes:e.data.byteLength??e.data.size});});return ws;}});
 const draw=CanvasRenderingContext2D.prototype.drawImage;CanvasRenderingContext2D.prototype.drawImage=function(...args){const t=at(),result=Reflect.apply(draw,this,args);if(this.canvas.id==='videoCanvas'){probe.draws.push({start:t,end:at()});const meta=mainTimes.get(args[0]?.timestamp);note({type:'vidlat',stage:'draw',frame_id:meta?.id??globalThis.__vidlatId,stamp:args[0]?.timestamp,at:at()})}return result;};
 const attached=new WeakSet();setInterval(()=>{for(const c of document.querySelectorAll('#videoCanvas'))if(!attached.has(c)){attached.add(c);new MutationObserver(()=>{const seq=Number(c.dataset.displaySequence);if(seq!==probe.frames.at(-1)?.sequence){const f={sequence:seq,kind:c.dataset.displayKind,codec:c.dataset.streamCodec,drawn:at(),width:c.width,height:c.height};probe.frames.push(f);requestAnimationFrame(()=>f.raf=at());}}).observe(c,{attributes:true,attributeFilter:['data-display-sequence']});}},25);
 try{new PerformanceObserver(list=>{for(const e of list.getEntries())probe.longtasks.push({at:performance.timeOrigin+e.startTime,duration:e.duration});}).observe({entryTypes:['longtask']});}catch{}
 const tick=t=>{probe.raf.push(performance.timeOrigin+t);if(probe.raf.length>30000)probe.raf.splice(0,5000);requestAnimationFrame(tick)};requestAnimationFrame(tick);
}
try{
 const denv=await display.start();pixels=await new PhysicalPixels(denv).start();network=await startViewerNetwork(state.relayUrl,{httpsOrigins:[state.webOrigin],uplinkMbps:8,oneWayDelayMs:30});
 browser=await chromium.launch({headless:false,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox',...(process.env.VIDLAT_VIEWER_DEFAULT_GPU==='1'?[]:['--disable-gpu']),'--disable-dev-shm-usage','--js-flags=--max-old-space-size=1024','--window-size=1920,1080',`--disk-cache-dir=${root}/viewer/cache`,`--force-device-scale-factor=${Number((process.env.DPRS??'1').split(',')[0])}`,`--proxy-server=${network.proxyUrl}`],env:{...process.env,...denv,TMPDIR:paths.tmp}});
 for(const dpr of (process.env.DPRS??'1,2').split(',').map(Number)){
  const ctx=await browser.newContext({viewport:{width:1920,height:1080},deviceScaleFactor:dpr,...(process.env.VIDHOPS_RECORD==='1'?{recordVideo:{dir:E+'/recordings',size:{width:1920,height:1080}}}:{})});await ctx.addInitScript(observe);
  const p=await ctx.newPage();p.setDefaultTimeout(15000);await p.exposeBinding('__vidlatPhysicalInput',(_,v)=>{if(v.kind==='pointerdown'||v.kind==='keydown'||(pixels.active?.mode==='frames'&&v.kind==='wheel'))pixels.begin(v.at)});network.attach(p);
  const cell={dpr,status:'RUNNING',clicks:[],scroll:[],shortcuts:[]};report.cells.push(cell);
  try{
   await p.goto(state.webOrigin);await p.locator('#videoCanvas').waitFor({state:'attached',timeout:60000});await p.waitForTimeout(5000);const play=p.getByRole('button',{name:'Play Stream'});if(await play.isVisible())await play.click();await p.waitForTimeout(1000);
   if(await p.evaluate(()=>Boolean(document.fullscreenElement)))await p.keyboard.press('Escape');
   if(process.env.VIDLAT_MATCHED_GEOMETRY==='1')await p.addStyleTag({content:'#videoCanvas,#videoWorkerCanvas,#videoStream{position:fixed!important;left:589px!important;top:386px!important;width:716px!important;height:448px!important;max-width:none!important;max-height:none!important;}'});
   const surface=()=>p.locator('#videoCanvas:visible,#videoWorkerCanvas:visible,#videoStream:visible').first();
   async function navigate(url){const b=await surface().boundingBox();await p.mouse.click(b.x+b.width*.6,b.y+b.height*.1);await p.keyboard.press('Control+l');await p.waitForTimeout(300);await p.keyboard.type(url,{delay:40});await p.keyboard.press('Enter');await p.waitForTimeout(10000);if(new URL((await controls()).url).href!==new URL(url).href)throw Error('MP-10 native omnibox navigation missed requested page');}
   const sourceHeight=paths.ownerAB?887:800;
   async function point(bounds){const box=await surface().boundingBox(),k=Math.min(box.width/1280,box.height/sourceHeight);return {x:box.x+(box.width-1280*k)/2+(bounds.x+bounds.width/2)*k,y:box.y+(box.height-sourceHeight*k)/2+(bounds.y+bounds.height/2)*k};}
   await navigate('https://en.wikipedia.org/wiki/Web_browser');let first=await controls();if(first.bannerClose){const q=await point(first.bannerClose);await p.screenshot({path:E+`/dpr${dpr}-banner-before.png`});await p.mouse.click(q.x,q.y);await p.waitForTimeout(2000);}cell.article=await controls();await p.screenshot({path:E+`/dpr${dpr}-article.png`});
   const day=cell.article.controls.find(c=>c.id.endsWith('-day')),night=cell.article.controls.find(c=>c.id.endsWith('-night'));
   if(day&&night&&Math.max(day.bounds.y,night.bounds.y)<780){
    const q=await point(day.bounds);await p.mouse.click(q.x,q.y);await p.waitForTimeout(2000);
    for(let n=0;n<Number(process.env.TRIALS??20);n++){
     const dark=n%2===0,q=await point((dark?night:day).bounds),b=await surface().boundingBox();const nativeGeometry=await pixels.arm(p,{x:b.x+2*b.width/1280,y:b.y+(paths.ownerAB?187:100)*b.height/sourceHeight,width:30,height:30},'latency',{luminance:dark?[0,100]:[180,255]});if(nativeGeometry.target_already_visible)throw Error('MP-10 target theme already visible before input');await p.mouse.click(q.x,q.y);
     const physical=await pixels.result();const sample={input:physical.input_at,visible:physical.presented_at,latency:physical.latency_ms,physical,native_geometry:nativeGeometry};cell.clicks.push(sample);await save();await p.waitForTimeout(500);
    }
   }else cell.click_blocker='public theme controls absent/outside source viewport';
   await navigate('https://en.wikipedia.org/wiki/Web_browser');first=await controls();if(first.bannerClose){const q=await point(first.bannerClose);await p.mouse.click(q.x,q.y);await p.waitForTimeout(2000);}const box=await surface().boundingBox();await p.mouse.move(box.x+box.width*.7,box.y+box.height*.5);
   for(const [delta,hz]of [[120,20],[120,60],[16,60]]){
    const b=await surface().boundingBox();await pixels.arm(p,{x:b.x+b.width*.55,y:b.y+b.height*.5,width:128,height:128},'frames');const cpuBefore=await cpu.sample(),begin=Date.now(),pending=[];for(let n=0;n<hz*5;n++){const sent=p.mouse.wheel(0,(Math.floor(n/(hz/2))%2?-1:1)*delta);if(process.env.FAST_SCROLL==='1'&&hz===60)pending.push(sent);else await sent;await p.waitForTimeout(Math.max(0,begin+(n+1)*1000/hz-Date.now()));}await Promise.all(pending);await p.waitForTimeout(500);
    const row=await p.evaluate(({begin,delta,hz})=>({begin,end:begin+5000,delta,hz,input_count:probe.inputs.filter(x=>x.kind==='wheel'&&x.at>=begin&&x.at<begin+5000).length,frames:probe.frames.filter(f=>f.drawn>=begin&&f.drawn<begin+5000)}),{begin,delta,hz});row.cpu=cpuSpan(cpuBefore,await cpu.sample());row.dataset_fps=row.frames.length/5;row.native=await pixels.end(begin,begin+5000);row.fps=row.native.fps;cell.scroll.push(row);await save();
   }
   await navigate('https://www.wikipedia.org/');cell.portal=await controls();const input=cell.portal.controls.find(c=>c.id==='searchInput');
   if(input){const q=await point(input.bounds);await p.mouse.click(q.x,q.y);await p.keyboard.type('chariox');await p.waitForTimeout(1500);const typed=await controls();await p.screenshot({path:E+`/dpr${dpr}-shortcut-seed.png`});await p.keyboard.press('Control+a');await p.keyboard.press('Backspace');await p.waitForTimeout(1500);const cleared=await controls();await p.screenshot({path:E+`/dpr${dpr}-shortcut-cleared.png`});cell.shortcuts.push({typed:typed.controls.find(c=>c.id==='searchInput')?.value,after_ctrl_a_backspace:cleared.controls.find(c=>c.id==='searchInput')?.value,expected:''});}
   cell.typing=[];
   if(input){
    const b=await surface().boundingBox(),k=Math.min(b.width/1280,b.height/sourceHeight),field=input.bounds;
    const roi={x:b.x+(b.width-1280*k)/2+(field.x+8)*k,y:b.y+(b.height-sourceHeight*k)/2+(field.y+4)*k,width:Math.min(128,field.width*k-16),height:Math.min(40,field.height*k-8)};
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
   const button=cell.portal.controls.find(c=>c.id==='js-lang-list-button');if(button){const q=await point(button.bounds);cell.portal_button={source_bounds:button.bounds,point:q,surface:await surface().boundingBox()};cell.portal_toggles=[];for(let n=0;n<4;n++){await p.mouse.click(q.x,q.y);await p.waitForTimeout(1000);const cs=await controls();cell.portal_toggles.push({url:cs.url,expanded:cs.controls.find(c=>c.id==='js-lang-list-button')?.expanded});await p.screenshot({path:E+`/dpr${dpr}-portal-toggle${n}.png`});}}
   await p.screenshot({path:E+`/dpr${dpr}-portal.png`});cell.metrics=await p.evaluate(()=>probe);cell.status='MEASURED_NOT_ACCEPTED';
  }catch(e){cell.status='FAIL';cell.error=e.message.split('\n')[0].replace(/https?:\/\/\S+/g,'[URL]').slice(0,300);await p.screenshot({path:E+`/dpr${dpr}-failure.png`}).catch(()=>{});}finally{cell.metrics=await p.evaluate(()=>probe).catch(()=>({}));await save();await ctx.close();}
 }
 report.network=await network.snapshot();report.status=report.cells.every(c=>c.status==='MEASURED_NOT_ACCEPTED')?'MEASURED_NOT_ACCEPTED':'FAIL';
}catch(e){report.status='FAIL';report.error=e.message.split('\n')[0].slice(0,300);process.exitCode=1;}finally{report.cpu_samples=cpu.samples;await cpu.close();await browser?.close();await network?.close();await pixels?.close();await display.close();await ipc.close();await save();if(report.status==='FAIL')process.exitCode=1;console.log(JSON.stringify({items:report.items,status:report.status,cells:report.cells.map(c=>({dpr:c.dpr,status:c.status,clicks:c.clicks.map(x=>x.latency),scroll:c.scroll.map(x=>({delta:x.delta,hz:x.hz,fps:x.fps})),shortcuts:c.shortcuts,error:c.error})),error:report.error}));}

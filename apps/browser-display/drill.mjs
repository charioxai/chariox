// MD-DISPLAY-02/04: real kernel websocket + headed, sandboxed host browser.
// No credentials/providers/Cloud. External public tools are supplied explicitly.
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir, mkdtemp, chmod, chown, cp, rm, statfs, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { fixture } from './drill-fixtures.mjs';
import { compare, distribution } from './drill-metrics.mjs';
import { summarizeStages } from './drill-stages.mjs';
import { launchOwned, waitChild, stopGroup, checkChild } from './drill-owned-process.mjs';
const here = path.dirname(fileURLToPath(import.meta.url));
const [binary, output, tools, pytools] = process.argv.slice(2);
if (![binary,output,tools,pytools].every(value => value && path.isAbsolute(value))) throw Error('MD-DISPLAY: provide absolute kernel-test binary, evidence directory, Node tools and PyAV tools');
const require = createRequire(path.join(tools,'package.json'));
const { chromium } = require('playwright-core'), { PNG } = require('pngjs');
const pause = ms => new Promise(resolve => setTimeout(resolve,ms));
const receipt = { item: 'MD-DISPLAY-02/04', status: 'RED', source: execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(), source_dirty: Boolean(execFileSync('git',['status','--porcelain'],{encoding:'utf8'}).trim()), commands: process.argv.slice(1), codec: 'vp09.00.10.08 + exact PNG/tiles', target_encrypted_bitrate: Number(process.env.MD_BITRATE || 2_000_000), css_geometry:[1280,800],dpr:2, transport:'production local scoped-auth relay + kernel encrypted request/event path', samples:[], cleanup:[] };
await mkdir(output,{recursive:true});
const root = await mkdtemp(path.join(tmpdir(),'chariox-md-display-impl-'));
let kernel, display, viewer, browser, server, ready;
let kernelExit;
const ts=require('typescript');
const chrome=process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE;
const xvfb=process.env.CHARIOX_DISPLAY_XVFB || '/usr/bin/Xvfb';
if (!chrome || !path.isAbsolute(chrome) || !path.isAbsolute(xvfb)) throw Error('MD-DISPLAY: configure absolute native Chromium and Xvfb executables');
const relayCryptoSource=await readFile(path.resolve(here,'../../packages/kernel-client/src/browser-relay-crypto.ts'),'utf8');
const relayCrypto=ts.transpileModule(relayCryptoSource,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText;
const groups = [], errors = [], log = [];
async function resource() {
 const mem=await readFile('/proc/meminfo','utf8'), disk=await statfs('/');
 const sample={at:new Date().toISOString(),at_ms:performance.now(),mem_available_bytes:Number(mem.match(/^MemAvailable:\s+(\d+)/m)[1])*1024,disk_free_bytes:Number(disk.bavail)*Number(disk.bsize),processes:[]};
 for(const name of await readdir('/proc')) {
  if(!/^\d+$/.test(name))continue;
  try {const text=await readFile(`/proc/${name}/stat`,'utf8'),f=text.slice(text.lastIndexOf(')')+2).split(' ');const command=await readFile(`/proc/${name}/cmdline`,'utf8');if(groups.includes(Number(f[2]))||command.includes(root)||Number(name)===process.pid)sample.processes.push({pid:Number(name),cpu_ticks:Number(f[11])+Number(f[12]),rss_bytes:Number(f[21])*4096});}catch{}
 }
 receipt.samples.push(sample);
 if(sample.mem_available_bytes<16*1024**3||sample.disk_free_bytes<60_000_000_000)throw Error('MD-DISPLAY: resource floor');
 return sample;
}
async function until(check,label,timeout=20000) {const end=Date.now()+timeout;while(Date.now()<end){if(errors.length)throw errors[0];const value=await check();if(value)return value;await pause(25);}throw Error('MD-DISPLAY timeout: '+label);}
try {
 await resource();
 if(process.env.MD_RELAY==='0')throw Error('MD-DISPLAY: browser origins cannot attach to the native local socket; use the scoped relay drill');
 await chmod(root,0o755);
 const home=path.join(root,'home');await mkdir(home,{mode:0o700});await chown(home,65534,65534);
 await cp(binary,path.join(root,'kernel-tests'));await chmod(path.join(root,'kernel-tests'),0o755);
 const python=path.join(root,'python');await mkdir(python);
 for(const name of ['av','av.libs'])await cp(path.join(pytools,name),path.join(python,name),{recursive:true});
 const pythonWrapper=path.join(root,'encoder-python');
 await writeFile(pythonWrapper,`#!/bin/sh\nPYTHONPATH='${python}' exec /usr/bin/python3 "$@"\n`,{mode:0o755});
 display=await launchOwned(xvfb,['-displayfd','3','-screen','0','2560x1600x24','-nolisten','tcp','-ac'],{detached:true,stdio:['ignore','ignore','ignore','pipe']});groups.push(display.pid);
 let screen='';display.stdio[3].on('data',bytes=>screen+=bytes);await until(()=>{checkChild(display,'Xvfb');return screen.includes('\n')},'display');
 const sourceText=await readFile(path.resolve(here,'../../docs/MULTIDOMAIN_KERNEL_BROWSER.md'),'utf8');
 server=createServer(async(req,res)=>{
  try {
   const name=new URL(req.url,'http://localhost').pathname;
   const page=fixture(name,`http://127.0.0.1:${server.address().port}`,sourceText);
   if(page){res.setHeader('Content-Type','text/html');res.end(page);return;}
   if(name==='/browser-relay-crypto.mjs'){res.setHeader('Content-Type','text/javascript');res.end(relayCrypto);return;}
   if(name==='/relay-bootstrap'){res.setHeader('Content-Type','application/json');res.end(await readFile(path.join(root,'home','relay-bootstrap.private.json')));return;}
   const assets={'/harness.html':'harness.html','/presenter.mjs':'presenter.mjs'};
   if(!assets[name]){res.writeHead(404).end();return;}
   res.setHeader('Content-Type',name.endsWith('.mjs')?'text/javascript':'text/html');res.end(await readFile(path.join(here,assets[name])));
  }catch(error){errors.push(error);res.writeHead(500).end();}
 });
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const origin=`http://127.0.0.1:${server.address().port}`;
 kernel=await launchOwned(path.join(root,'kernel-tests'),['--ignored','--exact','runtime::router::tests::kernel_browser::display::kernel_browser_display_protocol_drill','--nocapture'],{uid:65534,gid:65534,detached:true,cwd:root,env:{PATH:'/usr/bin:/bin',HOME:home,TMPDIR:home,DISPLAY:`:${screen.trim()}`,CHARIOX_HOME:path.join(home,'chariox'),CHARIOX_LOG_DIR:path.join(home,'logs'),CHARIOX_DISPLAY_DRILL_ROOT:home,CHARIOX_DISPLAY_FIXTURE_URL:`${origin}/docs`,CHARIOX_KERNEL_BROWSER_EXECUTABLE:chrome,CHARIOX_BROWSER_CONTROLLER_NODE:process.execPath,CHARIOX_KERNEL_BROWSER_DISPLAY:'1',CHARIOX_BROWSER_DISPLAY_TIMING:'1',CHARIOX_BROWSER_DISPLAY_PYTHON:pythonWrapper},stdio:['ignore','pipe','pipe']});groups.push(kernel.pid);
 kernel.stdout.on('data',b=>log.push(b));kernel.stderr.on('data',b=>log.push(b));
 kernelExit=waitChild(kernel);
 ready=await until(async()=>{checkChild(kernel,'kernel');try{return JSON.parse(await readFile(path.join(home,'ready.json'),'utf8'))}catch{return null}},'focused MCP opens user-domain tab',45000);
 receipt.protocol=ready.protocol;receipt.opened_by=ready.opened_by;
 const viewerHome=path.join(root,'viewer');await mkdir(viewerHome,{mode:0o700});await chown(viewerHome,65534,65534);
 viewer=await launchOwned(chrome,['--headless=new','--remote-debugging-address=127.0.0.1','--remote-debugging-port=0',`--user-data-dir=${viewerHome}`,'--no-first-run','--disable-background-networking','--disable-dev-shm-usage','about:blank'],{uid:65534,gid:65534,detached:true,cwd:root,env:{PATH:'/usr/bin:/bin',HOME:viewerHome,TMPDIR:viewerHome},stdio:'ignore'});groups.push(viewer.pid);
 const port=await until(async()=>{checkChild(viewer,'viewer');try{return Number((await readFile(path.join(viewerHome,'DevToolsActivePort'),'utf8')).split('\n')[0])}catch{return null}},'viewer');
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
 const page=await browser.contexts()[0].newPage();page.on('pageerror',()=>errors.push(Error('MD-DISPLAY browser callback failure')));await page.goto(`${origin}/harness.html`);await page.waitForFunction(()=>window.MDDisplay);
 await page.evaluate(async({ready,bitrate,pngOnly})=>{
  if(pngOnly)globalThis.VideoDecoder=undefined;
  const api=await import('/browser-relay-crypto.mjs');
  const sender=await api.createRelayKeypair();
  const bootstrap=await (await fetch('/relay-bootstrap')).json();
  let socket,daemonKey;
  for(let attempt=0;attempt<100;attempt++){
    try {
     socket=new WebSocket(bootstrap.relay_url);await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject});
     const connected=new Promise((resolve,reject)=>{socket.onmessage=event=>{const m=JSON.parse(event.data);m.kind==='client_connected'?resolve(m):reject(Error('MD-DISPLAY relay target not ready'))};socket.onclose=()=>reject(Error('MD-DISPLAY relay closed before ready'))});
     socket.send(JSON.stringify({kind:'client_connect',auth_token:bootstrap.client_token,target:{daemon_id:bootstrap.daemon_id}}));
     daemonKey=(await connected).daemon_public_key;break;
    }catch{socket?.close();await new Promise(resolve=>setTimeout(resolve,100));}
   }
  if(!daemonKey)throw Error('MD-DISPLAY relay did not admit kernel target');
  let id=0;const pending=new Map(),listeners=new Set();window.mdWireBytes=0;window.mdFrames=[];window.mdTimings=[];
  const stamp=()=>performance.timeOrigin+performance.now();
  const timing=(stage,started)=>{const ended=stamp();mdTimings.push({stage,started_ms:started,ended_ms:ended,duration_ms:ended-started});};
  socket.onmessage=async event=>{
   window.mdWireBytes+=new TextEncoder().encode(event.data).length;
   const arrived=stamp();const message=JSON.parse(event.data);
   if(message.kind==='client_response'){
     const p=pending.get(message.request_id);pending.delete(message.request_id);
     if(!p)return;
     if(message.error)p.reject(Error(message.error.message));else try{p.resolve(JSON.parse(await api.decryptRelayPayload(sender.privateKey,message.encrypted_response,daemonKey)))}catch(error){p.reject(error)}
    }else if(message.kind==='client_event'){
     timing('event_received',arrived);
     const value=JSON.parse(await api.decryptRelayPayload(sender.privateKey,message.encrypted_event,daemonKey));
     timing('client_event_decrypt',arrived);
     window.mdFrames.push({sequence:value.frame.sequence,kind:value.frame.kind,bytes:event.data.length});for(const listener of listeners)listener(value);
   }
  };
  const control=value=>new Promise((resolve,reject)=>{const key=String(++id),timer=setTimeout(()=>{pending.delete(key);reject(Error('MD-DISPLAY request timeout'))},20000);pending.set(key,{resolve:value=>{clearTimeout(timer);resolve(value)},reject:error=>{clearTimeout(timer);reject(error)}});socket.send(JSON.stringify({request_id:key,...value}))});
  window.mdTransport={request:async request=>{
    const started=stamp();
    const encrypted=await api.encryptRelayPayload(daemonKey,JSON.stringify({command_id:'md-display-'+(id+1),request}),sender);
    timing('client_request_encrypt',started);
    const sent=stamp();const result=await control({kind:'client_request',target:{daemon_id:bootstrap.daemon_id},encrypted_request:encrypted.payload});
    timing(request.KernelBrowser?.command.op==='display_input'?'input_round_trip':'capture_or_control_round_trip',sent);return result;
   },onEvent:listener=>{listeners.add(listener);return()=>listeners.delete(listener)}};
  mdTransport.subscribeDisplay=binding=>control({kind:'client_subscribe',subscription_id:binding.subscription_id,target:{daemon_id:bootstrap.daemon_id},session_id:binding.subscription_id,attachment_id:String(binding.generation),client_public_key:sender.publicKeyBase64,subscription_scope:'kernel_browser_display',resume_from_event_id:null});
  mdTransport.unsubscribeDisplay=binding=>control({kind:'client_unsubscribe',subscription_id:binding.subscription_id,client_public_key:sender.publicKeyBase64});
  window.mdStream=await MDDisplay.attachBrowserDisplay(MDDisplay.canvas,mdTransport,{tab_id:ready.tab_id,generation:ready.generation},{bitrate,onTiming:timing,onPresented:frame=>{
    const sample={sequence:frame.sequence,drawn_ms:stamp()};window.mdPresentation=sample;
    requestAnimationFrame(()=>{
      if(window.mdProbeLeft!==undefined){const c=MDDisplay.canvas.getContext('2d');let n=0;for(let i=0;i<5;i++){const p=c.getImageData(mdProbeLeft+8+i*16,56,1,1).data;if(p[0]>128)n|=1<<i}sample.step=n;}
      sample.presented_ms=stamp();
    });
  }});
 },{ready,bitrate:receipt.target_encrypted_bitrate,pngOnly:process.env.MD_PNG_ONLY==='1'});
 const first=await page.evaluate(()=>mdStream.next());receipt.bootstrap={kind:first.kind,sequence:first.sequence};
 const reference=async()=>Buffer.from((await page.evaluate(async()=>{const r=await mdTransport.request({KernelBrowser:{command:{op:'display_capture',tab_id:mdStream.binding.tab_id,generation:mdStream.binding.generation}}});return r.KernelBrowser.result.data_base64})),'base64');
 const actual=async()=>Buffer.from((await page.evaluate(()=>MDDisplay.canvas.toDataURL('image/png'))).split(',')[1],'base64');
 async function pair(name) {
  const source=await reference(),view=await actual();const {diff,...metric}=compare(source,view,PNG);
  await writeFile(path.join(output,name+'-source.png'),source);await writeFile(path.join(output,name+'-viewer.png'),view);if(diff)await writeFile(path.join(output,name+'-diff.png'),diff);return metric;
 }
 receipt.bootstrap.fidelity=await pair('bootstrap-video');
 const settled=await page.evaluate(()=>mdStream.next());receipt.settled={kind:settled?.kind??'unchanged-exact',sequence:settled?.sequence??first.sequence,fidelity:await pair('settled')};
 if(!receipt.settled.fidelity.lossless)throw Error('MD-DISPLAY: settled pixels differ');
 // A source compositor may finish painting after its first protected snapshot.
 // Permit bounded distinct refinements, then require an unchanged exact poll.
 receipt.idle_refinements=0;
 while(await page.evaluate(()=>mdStream.next())!==null){
  if(++receipt.idle_refinements>5)throw Error('MD-DISPLAY: source did not settle');
 }
 const idleMs=Number(process.env.MD_IDLE_MS||0);
 if(idleMs){
  const idleStarted=performance.now();let polls=0;
  while(performance.now()-idleStarted<idleMs){await page.evaluate(()=>mdStream.next());polls++;await pause(250);}
  receipt.static_polling={duration_ms:performance.now()-idleStarted,polls};
 }
 const sourceProbe=PNG.sync.read(await reference());
 let probeLeft=null,runStart=null;
 for(let x=sourceProbe.width-360;x<sourceProbe.width;x++){
  const offset=(56*sourceProbe.width+x)*4,black=sourceProbe.data[offset]<5&&sourceProbe.data[offset+1]<5&&sourceProbe.data[offset+2]<5;
  if(black&&runStart===null)runStart=x;
  if(!black&&runStart!==null){if(x-runStart===80)probeLeft=runStart;runStart=null;}
 }
 if(probeLeft===null)throw Error('MD-DISPLAY: cannot bind fixture probe to captured viewport');
 receipt.probe_pixel_left=probeLeft;
 await page.evaluate(left=>{window.mdProbeLeft=left},probeLeft);
 const probes=[];const startBytes=await page.evaluate(()=>mdWireBytes),start=performance.now();
 for(let i=1;i<=20;i++) {
  await resource();
  const probe=await page.evaluate(async left=>{
   const stamp=()=>performance.timeOrigin+performance.now(), started=stamp();
   await mdStream.input({kind:'click',x:1190,y:28});
   const inputAck=stamp(),frame=await mdStream.next(),creditReleased=stamp();
   while(mdPresentation?.sequence!==frame?.sequence||!mdPresentation?.presented_ms)await new Promise(resolve=>requestAnimationFrame(resolve));
   const {drawn_ms,presented_ms,step}=mdPresentation;
   return {started_ms:started,input_ack_ms:inputAck,drawn_ms,presented_ms,credit_released_ms:creditReleased,latency_ms:presented_ms-started,step,kind:frame?.kind};
  },probeLeft);
  if(probe.step!==i){await pair('failed-probe-'+i);throw Error(`MD-DISPLAY: input visual acknowledgement ${i} got ${probe.step}`);}
  probes.push(probe.latency_ms);(receipt.probes??=[]).push(probe);if(!probe.kind)throw Error('MD-DISPLAY: missing changed frame');
 }
 const endResource=await resource();
 const firstResource=receipt.samples.find(sample=>sample.processes.some(process=>process.pid===kernel.pid));
 if(firstResource){const byPid=new Map(firstResource.processes.map(process=>[process.pid,process.cpu_ticks]));const delta=endResource.processes.reduce((total,process)=>total+Math.max(0,process.cpu_ticks-(byPid.get(process.pid)??process.cpu_ticks)),0);receipt.observed_owned_cpu_percent=delta/100/((endResource.at_ms-firstResource.at_ms)/1000)*100;receipt.cpu_note='live process deltas at Linux CLK_TCK=100; excludes already-exited encoder processes';}
 receipt.latency=distribution(probes);receipt.measurement_duration_ms=performance.now()-start;receipt.measured_local_response_bytes=(await page.evaluate(()=>mdWireBytes))-startBytes;receipt.local_bytes_per_second=receipt.measured_local_response_bytes*1000/receipt.measurement_duration_ms;receipt.frames=await page.evaluate(()=>mdFrames);receipt.client_timings=await page.evaluate(()=>mdTimings);
 receipt.after_input_verification=await page.evaluate(async()=>{const frame=await mdStream.next();return frame?{kind:frame.kind,sequence:frame.sequence}:null});
 receipt.final_fidelity=await pair('after-input');
 if(!receipt.final_fidelity.lossless)throw Error('MD-DISPLAY: small-change pixels differ');
 // Explicit stale document must fail through the production input seam.
 const stale=await page.evaluate(async()=>{try{await mdTransport.request({KernelBrowser:{command:{op:'display_input',tab_id:mdStream.binding.tab_id,generation:mdStream.binding.generation,document_id:'stale-fixture-loader',input:{kind:'click',x:1190,y:28}}}});return false}catch{return true}});if(!stale)throw Error('MD-DISPLAY: stale document admitted');receipt.stale_document_rejected=true;
 receipt.takeover=await page.evaluate(()=>mdStream.takeover());
 receipt.actors=await page.evaluate(()=>mdStream.actors());
 const agentProbe=async name=>{await writeFile(path.join(home,name),'MD-DISPLAY focused MCP probe');return until(async()=>{try{return JSON.parse(await readFile(path.join(home,name+'.json'),'utf8'))}catch{return null}},name);};
 receipt.agent_during_takeover=await agentProbe('PROBE_TAKEOVER');
 if(!receipt.agent_during_takeover.takeover_fenced || !receipt.agent_during_takeover.observed_document)throw Error('MD-DISPLAY: takeover did not fence focused MCP input');
 await page.evaluate(()=>mdStream.input({kind:'key',key:'Tab'}));
 await page.evaluate(()=>mdStream.release());
 receipt.agent_after_release=await agentProbe('PROBE_RELEASE');
 if(receipt.agent_after_release.rejected)throw Error('MD-DISPLAY: focused MCP input did not resume after release');
 if(process.env.MD_REGION_CAPTURE==='1') {
  // Keep the same subscription across real navigation, then capture via the
  // authenticated human relay request and production native-pixel crop path.
  const oldDocument=await page.evaluate(()=>mdStream.presenter.documentId);
  await page.evaluate(async url=>{
   await mdTransport.request({KernelBrowser:{command:{op:'navigate',tab_id:mdStream.binding.tab_id,generation:mdStream.binding.generation,url}}});
   await mdStream.next();
  },`${origin}/region`);
  const captured=await page.evaluate(async()=>{
   const binding=mdStream.binding;
   return mdTransport.request({CaptureVisibleRegion:{capture_id:'mdval-dpr2-region',surface:{kind:'kernel_browser',tab_id:binding.tab_id,generation:binding.generation},
    region:{x:95,y:95,width:230,height:30,viewport_width:1280,viewport_height:800,frame_width:2560,frame_height:1600}}});
  });
  const capture=captured.VisibleRegionCaptured?.capture;
  if(!capture||capture.width!==460||capture.height!==60)throw Error('MD-CAPTURE: native DPR2 crop geometry');
  const bytes=Buffer.from(capture.data_base64,'base64'),png=PNG.sync.read(bytes);
  const black=(x,y)=>{const p=(y*png.width+x)*4;return png.data[p]===0&&png.data[p+1]===0&&png.data[p+2]===0&&png.data[p+3]===255};
  const protectedPixels=[black(20,20),black(220,20),black(420,20)];
  if(!protectedPixels.every(Boolean)||black(0,0))throw Error('MD-CAPTURE: protected native pixels or retained background incorrect');
  await writeFile(path.join(output,'protected-region-dpr2.png'),bytes);
  receipt.region_capture={status:'PASS',width:capture.width,height:capture.height,protected_pixels:protectedPixels,retained_background:true,source:'authenticated encrypted local relay -> kernel owner authority -> sandboxed native Chromium -> trusted masks -> raster crop'};
  const newDocument=await page.evaluate(()=>mdStream.presenter.documentId);
  if(!oldDocument||!newDocument||oldDocument===newDocument)throw Error('MD-DISPLAY: navigation did not refresh subscription document');
  receipt.navigation_same_subscription={status:'PASS',before:oldDocument,after:newDocument};
 }
 await page.evaluate(()=>mdStream.close());
 await writeFile(path.join(home,'STOP'),'MD-DISPLAY owned stop');
 const exit=await kernelExit;receipt.kernel_exit=exit;if(exit.code!==0)throw Error('MD-DISPLAY kernel drill failed');
 await writeFile(path.join(output,'kernel.log'),Buffer.concat(log));
 receipt.status='PASS_LOCAL_COMPONENT';
 receipt.latency_goal={p50_ms:80,p95_ms:150,passed:receipt.latency.p50_ms<=80&&receipt.latency.p95_ms<=150};
 if(process.env.MD_REQUIRE_LATENCY==='1'&&!receipt.latency_goal.passed)throw Error('MD-DISPLAY: input-to-presentation latency goal remains RED');
} catch(error) {receipt.status='RED';receipt.error=String(error.message);process.exitCode=1;}
finally {
 if(kernel&&kernel.exitCode===null&&kernel.signalCode===null) {await writeFile(path.join(root,'home','STOP'),'MD-DISPLAY cleanup stop').catch(()=>{});await Promise.race([kernelExit,pause(5000)]);}
 try {await browser?.close();await stopGroup(viewer);await stopGroup(kernel);await stopGroup(display);await pause(500);
  const remaining=[];
  for(const name of await readdir('/proc'))if(/^\d+$/.test(name)){try{const command=await readFile(`/proc/${name}/cmdline`,'utf8');if(command.includes(root))remaining.push(Number(name));}catch{}}
  if(remaining.length)throw Error('MD-DISPLAY: owned-root processes remain: '+remaining.join(','));
  receipt.cleanup.push('owned Chromium/controller/encoder/kernel/Xvfb settled; exact-root process inventory empty');}catch(error){receipt.cleanup.push(error.message);receipt.status='RED';process.exitCode=1;}
 // Read only our non-secret, fixed-label diagnostic files before disposing state.
 const traces=[];
 async function collectTiming(directory){for(const entry of await readdir(directory,{withFileTypes:true}).catch(()=>[])){const p=path.join(directory,entry.name);if(entry.isDirectory()&&entry.name!=='profile')await collectTiming(p);else if(entry.name==='display-timing.jsonl'){const lines=(await readFile(p,'utf8')).trim().split('\n');for(const line of lines)if(line)traces.push(JSON.parse(line));}}}
 await collectTiming(path.join(root,'home','chariox'));
 receipt.host_timings=traces;receipt.kernel_timings=Buffer.concat(log).toString().split('\n').filter(line=>line.startsWith('MD-DISPLAY-TIMING ')).map(line=>JSON.parse(line.slice('MD-DISPLAY-TIMING '.length)));
 receipt.stage_breakdown=summarizeStages(receipt);
 if(server)await new Promise(resolve=>server.close(resolve));
 // Delete only the exact freshly-created disposable root, after owned teardown.
 if(!process.exitCode || receipt.cleanup.some(value=>value.includes('inventory empty'))){await rm(root,{recursive:true,force:true});receipt.cleanup.push('exact disposable state removed');}else receipt.cleanup.push('uncertain teardown state retained at '+root);
 await writeFile(path.join(output,'kernel.log'),Buffer.concat(log));
 receipt.finished_at=new Date().toISOString();await writeFile(path.join(output,'results.json'),JSON.stringify(receipt,null,2));console.log(JSON.stringify({item:receipt.item,status:receipt.status,error:receipt.error,latency:receipt.latency,lossless:receipt.settled?.fidelity.lossless}));
}

// MD-DISPLAY-02/04: real kernel websocket + headed, sandboxed host browser.
// No credentials/providers/Cloud. External public tools are supplied explicitly.
import {profileOwnedCpu} from './drill-profile.mjs';
import { createHash } from 'node:crypto';
import { createReadStream, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir, mkdtemp, chmod, chown, cp, rm, statfs, readdir, open, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import {nativeLoader} from './native-loader.mjs';
import { shapeViewerLeg, shapeViewerLegUserspace } from './drill-netem.mjs';
import {assertIndependentNavigation} from './drill-navigation.mjs';
import { drainRepairs, verifySettled } from './drill-settle.mjs';
import { measureWorkload } from './drill-workloads.mjs';
import { measureSiteLatency } from './drill-site-latency.mjs';
import { fixture } from './drill-fixtures.mjs';
import {CpuSampler,cpuSpan} from './drill-cpu.mjs';
import { distribution } from './drill-metrics.mjs';
import {MetricsWorker} from './drill-metrics-worker.mjs';
import {sourceIdentity} from './source-identity.mjs';
import {memoryFloorGiB} from './drill-resources.mjs';
import { summarizeStages } from './drill-stages.mjs';
import {stressProtection} from './drill-protection.mjs';
import {KernelLogCapture} from './drill-kernel-log.mjs';
import {collectRelayDiagnostics} from './drill-teardown.mjs';
import { launchOwned, waitChild, stopGroup, checkChild } from './drill-owned-process.mjs';
const here = path.dirname(fileURLToPath(import.meta.url));
const [binary, output, tools, pytools] = process.argv.slice(2);
if (![binary,output,tools,pytools].every(value => value && path.isAbsolute(value))) throw Error('MD-DISPLAY: provide absolute kernel-test binary, evidence directory, Node tools and PyAV tools');
const require = createRequire(path.join(tools,'package.json'));
let chromium, PNG, relayCrypto, metrics;
const pause = ms => new Promise(resolve => setTimeout(resolve,ms));
const geometry=process.env.MD_GEOMETRY==='1920x1080'?{width:1920,height:1080,dpr:1}:{width:1280,height:800,dpr:Number(process.env.MD_DPR||2)};
if(![1,2].includes(geometry.dpr))throw Error('MP-08/MP-11: unsupported drill DPR');
const receipt = { item: 'MD-DISPLAY-02/04', status: 'RED', ...await sourceIdentity(), commands: process.argv.slice(1), codec: null,requested_codec:process.env.MD_CODEC||'auto', target_encrypted_bitrate: Number(process.env.MD_BITRATE || 2_000_000), css_geometry:[geometry.width,geometry.height],dpr:geometry.dpr, transport:'production local scoped-auth relay + kernel encrypted request/event path', samples:[], cleanup:[] };
await mkdir(output,{recursive:true});
const root = await mkdtemp(path.join(process.env.MD_SCRATCH_PARENT || tmpdir(),'chariox-md-display-impl-'));
receipt.state_root=root;
const cpu=new CpuSampler({root,viewerRoot:path.join(root,'viewer')});cpu.start();
const runUid=Number(process.env.MD_RUNTIME_UID || 65534),runGid=Number(process.env.MD_RUNTIME_GID || 65534);
const chrome=process.env.MD_CHROME || '/usr/bin/google-chrome';
const runtimePath=process.env.MD_RUNTIME_PATH || '/usr/bin:/bin';
const quote=value=>"'"+value.replaceAll("'","'\"'\"'")+"'";
let kernel, display, viewer, browser, page, server, ready, shaped, shortTmp, kernelProfiler,stopCpuProfile,dynamicFixtureServed=false;
const workload=process.env.MD_WORKLOAD||'docs';
receipt.workload=workload;const fixtureStats=[];
receipt.requested_hardware=process.env.MD_SOFTWARE==='0';receipt.memory_floor_gib=memoryFloorGiB(process.env.MD_MEMORY_FLOOR_GIB);
receipt.requested_software_encoder=process.env.MD_ENCODER||'libx264';receipt.requested_converter=process.env.MD_LIBYUV?'libyuv':'auto';
let kernelExit;
const groups = [], errors = [], log = new KernelLogCapture();
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{receipt.interrupted=signal;errors.push(Error('MD-DISPLAY: interrupted '+signal));
 // Closing only this run's browser breaks any long repair evaluate; finally
 // still settles its owned process groups and writes the interrupted receipt.
 void browser?.close().catch(()=>{});
});
async function resource() {
 if(errors.length)throw errors[0];
 const mem=await readFile('/proc/meminfo','utf8'), disk=await statfs('/');
 const sample={at:new Date().toISOString(),at_ms:performance.now(),mem_available_bytes:Number(mem.match(/^MemAvailable:\s+(\d+)/m)[1])*1024,disk_free_bytes:Number(disk.bavail)*Number(disk.bsize),processes:[]};
 sample.cpu=await cpu.sample();sample.processes=sample.cpu.processes;receipt.samples.push(sample);
 if(sample.mem_available_bytes<memoryFloorGiB(process.env.MD_MEMORY_FLOOR_GIB)*1024**3||sample.disk_free_bytes<10*1024**3)throw Error('MD-DISPLAY: resource floor');
 return sample;
}
async function until(check,label,timeout=20000) {const end=Date.now()+timeout;while(Date.now()<end){if(errors.length)throw errors[0];const value=await check();if(value)return value;await pause(25);}throw Error('MD-DISPLAY timeout: '+label);}
try {
 const ts=require('typescript');
 ({chromium}=require('playwright-core'));({PNG}=require('pngjs'));metrics=new MetricsWorker(tools);
 const relayCryptoSource=await readFile(path.resolve(here,'../../packages/kernel-client/src/browser-relay-crypto.ts'),'utf8');
 relayCrypto=ts.transpileModule(relayCryptoSource,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText;
 await resource();
 if(process.env.MD_RELAY==='0')throw Error('MD-DISPLAY: browser origins cannot attach to the native local socket; use the scoped relay drill');
 // Chromium SingletonSocket uses TMPDIR, whose Unix path must stay short.
 // Persistent profile/CHARIOX_HOME stay below the requested LAN state root.
 shortTmp=await mkdtemp(path.join(tmpdir(),'chariox-md-tmp-'));await chmod(shortTmp,0o700);await chown(shortTmp,runUid,runGid);receipt.short_tmp_root=shortTmp;
 await writeFile(path.join(output,'run-roots.json'),JSON.stringify({item:'MP-08/MP-10/MP-11',state_root:root,short_tmp_root:shortTmp}));
 await chmod(root,0o755);
 const home=path.join(root,'home');await mkdir(home,{mode:0o700});await chown(home,runUid,runGid);
 const copiedBinary=path.join(root,'kernel-tests');
 await cp(binary,copiedBinary);await chmod(copiedBinary,0o755);
 const binaryFile=await open(copiedBinary,'r');
 try{const magic=Buffer.alloc(4);const read=await binaryFile.read(magic,0,4,0);if(read.bytesRead!==4||!magic.equals(Buffer.from([127,69,76,70])))throw Error('MD-DISPLAY: copied Linux test binary is not ELF; wait for build completion')}finally{await binaryFile.close()}
 const digest=createHash('sha256');for await(const bytes of createReadStream(copiedBinary))digest.update(bytes);
 receipt.binary={source_path:binary,copied_sha256:digest.digest('hex')};
 receipt.client_assets=[];for(const name of ['harness.html','presenter.mjs','stripe-presenter.mjs','decoder-worker.mjs','scroll-prediction.mjs','motion-samples.mjs']){const contents=await readFile(path.join(here,name));receipt.client_assets.push({name,sha256:createHash('sha256').update(contents).digest('hex')});}
 const override = {};
 const assetRoot=process.env.MD_ASSET_ROOT??path.resolve(here,'../kernel/slice-linux-docker/docker');
 if(!path.isAbsolute(assetRoot))throw Error('MP-11: explicit asset root must be absolute');
 receipt.controller_asset_root=assetRoot;
 if(process.env.MD_SOURCE_ASSETS==='1') {
   const assets=path.join(root,'controller-assets');await mkdir(assets);
   const inventory=await readFile(path.resolve(here,'../kernel/src/runtime/kernel_browser_assets.rs'),'utf8');
   receipt.controller_assets=[];
   for(const match of inventory.matchAll(/include_bytes!\("\.\.\/\.\.\/slice-linux-docker\/docker\/([^"/]+)"\)/g)) {
     const contents=await readFile(path.join(assetRoot,match[1]));
     await writeFile(path.join(assets,match[1]),contents);
     receipt.controller_assets.push({name:match[1],sha256:createHash('sha256').update(contents).digest('hex')});
   }
   override.CHARIOX_KERNEL_BROWSER_SCRIPT=path.join(assets,'kernel-browser-host.mjs');
   receipt.asset_mode='explicit product script override, current source hashes; binary identity separately bound';
 }
 receipt.chromium={path:chrome,version:execFileSync(chrome,['--version'],{encoding:'utf8'}).trim()};
 let openh264Adapter;
 if(process.env.MD_OPENH264_ADAPTER){
  const destination=path.join(root,'openh264');await mkdir(destination);
  const directory=path.dirname(process.env.MD_OPENH264_ADAPTER);receipt.openh264_assets=[];
  for(const name of ['chariox-openh264.so','libopenh264.so.8']){const source=path.join(directory,name),contents=await readFile(source);await cp(source,path.join(destination,name));receipt.openh264_assets.push({name,sha256:createHash('sha256').update(contents).digest('hex')});}
  openh264Adapter=path.join(destination,'chariox-openh264.so');receipt.openh264_mode='Cisco2.6.0 native SCREEN_CONTENT_REAL_TIME, verified by GetOption; externally runtime-downloaded library';
 }
 // MP-08/MP-10: stage a public native worker with this run's UID/path access.
 let nativeWorker;
 if(process.env.MD_NATIVE_WORKER){
  if(!path.isAbsolute(process.env.MD_NATIVE_WORKER))throw Error('MP-10: absolute native worker required');
  const executable=path.join(root,'native-worker-elf');await cp(process.env.MD_NATIVE_WORKER,executable);await chmod(executable,0o755);
  const digest=createHash('sha256');for await(const bytes of createReadStream(executable))digest.update(bytes);
  receipt.native_worker={source_path:process.env.MD_NATIVE_WORKER,copied_sha256:digest.digest('hex')};
  nativeWorker=executable;
  if(process.env.MD_BINARY_LOADER){
   if(!path.isAbsolute(process.env.MD_BINARY_LOADER)||!path.isAbsolute(process.env.MD_BINARY_LIBS||''))throw Error('MP-10: absolute worker loader/library paths required');
   const selected=nativeLoader(executable,process.env.MD_BINARY_LOADER,process.env.MD_BINARY_LIBS);
   nativeWorker=path.join(root,'native-worker');await writeFile(nativeWorker,`#!/bin/sh\nexec ${quote(selected.loader)} --library-path ${quote(selected.library_path)} ${quote(executable)} "$@"\n`,{mode:0o755});
   Object.assign(receipt.native_worker,selected);
  }
  if(process.env.MD_NATIVE_LIBYUV){
   const directory=path.join(root,'native-libs');await mkdir(directory);await cp(process.env.MD_NATIVE_LIBYUV,path.join(directory,'libyuv.so.0'));
   nativeWorker=path.join(root,'native-worker');await writeFile(nativeWorker,`#!/bin/sh\nLD_LIBRARY_PATH=${quote(directory)} exec ${quote(executable)} "$@"\n`,{mode:0o755});
   receipt.native_worker.converter_sha256=createHash('sha256').update(await readFile(process.env.MD_NATIVE_LIBYUV)).digest('hex');
  }
  if(process.env.MD_SOURCE_ASSETS==='1'){
   const wrapper=path.join(root,'native-controller-node');
   await writeFile(wrapper,`#!/bin/sh\nCHARIOX_BROWSER_DISPLAY_NATIVE_WORKER=${quote(nativeWorker)} exec ${quote(process.execPath)} "$@"\n`,{mode:0o755});
   override.CHARIOX_BROWSER_CONTROLLER_NODE=wrapper;
   receipt.native_worker.preview='explicit Node/source override; not embedded final-kernel validation';
  }
 }
 const python=path.join(root,'python');await mkdir(python);
 if(process.env.MD_LIBYUV){
  const contents=await readFile(process.env.MD_LIBYUV),destination=path.join(python,'libyuv.so.0');await writeFile(destination,contents);
  override.CHARIOX_BROWSER_DISPLAY_LIBYUV=destination;
  receipt.colour_converter={name:'libyuv ARGBToI420 (little-endian BGRx)',sha256:createHash('sha256').update(contents).digest('hex')};
 }
 for(const name of ['av','av.libs'])await cp(path.join(pytools,name),path.join(python,name),{recursive:true});
 const pythonWrapper=path.join(root,'encoder-python');
 const profiles=path.join(root,'profiles');let profiling='';
 if(process.env.MD_PROFILE==='1'){
  await mkdir(profiles,{mode:0o700});await chown(profiles,runUid,runGid);
  profiling=`if [ "$1" = "-u" ]; then shift; fi\ncase "$1" in\n *kernel-browser-encoder.py) set -- -m cProfile -o ${quote(profiles)}/encoder.$$.prof "$@" ;;\n *kernel-browser-xshm.py) set -- -m cProfile -o ${quote(profiles)}/capture.$$.prof "$@" ;;\nesac\n`;
  const nodeWrapper=path.join(root,'controller-node');
  await writeFile(nodeWrapper,`#!/bin/sh\nexec ${quote(process.execPath)} --cpu-prof --cpu-prof-dir=${quote(profiles)} "$@"\n`,{mode:0o755});
  override.CHARIOX_BROWSER_CONTROLLER_NODE=nodeWrapper;
  receipt.profiling='MP-10: V8/cProfile diagnostics enabled; timing overhead, separate from final comparison';
 }
 await writeFile(pythonWrapper,`#!/bin/sh\n${profiling}PYTHONPATH=${quote(python)} exec ${quote(process.env.MD_PYTHON || '/usr/bin/python3')} "$@"\n`,{mode:0o755});
 display=await launchOwned('/usr/bin/Xvfb',['-displayfd','3','-screen','0','2560x1600x24','-nolisten','tcp','-ac'],{detached:true,stdio:['ignore','ignore','ignore','pipe']});groups.push(display.pid);await cpu.track(display.pid);
 let screen='';display.stdio[3].on('data',bytes=>screen+=bytes);await until(()=>{checkChild(display,'Xvfb');return screen.includes('\n')},'display');
 const sourceText=await readFile(path.resolve(here,'../../docs/MULTIDOMAIN_KERNEL_BROWSER.md'),'utf8');
 server=createServer(async(req,res)=>{
  try {
   const name=new URL(req.url,'http://localhost').pathname;
   if(name==='/fixture-statistics'&&req.method==='POST'){let text='';for await(const chunk of req){text+=chunk;if(text.length>1024)throw Error('fixture statistics bound')};const value=JSON.parse(text);if(![value.updates,value.duration_ms,value.target_hz].every(Number.isFinite))throw Error('fixture statistics shape');fixtureStats.push(value);res.end('ok');return;}
   // MP-11 (owner 2026-10-08): a cross-site (isolated) consent-style frame with a protected field.
   if(name==='/frame-consent'){res.setHeader('Content-Type','text/html');res.end('<!doctype html><body style="margin:0;height:100vh;background:rgb(0,90,200)" onclick="document.body.style.background=\'rgb(0,200,90)\'"><input type="password" value="fixture" style="position:absolute;left:250px;top:120px;width:120px;height:40px;border:0"></body>');return;}
   const page=fixture(name,`http://127.0.0.1:${server.address().port}`,sourceText);
   if(page){
    res.setHeader('Content-Type','text/html');const dynamic=process.env.MD_DYNAMIC_PROTECTED==='1'&&!dynamicFixtureServed;if(dynamic)dynamicFixtureServed=true;
    const overlay=`<div id="protected-fixture" ${dynamic?'':'data-chariox-observation-protected'} style="position:fixed;left:900px;top:200px;width:150px;height:80px;background:red;color:white;z-index:100">Protected fixture</div>`;
    const button=dynamic?`<button style="position:fixed;left:720px;top:200px;width:160px;height:80px;z-index:100" onclick="document.querySelector('#protected-fixture').setAttribute('data-chariox-observation-protected','')">Protect</button>`:'';
    const frame=process.env.MD_FRAMES==='1'?`<iframe src="http://localhost:${server.address().port}/frame-consent" style="position:fixed;left:600px;top:400px;width:400px;height:200px;border:0;z-index:100"></iframe>`:'';
    res.end(process.env.MD_PROTECTED==='1'||frame?page.replace('</body>',(process.env.MD_PROTECTED==='1'?overlay+button:'')+frame+'</body>'):page);return;
   }
   if(name==='/browser-relay-crypto.mjs'){res.setHeader('Content-Type','text/javascript');res.end(relayCrypto);return;}
   if(name==='/relay-bootstrap'){const bootstrap=JSON.parse(await readFile(path.join(root,'home','relay-bootstrap.private.json'),'utf8'));if(shaped)bootstrap.relay_url=shaped.url;res.setHeader('Content-Type','application/json');res.end(JSON.stringify(bootstrap));return;}
   const assets={'/harness.html':'harness.html','/presenter.mjs':'presenter.mjs','/stripe-presenter.mjs':'stripe-presenter.mjs','/decoder-worker.mjs':'decoder-worker.mjs','/scroll-prediction.mjs':'scroll-prediction.mjs','/motion-samples.mjs':'motion-samples.mjs'};
   if(!assets[name]){res.writeHead(404).end();return;}
   res.setHeader('Content-Type',name.endsWith('.mjs')?'text/javascript':'text/html');res.end(await readFile(path.join(here,assets[name])));
  }catch(error){errors.push(error);res.writeHead(500).end();}
 });
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const origin=`http://127.0.0.1:${server.address().port}`;
 kernel=await launchOwned(process.env.MD_BINARY_LOADER || path.join(root,'kernel-tests'),[...(process.env.MD_BINARY_LOADER ? ['--library-path',process.env.MD_BINARY_LIBS,path.join(root,'kernel-tests')] : []),'--ignored','--exact','runtime::router::tests::kernel_browser::display::kernel_browser_display_protocol_drill','--nocapture'],{uid:runUid,gid:runGid,detached:true,cwd:root,env:{...override,PATH:runtimePath,HOME:home,TMPDIR:shortTmp,DISPLAY:`:${screen.trim()}`,CHARIOX_HOME:path.join(home,'chariox'),CHARIOX_LOG_DIR:path.join(home,'logs'),CHARIOX_DISPLAY_DRILL_ROOT:home,CHARIOX_DISPLAY_FIXTURE_URL:`${origin}/${workload}`,CHARIOX_KERNEL_BROWSER_EXECUTABLE:chrome,CHARIOX_KERNEL_BROWSER_DISPLAY:'1',CHARIOX_BROWSER_DISPLAY_TIMING:'1',CHARIOX_BROWSER_DISPLAY_GEOMETRY:process.env.MD_GEOMETRY,LIBVA_DRIVER_NAME:process.env.LIBVA_DRIVER_NAME,CHARIOX_BROWSER_DISPLAY_SOFTWARE:process.env.MD_SOFTWARE,CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER:process.env.MD_ENCODER,CHARIOX_BROWSER_DISPLAY_STRIPE_WORKERS:process.env.MD_STRIPE_WORKERS,CHARIOX_BROWSER_DISPLAY_OPENH264_ADAPTER:openh264Adapter,CHARIOX_BROWSER_DISPLAY_PYTHON:pythonWrapper,CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER:nativeWorker},stdio:['ignore','pipe','pipe']});groups.push(kernel.pid);await cpu.track(kernel.pid);
 kernel.stdout.on('data',b=>log.record('stdout',b));kernel.stderr.on('data',b=>log.record('stderr',b));
 kernelExit=waitChild(kernel);
 ready=await until(async()=>{checkChild(kernel,'kernel');try{return JSON.parse(await readFile(path.join(home,'ready.json'),'utf8'))}catch{return null}},'focused MCP opens user-domain tab',45000);
 receipt.protocol=ready.protocol;receipt.opened_by=ready.opened_by;
 if(process.env.MD_KERNEL_PERF){
  if(!path.isAbsolute(process.env.MD_KERNEL_PERF))throw Error('MP-10: absolute profiler executable required');
  // Sample instruction addresses only; never capture stacks/runtime key bytes.
  kernelProfiler=await launchOwned(process.env.MD_KERNEL_PERF,['record','-q','--no-inherit','-e','cpu-clock','-F','99','-p',String(kernel.pid),'-o',path.join(output,'kernel.perf.data')],{detached:true,stdio:'ignore',env:{PATH:runtimePath,...(process.env.MD_PERF_LIBS?{LD_LIBRARY_PATH:process.env.MD_PERF_LIBS}:{})}});
  receipt.kernel_profiling='MP-10: instruction-only cpu-clock samples of this owned kernel; no stack or memory capture';
 }
 if(process.env.MD_SOURCE_ASSETS!=='1'){
  // Only public controller source files; never profile/config/auth/identity files.
  const base=path.join(home,'chariox','kernel-browser','controller-assets');
  const directories=(await readdir(base,{withFileTypes:true})).filter(d=>d.isDirectory()&&/^[a-f0-9]{64}$/.test(d.name));
  if(directories.length!==1)throw Error('MD-DISPLAY: ambiguous materialized controller assets');
  const inventory=await readFile(path.resolve(here,'../kernel/src/runtime/kernel_browser_assets.rs'),'utf8');receipt.controller_assets=[];
  for(const match of inventory.matchAll(/include_bytes!\("\.\.\/\.\.\/slice-linux-docker\/docker\/([^"/]+)"\)/g)){
   const actual=await readFile(path.join(base,directories[0].name,match[1]));const expected=await readFile(path.join(assetRoot,match[1]));
   const sha256=createHash('sha256').update(actual).digest('hex');if(sha256!==createHash('sha256').update(expected).digest('hex'))throw Error('MD-DISPLAY: embedded controller differs from measured source');
   receipt.controller_assets.push({name:match[1],sha256});
  }
  if(!receipt.controller_assets.length)throw Error('MD-DISPLAY: empty embedded asset inventory');
  receipt.asset_mode='actual embedded controller materialization, allowlisted source SHA match; no script override';
 }

 if(process.env.MD_NETEM_PROFILE&&process.env.MD_NETEM_PROFILE!=='local'){
  const bootstrap=JSON.parse(await readFile(path.join(home,'relay-bootstrap.private.json'),'utf8'));
  shaped=process.env.MD_USERSPACE_SHAPING==='1'?await shapeViewerLegUserspace(process.env.MD_NETEM_PROFILE,bootstrap.relay_url):await shapeViewerLeg(process.env.MD_NETEM_PROFILE,bootstrap.relay_url,process.env.MD_HOST_NETNS);receipt.network=shaped.info;
 }else receipt.network={name:'local',rtt:0,jitter:0,loss:0,mbps:0};
 const viewerHome=path.join(root,'viewer');await mkdir(viewerHome,{mode:0o700});await chown(viewerHome,runUid,runGid);
 viewer=await launchOwned(chrome,['--headless=new','--remote-debugging-address=127.0.0.1','--remote-debugging-port=0',`--user-data-dir=${viewerHome}`,'--no-first-run','--disable-background-networking','--disable-dev-shm-usage','about:blank'],{uid:runUid,gid:runGid,detached:true,cwd:root,env:{PATH:runtimePath,HOME:viewerHome,TMPDIR:shortTmp},stdio:'ignore'});groups.push(viewer.pid);await cpu.track(viewer.pid,'viewer');
 const port=await until(async()=>{checkChild(viewer,'viewer');try{return Number((await readFile(path.join(viewerHome,'DevToolsActivePort'),'utf8')).split('\n')[0])}catch{return null}},'viewer');
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
 page=await browser.contexts()[0].newPage();page.on('pageerror',()=>errors.push(Error('MD-DISPLAY browser callback failure')));await page.goto(`${origin}/harness.html`);await page.waitForFunction(()=>window.MDDisplay);
 await page.evaluate(async({ready,bitrate,pngOnly,creditWindow,requestedCodec,dpr,defaultDpr,protectedFixture,dynamicProtection,stripes,legacyRelay,requireBinary})=>{
  if(pngOnly)globalThis.VideoDecoder=undefined;
  const api=await import('/browser-relay-crypto.mjs');
  const sender=await api.createRelayKeypair();
  const bootstrap=await (await fetch('/relay-bootstrap')).json();
  let socket,daemonKey,relayProtocolVersion=0;
  for(let attempt=0;attempt<100;attempt++){
    try {
     socket=await api.connectRelaySocket(bootstrap.relay_url,!legacyRelay);
     const connected=new Promise((resolve,reject)=>{socket.onmessage=event=>{const m=JSON.parse(event.data);m.kind==='client_connected'?resolve(m):reject(Error('MD-DISPLAY relay target not ready'))};socket.onclose=()=>reject(Error('MD-DISPLAY relay closed before ready'))});
     socket.send(JSON.stringify({kind:'client_connect',auth_token:bootstrap.client_token,target:{daemon_id:bootstrap.daemon_id}}));
     const admitted=await connected;daemonKey=admitted.daemon_public_key;relayProtocolVersion=socket.protocol==='chariox-relay-binary-v96'?96:0;break;
    }catch{socket?.close();await new Promise(resolve=>setTimeout(resolve,100));}
   }
  if(!daemonKey)throw Error('MD-DISPLAY relay did not admit kernel target');
  if(requireBinary&&relayProtocolVersion<96)throw Error('MP-08/MP-10: binary relay96 was not negotiated');
  window.mdRelayProtocolVersion=relayProtocolVersion;window.mdBinaryRelayFrames=0;
  let id=0;const pending=new Map(),listeners=new Set();window.mdWireBytes=0;window.mdFrames=[];window.mdPresentations=[];window.mdTimings=[];
  const stamp=()=>performance.timeOrigin+performance.now();
  const timing=(stage,started)=>{const ended=stamp();mdTimings.push({stage,started_ms:started,ended_ms:ended,duration_ms:ended-started});};
  socket.onmessage=async event=>{
   const binary=event.data instanceof ArrayBuffer;
   if(binary)window.mdBinaryRelayFrames++;
   window.mdWireBytes+=binary?event.data.byteLength:new TextEncoder().encode(event.data).length;
   const arrived=stamp();const message=binary?api.decodeBinaryRelayEvent(new Uint8Array(event.data)):JSON.parse(event.data);
   if(message.kind==='client_response'){
     const p=pending.get(message.request_id);pending.delete(message.request_id);
     if(!p)return;
     if(message.error)p.reject(Error(message.error.message));else try{p.resolve(JSON.parse(await api.decryptRelayPayload(sender.privateKey,message.encrypted_response,daemonKey)))}catch(error){p.reject(error)}
    }else if(message.kind==='client_event'){
     timing('event_received',arrived);
     const value=await api.decryptRelayEvent(sender.privateKey,message.encrypted_event,daemonKey);
     timing('client_event_decrypt',arrived);
     window.mdFrames.push({sequence:value.frame.sequence,kind:value.frame.kind,codec:value.frame.codec??null,bytes:binary?event.data.byteLength:event.data.length,arrived_ms:arrived});for(const listener of listeners)listener(value);
   }
  };
  const control=(value,reserved)=>new Promise((resolve,reject)=>{const key=reserved??String(++id),timer=setTimeout(()=>{pending.delete(key);reject(Error('MD-DISPLAY request timeout'))},20000);pending.set(key,{resolve:value=>{clearTimeout(timer);resolve(value)},reject:error=>{clearTimeout(timer);reject(error)}});socket.send(JSON.stringify({request_id:key,...value}))});
  window.mdTransport={kernelProtocolVersion:ready.protocol,displayEventEncoding:'CXD1',relayProtocolVersion,request:async request=>{
    const started=stamp(),reserved=String(++id);
    const encrypted=await api.encryptRelayPayload(daemonKey,JSON.stringify({command_id:'md-display-'+reserved,request}),sender);
    timing('client_request_encrypt',started);
    const sent=stamp();const result=await control({kind:'client_request',target:{daemon_id:bootstrap.daemon_id},encrypted_request:encrypted.payload},reserved);
    timing(request.KernelBrowser.command.op==='display_input'?'input_round_trip':'capture_or_control_round_trip',sent);return result;
   },onEvent:listener=>{listeners.add(listener);return()=>listeners.delete(listener)}};
  mdTransport.subscribeDisplay=binding=>control({kind:'client_subscribe',subscription_id:binding.subscription_id,target:{daemon_id:bootstrap.daemon_id},session_id:binding.subscription_id,attachment_id:String(binding.generation),client_public_key:sender.publicKeyBase64,subscription_scope:'kernel_browser_display',resume_from_event_id:null});
  mdTransport.unsubscribeDisplay=binding=>control({kind:'client_unsubscribe',subscription_id:binding.subscription_id,client_public_key:sender.publicKeyBase64});
  const motionSamples=new (await import('/motion-samples.mjs')).MotionSamples();window.mdProtection={enabled:!dynamicProtection,frames:0,violations:0,kinds:{},failures:[]};
  window.mdStream=await MDDisplay.attachBrowserDisplay(MDDisplay.canvas,mdTransport,{tab_id:ready.tab_id,generation:ready.generation},{bitrate,creditWindow,stripes,...(defaultDpr?{}:{deviceScaleFactor:dpr}),codec:requestedCodec,onTiming:timing,onPresented:frame=>{
    if(protectedFixture&&mdProtection.enabled){
      const pixels=MDDisplay.canvas.getContext('2d').getImageData(908*dpr,208*dpr,134*dpr,64*dpr).data;
      // MP-11: lossy H264/VP8 may shift a black mask's decoded RGB floor.
      // Exact PNG is black; codec frames must remain dark and hide the red/text.
      const limit=frame.kind==='png'?0:64;
      let opaque=true;for(let i=0;i<pixels.length;i+=4)if(pixels[i]>limit||pixels[i+1]>limit||pixels[i+2]>limit||pixels[i+3]!==255)opaque=false;
      mdProtection.frames++;mdProtection.violations+=Number(!opaque);mdProtection.kinds[frame.kind]=(mdProtection.kinds[frame.kind]??0)+1;
      if(!opaque&&mdProtection.failures.length<16){
        let maximum=0,above=0;for(let i=0;i<pixels.length;i+=4){maximum=Math.max(maximum,pixels[i],pixels[i+1],pixels[i+2]);if(pixels[i]>limit||pixels[i+1]>limit||pixels[i+2]>limit)above++;}
        mdProtection.failures.push({sequence:frame.sequence,kind:frame.kind,document:frame.document_id,pixel:Array.from(pixels.slice(0,4)),maximum,above,png:MDDisplay.canvas.toDataURL('image/png')});
      }
    }
    const sample={sequence:frame.sequence,kind:frame.kind,drawn_ms:stamp(),content_changed:motionSamples.sample(MDDisplay.canvas)};window.mdPresentation=sample;mdPresentations.push(sample);window.mdOnPresented?.(sample);
    requestAnimationFrame(()=>{
      if(window.mdProbeLeft!==undefined){const c=MDDisplay.canvas.getContext('2d');let n=0;for(let i=0;i<5;i++){const p=c.getImageData(mdProbeLeft+4*dpr+i*8*dpr,28*dpr,1,1).data;if(p[0]>128)n|=1<<i}sample.step=n;}
      sample.presented_ms=stamp();
    });
  }});
 },{ready,bitrate:receipt.target_encrypted_bitrate,pngOnly:process.env.MD_PNG_ONLY==='1',creditWindow:Number(process.env.MD_CREDIT_WINDOW||4),requestedCodec:process.env.MD_CODEC||null,dpr:geometry.dpr,defaultDpr:process.env.MD_DEFAULT_DPR==='1',protectedFixture:process.env.MD_PROTECTED==='1',dynamicProtection:process.env.MD_DYNAMIC_PROTECTED==='1',stripes:process.env.MD_STRIPES!=='0',legacyRelay:process.env.MD_LEGACY_RELAY==='1',requireBinary:process.env.MD_REQUIRE_BINARY_RELAY==='1'});
 receipt.relay_protocol_version=await page.evaluate(()=>mdRelayProtocolVersion);receipt.binary_relay_frames=await page.evaluate(()=>mdBinaryRelayFrames);
 receipt.decode_support=await page.evaluate(async()=>Object.fromEntries(await Promise.all(['avc1.420033','vp8'].map(async codec=>[codec,typeof VideoDecoder==='function'&&Boolean((await VideoDecoder.isConfigSupported({codec})).supported)]))));
 receipt.default_dpr_negotiation=process.env.MD_DEFAULT_DPR==='1';
 if(receipt.default_dpr_negotiation&&await page.evaluate(()=>mdStream.binding.device_scale_factor)!==1)throw Error('MP-08: #893 default DPR is unsupported');
 if(process.env.MD_GEOMETRY_PROBE==='1'){
  receipt.unsupported_dpr_rejected=await page.evaluate(async ready=>{try{const reply=await mdTransport.request({KernelBrowser:{command:{op:'display_subscribe',tab_id:ready.tab_id,generation:ready.generation,codecs:['avc1.420033','png','chariox-video-dependencies-v1','chariox-stripes-v1'],bitrate:8000000,device_scale_factor:2}}});return Boolean(reply.Error)}catch{return true}},ready);
  if(!receipt.unsupported_dpr_rejected)throw Error('MP-08: #893 admitted unsupported 1080p DPR2');
 }
 receipt.codec=await page.evaluate(()=>mdStream.binding.codec);receipt.delivery=await page.evaluate(()=>mdStream.push?'push-ack-475':'credit');receipt.codec_provenance='kernel negotiated binding; delivered video codecs retained in frame metadata';
 const bootstrapStarted=performance.now();
 const first=await until(()=>page.evaluate(()=>mdStream.next()),'first asynchronous display frame');receipt.bootstrap={kind:first.kind,sequence:first.sequence,duration_ms:performance.now()-bootstrapStarted};
 const reference=async()=>Buffer.from((await page.evaluate(async()=>{const r=await mdTransport.request({KernelBrowser:{command:{op:'display_capture',tab_id:mdStream.binding.tab_id,generation:mdStream.binding.generation}}});return r.KernelBrowser.result.data_base64})),'base64');
 const actual=async()=>page.evaluate(()=>({png:MDDisplay.canvas.toDataURL('image/png'),presentation:{...mdPresentation}}));
 async function pair(name) {
  const source=await reference(),snapshot=await actual(),view=Buffer.from(snapshot.png.split(',')[1],'base64');const {diff,...metric}=await metrics.compare(source,view);
  metric.presentation_ms=snapshot.presentation.presented_ms;metric.presentation_drawn_ms=snapshot.presentation.drawn_ms;metric.presentation_sequence=snapshot.presentation.sequence;metric.presentation_kind=snapshot.presentation.kind;
  await writeFile(path.join(output,name+'-source.png'),source);await writeFile(path.join(output,name+'-viewer.png'),view);if(diff)await writeFile(path.join(output,name+'-diff.png'),diff);
  if(process.env.MD_PROTECTED==='1'){
   const frame=PNG.sync.read(view),admitted=PNG.sync.read(source),dpr=geometry.dpr,limit=metric.lossless||metric.presentation_kind==='png'?0:64;let opaque=true,sourceOpaque=true;
   for(let y=208*dpr;y<272*dpr;y++)for(let x=908*dpr;x<1042*dpr;x++){
    const i=(y*frame.width+x)*4;if(frame.data[i]>limit||frame.data[i+1]>limit||frame.data[i+2]>limit||frame.data[i+3]!==255)opaque=false;
    const j=(y*admitted.width+x)*4;if(admitted.data[j]||admitted.data[j+1]||admitted.data[j+2]||admitted.data[j+3]!==255)sourceOpaque=false;
   }
   (receipt.protected_checks??=[]).push({step:name,opaque,source_opaque:sourceOpaque});
   if(!sourceOpaque)throw Error('MP-11: protected source pixels must be black before encoding');
   if(!opaque)throw Error('MP-11: protected source region must be opaque in presented pixels');
  }
  return metric;
 }
 if(process.env.MD_DYNAMIC_PROTECTED==='1'){
  const before=await actual();await writeFile(path.join(output,'before-protection-marker-viewer.png'),Buffer.from(before.png.split(',')[1],'base64'));
  await page.evaluate(()=>mdStream.input({kind:'click',x:800,y:240}));
  const protectedFrame=await until(()=>page.evaluate(()=>mdStream.next()),'attribute-only protected display frame');
  receipt.protection_insertion={initial_sequence:first.sequence,protected_sequence:protectedFrame.sequence,kind:protectedFrame.kind,fidelity:await pair('after-protection-marker')};
  await page.evaluate(()=>mdProtection.enabled=true);
 }
 receipt.bootstrap.fidelity=await pair('bootstrap-video');
 const settleStarted=performance.now();
 const settled=await verifySettled(()=>page.evaluate(()=>mdStream.next()),attempt=>pair('settled-verification-'+attempt));
 receipt.settle_duration_ms=performance.now()-settleStarted;receipt.settled={kind:'verified-unchanged',polls:settled.polls,verification_attempts:settled.verification_attempts,sequence:await page.evaluate(()=>mdStream.presenter.sequence),fidelity:settled.fidelity};
 if(!receipt.settled.fidelity.lossless)throw Error('MD-DISPLAY: settled pixels differ');
 if(process.env.MD_SITE_LATENCY==='1')receipt.site_latency=await measureSiteLatency({page,pause,pair,samples:Number(process.env.MD_SITE_SAMPLES||40),secondTab:process.env.MD_SECOND_TAB==='1'});
 else {
 if(process.env.MD_FRAMES==='1'){
  // MP-11: the isolated frame is visible and clickable; its protected field stays masked.
  const dpr=geometry.dpr,read=async()=>{await page.evaluate(()=>mdStream.next());return PNG.sync.read(Buffer.from((await actual()).png.split(',')[1],'base64'))};
  const px=(img,x,y)=>{const i=(y*dpr*img.width+x*dpr)*4;return [img.data[i],img.data[i+1],img.data[i+2]]},near=(a,b)=>a.every((v,i)=>Math.abs(v-b[i])<=24);
  let img=await read();receipt.frame_checks={visible:px(img,620,420),field:px(img,910,540)};
  if(!near(receipt.frame_checks.visible,[0,90,200]))throw Error('MP-11: isolated consent frame must be visible');
  if(!near(receipt.frame_checks.field,[0,0,0]))throw Error('MP-11: protected field inside an isolated frame must be masked');
  await page.evaluate(()=>mdStream.input({kind:'click',x:650,y:450}));
  for(const deadline=performance.now()+5000;!near(px(img=await read(),620,420),[0,200,90]);await new Promise(r=>setTimeout(r,50)))if(performance.now()>deadline)throw Error('MP-11: click into the isolated frame was not presented');
  Object.assign(receipt.frame_checks,{clicked:px(img,620,420),field_after:px(img,910,540)});await writeFile(path.join(output,'frames-clicked-viewer.png'),PNG.sync.write(img));
  if(!near(receipt.frame_checks.field_after,[0,0,0]))throw Error('MP-11: protected field inside an isolated frame must stay masked');
 }
 // A source compositor may finish painting after its first protected snapshot.
 // Permit bounded distinct refinements, then require an unchanged exact poll.
 receipt.idle_refinements=0;
 while(await page.evaluate(()=>mdStream.next())!==null){
  if(++receipt.idle_refinements>300)throw Error('MD-DISPLAY: source did not settle');
 }
 const idleMs=Number(process.env.MD_IDLE_MS||0);
 if(idleMs){
  const idleCpu=(await resource()).cpu;const idleFrames=await page.evaluate(()=>mdFrames.length);const idleStarted=performance.now();let polls=0;
  if(process.env.MD_WINDOW==='1')await page.evaluate(()=>mdStream.start());
  while(performance.now()-idleStarted<idleMs){if(process.env.MD_WINDOW!=='1')await page.evaluate(()=>mdStream.next());polls++;await pause(250);}
  if(process.env.MD_WINDOW==='1')await page.evaluate(()=>mdStream.stop());
  receipt.static_polling={mode:process.env.MD_WINDOW==='1'?'continuous credits':'manual250ms polls',duration_ms:performance.now()-idleStarted,polls,cpu:cpuSpan(idleCpu,(await resource()).cpu),video_mbps:(await page.evaluate(start=>mdFrames.slice(start).reduce((n,f)=>n+f.bytes,0),idleFrames))*8/(performance.now()-idleStarted)/1000};
 }
 const sourceProbe=PNG.sync.read(await reference());
 let probeLeft=null,runStart=null;
 for(let x=sourceProbe.width-360;x<sourceProbe.width;x++){
  const offset=(Math.round(28*geometry.dpr)*sourceProbe.width+x)*4,black=sourceProbe.data[offset]<5&&sourceProbe.data[offset+1]<5&&sourceProbe.data[offset+2]<5;
  if(black&&runStart===null)runStart=x;
  if(!black&&runStart!==null){if(x-runStart===40*geometry.dpr)probeLeft=runStart;runStart=null;}
 }
 if(probeLeft===null)throw Error('MD-DISPLAY: cannot bind fixture probe to captured viewport');
 receipt.probe_pixel_left=probeLeft;
 await page.evaluate(left=>{window.mdProbeLeft=left},probeLeft);
 if(workload!=='docs'){
  stopCpuProfile=await profileOwnedCpu(await cpu.sample(),output);
  receipt.motion=await measureWorkload({page,workload,pair,pause,resource,durationMs:Number(process.env.MD_MOTION_MS||10000)});
  await stopCpuProfile();stopCpuProfile=null;
 }
 const inputOffset=receipt.motion?.typing?.samples?.length??0;
 const probes=[];const clickCpu=(await resource()).cpu;
 if(process.env.MD_WINDOW==='1')await page.evaluate(()=>mdStream.start());
 const startBytes=await page.evaluate(()=>mdWireBytes),start=performance.now();
 for(let i=1;i<=20;i++) {
  await resource();
  const probe=await page.evaluate(async({left,expected,continuous})=>{
   const stamp=()=>performance.timeOrigin+performance.now(), started=stamp();
   await mdStream.input({kind:'click',x:mdStream.presenter.canvas.width/mdStream.binding.device_scale_factor-90,y:28});
   const inputAck=stamp();
   let frame;
   if(continuous) {
     const deadline=stamp()+10000;
     while(mdPresentation?.step!==expected||!mdPresentation?.presented_ms) {
       if(mdStream.error)throw mdStream.error;
       if(stamp()>deadline)throw Error('MD-DISPLAY visual acknowledgement timeout');
       await new Promise(resolve=>requestAnimationFrame(resolve));
     }
     frame={sequence:mdPresentation.sequence,kind:'continuous'};
   } else {
     const deadline=stamp()+10000;
     do {frame=await mdStream.next();if(!frame){if(stamp()>deadline)throw Error('MD-DISPLAY asynchronous visual acknowledgement timeout');await new Promise(resolve=>setTimeout(resolve,4));}} while(!frame);
   }
   const creditReleased=stamp();
   while(mdPresentation?.sequence!==frame?.sequence||!mdPresentation?.presented_ms)await new Promise(resolve=>requestAnimationFrame(resolve));
   const {drawn_ms,presented_ms,step}=mdPresentation;
   return {started_ms:started,input_ack_ms:inputAck,drawn_ms,presented_ms,credit_released_ms:creditReleased,latency_ms:presented_ms-started,step,kind:frame?.kind};
  },{left:probeLeft,expected:(i+inputOffset)&31,continuous:process.env.MD_WINDOW==='1'});
  if(probe.step!==((i+inputOffset)&31)){await pair('failed-probe-'+i);throw Error(`MD-DISPLAY: input visual acknowledgement ${i} got ${probe.step}`);}
  probes.push(probe.latency_ms);(receipt.probes??=[]).push(probe);if(!probe.kind)throw Error('MD-DISPLAY: missing changed frame');
 }
 if(process.env.MD_WINDOW!=='1')await page.evaluate(()=>mdStream.start());
 receipt.click_cpu=cpuSpan(clickCpu,(await resource()).cpu);
 const typeCpu=(await resource()).cpu;const typeProbes=[];for(let i=21;i<=40;i++){
  await page.evaluate(async()=>mdStream.input({kind:'click',x:mdStream.presenter.canvas.width/mdStream.binding.device_scale_factor-170,y:28}));await pause(100);
  const probe=await page.evaluate(async expected=>{const at=performance.timeOrigin+performance.now();await mdStream.input({kind:'text',text:'a'});const acknowledged=performance.timeOrigin+performance.now(),deadline=at+10000;while(mdPresentation?.step!==expected||!mdPresentation?.presented_ms){if(mdStream.error)throw mdStream.error;if(performance.timeOrigin+performance.now()>deadline)throw Error('MD-DISPLAY typing pixel acknowledgement timeout');await new Promise(r=>requestAnimationFrame(r));}return {started_ms:at,input_ack_ms:acknowledged,drawn_ms:mdPresentation.drawn_ms,presented_ms:mdPresentation.presented_ms,latency_ms:mdPresentation.presented_ms-at};},(i+inputOffset)&31);
  typeProbes.push(probe.latency_ms);(receipt.type_probes??=[]).push(probe);
 }
 receipt.type_condition='settled after motion; does not establish concurrent typing';receipt.type_latency=distribution(typeProbes);receipt.type_cpu=cpuSpan(typeCpu,(await resource()).cpu);
 await page.evaluate(()=>mdStream.stop());
 const endResource=await resource();
 const firstResource=receipt.samples.find(sample=>sample.processes.some(process=>process.pid===kernel.pid));
 if(firstResource){const byPid=new Map(firstResource.processes.map(process=>[process.pid,process.cpu_ticks]));const delta=endResource.processes.reduce((total,process)=>total+Math.max(0,process.cpu_ticks-(byPid.get(process.pid)??process.cpu_ticks)),0);receipt.observed_owned_cpu_percent=delta/100/((endResource.at_ms-firstResource.at_ms)/1000)*100;receipt.cpu_note='live process deltas at Linux CLK_TCK=100; excludes already-exited encoder processes';}
 receipt.latency=distribution(probes);receipt.measurement_duration_ms=performance.now()-start;receipt.measured_local_response_bytes=(await page.evaluate(()=>mdWireBytes))-startBytes;receipt.local_bytes_per_second=receipt.measured_local_response_bytes*1000/receipt.measurement_duration_ms;receipt.frames=await page.evaluate(()=>mdFrames);receipt.client_timings=await page.evaluate(()=>mdTimings);
 receipt.after_input_verification=await page.evaluate(async()=>{const frame=await mdStream.next();return frame?{kind:frame.kind,sequence:frame.sequence}:null});
 const inputVerified=await verifySettled(()=>page.evaluate(()=>mdStream.next()),attempt=>pair('after-input-verification-'+attempt));
 receipt.final_fidelity=inputVerified.fidelity;receipt.final_verification_attempts=inputVerified.verification_attempts;
 if(!receipt.final_fidelity.lossless)throw Error('MD-DISPLAY: small-change pixels differ');
 if(process.env.MD_PROTECTED==='1'){
  if(process.env.MD_PROTECTION_REPETITIONS)receipt.protection_stress=await stressProtection({page,pair,pause,resource,repetitions:Number(process.env.MD_PROTECTION_REPETITIONS)});
  receipt.protected_reference_recovery=await page.evaluate(async()=>{
   const previous=mdStream.presenter.sequence,independent=frame=>frame.kind==='png'||frame.kind==='video'&&frame.key||frame.kind==='stripes'&&frame.stripes.length===8&&frame.stripes.every(row=>row.key);let frame;const deadline=performance.now()+10000;
   // MP-08/MP-10: protocol 475 push asks the pump for a key; credits rewind.
   if(mdStream.push)mdStream.requestKey();else mdStream.presenter.sequence=0;
   while(!(frame=await mdStream.next())||mdStream.push&&!independent(frame)){if(performance.now()>deadline)throw Error('MP-11: protected reference recovery timeout');await new Promise(r=>setTimeout(r,4));}
   return {previous,sequence:frame.sequence,kind:frame.kind,independent:independent(frame),push:mdStream.push===true};
  });
  if(!receipt.protected_reference_recovery.independent)throw Error('MP-11: protected recovery must be independent');
  await pair('protected-reference-recovery');
 }
 // Full navigation preserves the display subscription and rotates its source.
 receipt.fixture_statistics=fixtureStats;
 const oldDocument=await page.evaluate(()=>mdStream.presenter.documentId);
 await page.evaluate(()=>mdStream.input({kind:'click',x:80,y:mdStream.presenter.canvas.height/mdStream.binding.device_scale_factor-25}));await pause(200);
 // MP-08/MP-10: pushed frames of the old document (painted between the
 // native click and the commit) precede it; judge the first new-document frame.
 const navigated=await until(async()=>{const frame=await page.evaluate(()=>mdStream.next());return frame&&frame.document_id!==oldDocument?frame:null},'asynchronous independent navigation frame');
 assertIndependentNavigation(navigated,oldDocument,await page.evaluate(()=>mdStream.binding.codec));
 const oldRejected=await page.evaluate(async document=>{try{await mdTransport.request({KernelBrowser:{command:{op:'display_input',tab_id:mdStream.binding.tab_id,generation:mdStream.binding.generation,document_id:document,input:{kind:'click',x:100,y:200}}}});return false}catch{return true}},oldDocument);
 if(!oldRejected)throw Error('MD-DISPLAY: navigated document accepted stale input');
 const navigation=await verifySettled(()=>page.evaluate(()=>mdStream.next()),attempt=>pair('after-navigation-verification-'+attempt));
 receipt.navigation={independent_kind:navigated.kind,changed_document:true,stale_input_rejected:oldRejected,repair_polls:navigation.polls,verification_attempts:navigation.verification_attempts,fidelity:navigation.fidelity};
 if(!receipt.navigation.fidelity.lossless)throw Error('MD-DISPLAY: navigated repair is not exact');
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
  // MP-11: crash after the real protected encode/presentation flow. Observe
 // kernel reclamation before the drill removes its disposable state parent.
 if(process.env.MD_SUPERVISOR_CRASH==='1')try{
  const packetRoots=(await readdir(shortTmp)).filter(name=>/^chariox-display-[a-f0-9]{32}$/.test(name)).map(name=>path.join(shortTmp,name));
  const browserRoot=path.join(home,'chariox','kernel-browser');
  const profileRoots=(await readdir(browserRoot)).filter(name=>/^[a-f0-9]{64}$/.test(name)).map(name=>path.join(browserRoot,name));
  const poolFiles=[],authFiles=[],profiles=[];let rasterBytes=0;
  for(const directory of [...packetRoots,...profileRoots])for(const name of await readdir(directory)){
   if(/^(encoder|raster)-[A-Za-z0-9]{6}$/.test(name))for(const file of name.startsWith('raster-')?['0','1','2']:['raster']){
    const filename=path.join(directory,name,file);const info=await stat(filename).catch(()=>null);
    if(info){poolFiles.push(filename);rasterBytes+=info.size;}
   }
   if(name==='display.xauth')authFiles.push(path.join(directory,name));
   if(name==='profile')profiles.push(path.join(directory,name));
  }
  if(!rasterBytes||!profiles.length)throw Error('MP-11: abrupt supervisor drill did not create capture rasters and durable profile');
  const supervisors=[];
  for(const name of await readdir('/proc'))if(/^\d+$/.test(name))try{
   const pid=Number(name),command=readFileSync(`/proc/${pid}/cmdline`,'utf8').split('\0');
   if(command.some(arg=>arg.startsWith(root+'/')&&arg.endsWith('/kernel-browser-host.mjs')))supervisors.push(pid);
  }catch{}
  if(supervisors.length!==1||!Number.isSafeInteger(supervisors[0])||supervisors[0]<=1)throw Error('MP-11: unsafe or ambiguous owned supervisor PID');
  process.kill(supervisors[0],'SIGKILL');
  receipt.supervisor_crash={raster_bytes:rasterBytes,packet_roots:packetRoots,pool_files:poolFiles,auth_files:authFiles,durable_profiles:profiles,owned_supervisor_pid:supervisors[0]};
 }catch(error){receipt.cleanup.push(error.message);receipt.status='RED';process.exitCode=1}

 }
 if(!receipt.supervisor_crash)await page.evaluate(()=>mdStream.close());
 await writeFile(path.join(home,'STOP'),'MD-DISPLAY owned stop');
 const exit=await kernelExit;receipt.kernel_exit=exit;if(exit.code!==0)throw Error('MD-DISPLAY kernel drill failed');
 await log.writeTo(output);
 if(errors.length)throw errors[0];
 if(process.env.MD_PROTECTED==='1'){
  receipt.protected_presentations=await page.evaluate(()=>mdProtection);
  for(const failure of receipt.protected_presentations.failures){
   await writeFile(path.join(output,'protected-failure-'+failure.sequence+'.png'),Buffer.from(failure.png.split(',')[1],'base64'));delete failure.png;
  }
  if(!receipt.protected_presentations.frames||receipt.protected_presentations.violations)throw Error('MP-11: an encoded/displayed frame exposed a protected region');
 }
 receipt.status='PASS_LOCAL_COMPONENT';
 if(receipt.latency)receipt.latency_goal={p50_ms:80+receipt.network.rtt,p95_ms:100+receipt.network.rtt,passed:receipt.latency.p50_ms<=80+receipt.network.rtt&&receipt.latency.p95_ms<=100+receipt.network.rtt};
 if(process.env.MD_REQUIRE_LATENCY==='1'&&!receipt.latency_goal.passed)throw Error('MD-DISPLAY: input-to-presentation latency goal remains RED');
} catch(error) {
 receipt.status='RED';receipt.error=String(error.message);process.exitCode=receipt.interrupted?130:1;
 if(browser&&process.env.MD_PROTECTED==='1')try{
  const page=browser.contexts()[0].pages().at(-1);receipt.protected_presentations=await page.evaluate(()=>window.mdProtection);
  for(const failure of receipt.protected_presentations?.failures??[]){await writeFile(path.join(output,'protected-failure-'+failure.sequence+'.png'),Buffer.from(failure.png.split(',')[1],'base64'));delete failure.png;}
 }catch{}
 if(browser)try{const page=browser.contexts()[0].pages().at(-1);receipt.failure_client=await page.evaluate(()=>({frames:window.mdFrames,presentations:window.mdPresentations,stream_running:window.mdStream?.running,stream_error:window.mdStream?.error?.message,sequence:window.mdStream?.presenter?.sequence}));}catch{}
}
finally {
 await stopCpuProfile?.().catch(e=>errors.push(e));
 await collectRelayDiagnostics(page,receipt);
 await stopGroup(kernelProfiler);
 receipt.cpu_samples=cpu.samples;receipt.cpu_accounting='Linux CLK_TCK; separate source Chromium, capture/encode/kernel/relay pipeline, viewer browser and harness. Exited processes retain sampled high-water ticks; sub100ms processes can be missed. WebCodecs inside source Chromium cannot be partitioned (force software portable encoder for CPU comparison).';await cpu.close();
 try{await metrics?.close()}catch{receipt.cleanup.push('RED: owned PNG worker teardown failed');receipt.status='RED';process.exitCode=1}
 if(kernel&&kernel.exitCode===null&&kernel.signalCode===null) {await writeFile(path.join(root,'home','STOP'),'MD-DISPLAY cleanup stop').catch(()=>{});await Promise.race([kernelExit,pause(5000)]);}
 try {await browser?.close();await stopGroup(viewer);await stopGroup(kernel);await stopGroup(display);await pause(500);
  const remaining=[];
  for(const name of await readdir('/proc'))if(/^\d+$/.test(name)){try{const command=await readFile(`/proc/${name}/cmdline`,'utf8');if(command.includes(root)||(shortTmp&&command.includes(shortTmp)))remaining.push(Number(name));}catch{}}
  if(remaining.length)throw Error('MD-DISPLAY: owned-root processes remain: '+remaining.join(','));
  receipt.cleanup.push('owned Chromium/controller/encoder/kernel/Xvfb settled; exact-root process inventory empty');}catch(error){receipt.cleanup.push(error.message);receipt.status='RED';process.exitCode=1;}
 if(receipt.supervisor_crash){
  receipt.supervisor_crash.remaining_packet_roots=[];
  for(const directory of receipt.supervisor_crash.packet_roots)try{await stat(directory);receipt.supervisor_crash.remaining_packet_roots.push(directory)}catch(error){if(error.code!=='ENOENT')throw error}
  receipt.supervisor_crash.remaining_transient_files=[];
  for(const file of [...receipt.supervisor_crash.pool_files,...receipt.supervisor_crash.auth_files])if(await stat(file).catch(()=>null))receipt.supervisor_crash.remaining_transient_files.push(file);
  receipt.supervisor_crash.durable_profiles_retained=true;
  for(const profile of receipt.supervisor_crash.durable_profiles)if(!(await stat(profile).catch(()=>null))?.isDirectory())receipt.supervisor_crash.durable_profiles_retained=false;
  if(!receipt.supervisor_crash.durable_profiles_retained)throw Error('MP-11: supervisor cleanup removed durable browser profile');
  if(receipt.supervisor_crash.remaining_packet_roots.length||receipt.supervisor_crash.remaining_transient_files.length){receipt.cleanup.push('MP-11: supervisor death leaked reusable encoder raster');receipt.status='RED';process.exitCode=1}
 }
 // Read only our non-secret, fixed-label diagnostic files before disposing state.
 const traces=[];receipt.hardware_diagnostics=[];
 async function collectTiming(directory){for(const entry of await readdir(directory,{withFileTypes:true}).catch(()=>[])){const p=path.join(directory,entry.name);if(entry.isDirectory()&&entry.name!=='profile')await collectTiming(p);else if(entry.name==='display-hardware.jsonl'){for(const line of (await readFile(p,'utf8')).trim().split('\n'))if(line){const value=JSON.parse(line);if(typeof value.diagnostic!=='string'||value.diagnostic.length>4096)throw Error('MP-11: hardware diagnostic bound');receipt.hardware_diagnostics.push(value);}}else if(entry.name==='display-timing.jsonl'){const lines=(await readFile(p,'utf8')).trim().split('\n');for(const line of lines)if(line)traces.push(JSON.parse(line));}}}
 try{await collectTiming(path.join(root,'home','chariox'))}
 catch{receipt.diagnostic_error='MP-10: invalid host timing JSON';receipt.status='RED';process.exitCode=1}
 if(process.env.MD_PROFILE==='1')for(const name of await readdir(path.join(root,'profiles')).catch(()=>[])){
  if(/^(CPU\.[\w.-]+\.cpuprofile|(?:encoder|capture)\.\d+\.prof)$/.test(name))await cp(path.join(root,'profiles',name),path.join(output,name));
 }
 receipt.host_timings=traces;receipt.kernel_timings=log.timings();
 receipt.diagnostic_errors=log.diagnosticErrors;
 if(receipt.diagnostic_errors.length){receipt.status='RED';receipt.error='MP-10: invalid kernel timing JSON';process.exitCode=1}
 receipt.actual_encoders=[...new Set(traces.filter(t=>t.stage.startsWith('motion_backend_')).map(t=>t.stage.slice('motion_backend_'.length)))];
 receipt.hardware_fallback=receipt.requested_hardware&&!receipt.actual_encoders.includes('vaapi');
 if(receipt.hardware_fallback){receipt.hardware_warning='MP-10: HARDWARE REQUEST FAILED — successful VAAPI packets not observed; timings describe software fallback';console.error(receipt.hardware_warning);}
 receipt.actual_converters=[...new Set(traces.filter(t=>t.stage.startsWith('motion_converter_')).map(t=>t.stage.slice('motion_converter_'.length)))];
 if(receipt.status==='PASS_LOCAL_COMPONENT'&&process.env.MD_SOFTWARE==='1'&&receipt.codec!=='png'){
  const expected=receipt.codec==='vp8'?'vp8':receipt.codec?.startsWith('vp09')?'vp9':({'libx264':'x264','libopenh264':'openh264'})[receipt.requested_software_encoder];
  if(!receipt.actual_encoders.length||receipt.actual_encoders.some(e=>e!==expected)||receipt.requested_converter==='libyuv'&&(!receipt.actual_converters.length||receipt.actual_converters.some(c=>c!=='libyuv'))){
   receipt.status='RED';receipt.error='MP-10: actual encoder/converter does not match the requested comparison';process.exitCode=1;
  }
 }
 receipt.native_packet_batches=traces.filter(t=>t.stage==='motion_packet_native').length;
 // MP-08/MP-10: headline numbers are native resolution; contention fallback frames are reported separately.
 receipt.reduced_contention_frames=traces.filter(t=>t.stage==='motion_reduced_contention').length;
 receipt.lossless_scroll_frames=traces.filter(t=>t.stage==='native_shift_prepare').length;
 if(receipt.status==='PASS_LOCAL_COMPONENT'&&process.env.MD_REQUIRE_NATIVE_PACKET==='1'&&!receipt.native_packet_batches){receipt.status='RED';receipt.error='MP-10: native stripe bytes did not bypass Node';process.exitCode=1}
 receipt.stage_breakdown=summarizeStages(receipt);
 if(shaped)try{receipt.netem_stats=await shaped.close();receipt.cleanup.push('owned proxy and namespace-local qdisc removed')}catch{receipt.cleanup.push('RED: owned netem cleanup failed');receipt.status='RED';process.exitCode=1}
 if(server)await new Promise(resolve=>server.close(resolve));
 // Delete only the exact freshly-created disposable root, after owned teardown.
 if(!process.exitCode || receipt.cleanup.some(value=>value.includes('inventory empty'))){await rm(root,{recursive:true,force:true});if(shortTmp)await rm(shortTmp,{recursive:true,force:true});receipt.cleanup.push('exact disposable state and owned short temporary directory removed');}else receipt.cleanup.push('uncertain teardown state retained at '+root);
 await log.writeTo(output);
 receipt.finished_at=new Date().toISOString();await writeFile(path.join(output,'results.json'),JSON.stringify(receipt,null,2));console.log(JSON.stringify({item:receipt.item,status:receipt.status,error:receipt.error,latency:receipt.latency,lossless:receipt.settled?.fidelity.lossless}));
}

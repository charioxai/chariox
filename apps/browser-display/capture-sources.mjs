// MD-DISPLAY-02/04: headed compositor-source comparison, not runtime acceptance.
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, writeFile, chmod, chown, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fixture } from './drill-fixtures.mjs';
import { distribution, compare } from './drill-metrics.mjs';
import { launchOwned, stopGroup, checkChild } from './drill-owned-process.mjs';
const [output, tools] = process.argv.slice(2);
if (![output,tools].every(p=>p&&path.isAbsolute(p))) throw Error('MD-DISPLAY: absolute evidence/tools paths required');
const receipt={item:'MD-DISPLAY-02/04',source:execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),status:'RED',methods:[],cleanup:[]};
let root,display,chrome,browser,server;
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function until(f){for(let i=0;i<400;i++){const v=await f();if(v)return v;await pause(25)}throw Error('MD-DISPLAY: launch timeout')}
try {
 await mkdir(output,{recursive:true});
 const require=createRequire(path.join(tools,'package.json')), {chromium}=require('playwright-core'), {PNG}=require('pngjs');
 root=await mkdtemp(path.join(tmpdir(),'chariox-md-display-sources-'));await chmod(root,0o755);
 const home=path.join(root,'profile');await mkdir(home,{mode:0o700});await chown(home,65534,65534);
 display=await launchOwned('/usr/bin/Xvfb',['-displayfd','3','-screen','0','2560x1600x24','-nolisten','tcp','-ac'],{detached:true,stdio:['ignore','ignore','ignore','pipe']});
 let screen='';display.stdio[3].on('data',b=>screen+=b);await until(()=>{checkChild(display);return screen.includes('\n')});
 const text=await readFile(new URL('../../docs/MULTIDOMAIN_KERNEL_BROWSER.md',import.meta.url),'utf8');
 server=createServer((req,res)=>{res.setHeader('Content-Type','text/html');res.end(req.url==='/controller'?'<button id="start">Capture</button>':fixture(req.url,`http://127.0.0.1:${server.address().port}`,text))});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
 chrome=await launchOwned('/usr/bin/google-chrome',['--remote-debugging-port=0',`--user-data-dir=${home}`,'--no-first-run','--disable-background-networking','--auto-select-tab-capture-source-by-title=MD-DISPLAY /canvas','--allow-http-screen-capture','about:blank'],{uid:65534,gid:65534,detached:true,cwd:root,env:{PATH:'/usr/bin:/bin',HOME:home,TMPDIR:home,DISPLAY:`:${screen.trim()}`},stdio:'ignore'});
 const port=await until(async()=>{checkChild(chrome);try{return Number((await readFile(path.join(home,'DevToolsActivePort'),'utf8')).split('\n')[0])}catch{return null}});
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);const context=browser.contexts()[0];const page=await context.newPage();await page.goto(origin+'/canvas');
 const cdp=await context.newCDPSession(page);await cdp.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:2,mobile:false});await page.bringToFront();await pause(200);
 const reference=Buffer.from((await cdp.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false})).data,'base64');await writeFile(path.join(output,'native-reference.png'),reference);
 const times=[],sizes=[];let last,first;
 const onFrame=event=>{const now=performance.now();if(last)times.push(now-last);last=now;sizes.push(Buffer.from(event.data,'base64').length);first??=Buffer.from(event.data,'base64');void cdp.send('Page.screencastFrameAck',{sessionId:event.sessionId}).catch(()=>{});};
 cdp.on('Page.screencastFrame',onFrame);await cdp.send('Page.startScreencast',{format:'png',maxWidth:2560,maxHeight:1600,everyNthFrame:1});await pause(200);
 const still=first?compare(reference,first,PNG):null;await cdp.send('Page.stopScreencast');cdp.off('Page.screencastFrame',onFrame);
 if(first)await writeFile(path.join(output,'screencast-static.png'),first);
 await page.click('#motion');
 for(const format of ['jpeg','png']) {
  times.length=0;sizes.length=0;last=null;first=null;cdp.on('Page.screencastFrame',onFrame);
  const started=performance.now();await cdp.send('Page.startScreencast',{format,quality:95,maxWidth:2560,maxHeight:1600,everyNthFrame:1});await pause(5000);await cdp.send('Page.stopScreencast');cdp.off('Page.screencastFrame',onFrame);
  receipt.methods.push({method:'Page.startScreencast',format,frames:sizes.length,duration_ms:performance.now()-started,fps:sizes.length*1000/(performance.now()-started),cadence:distribution([...times]),encoded_mbps:sizes.reduce((a,b)=>a+b,0)*8/5e6,static_png_fidelity:format==='png'?still:null,geometry:first&&format==='png'?{width:first.readUInt32BE(16),height:first.readUInt32BE(20)}:null});
 }
 try{const frame=await cdp.send('HeadlessExperimental.beginFrame',{screenshot:{format:'png'}});receipt.methods.push({method:'HeadlessExperimental.beginFrame',supported:!!frame.screenshotData})}catch{receipt.methods.push({method:'HeadlessExperimental.beginFrame',supported:false,scope:'headed installed Chrome rejects this headless-only command'})}
 const controller=await context.newPage();await controller.goto(origin+'/controller');
 await controller.evaluate(()=>{document.querySelector('#start').onclick=async()=>{try{window.captureStream=await navigator.mediaDevices.getDisplayMedia({video:{width:2560,height:1600,frameRate:60},audio:false,preferCurrentTab:false});window.captureReady=true}catch(e){window.captureError=e.name}}});
 await controller.click('#start');await until(()=>controller.evaluate(()=>window.captureReady||window.captureError));
 const capture=await controller.evaluate(async()=>{
  if(window.captureError)return {method:'getDisplayMedia + WebCodecs',supported:false,error:window.captureError};
  const track=captureStream.getVideoTracks()[0],settings=track.getSettings();const probes=[];
  for(const codec of ['avc1.420033','vp09.00.40.08','av01.0.08M.08'])for(const hardwareAcceleration of ['prefer-hardware','prefer-software']){
   const config={codec,width:settings.width,height:settings.height,framerate:60,bitrate:8_000_000,bitrateMode:'variable',latencyMode:'realtime',hardwareAcceleration};
   let probe={codec,hardwareAcceleration};
   try{probe.supported=(await VideoEncoder.isConfigSupported(config)).supported;if(!probe.supported){probes.push(probe);continue}
    let count=0,bytes=0,error,started=performance.now();const latency=[];const submitted=new Map();
    const encoder=new VideoEncoder({output:chunk=>{count++;bytes+=chunk.byteLength;latency.push(performance.now()-submitted.get(chunk.timestamp));submitted.delete(chunk.timestamp)},error:e=>{error=e.name}});encoder.configure(config);
    const reader=new MediaStreamTrackProcessor({track}).readable.getReader();
    while(performance.now()-started<3000){const {value:frame,done}=await reader.read();if(done)break;if(encoder.encodeQueueSize<2){submitted.set(frame.timestamp,performance.now());encoder.encode(frame,{keyFrame:count===0})}frame.close();if(error)break}
    await encoder.flush();await reader.cancel();encoder.close();probe={...probe,frames:count,fps:count*1000/(performance.now()-started),mbps:bytes*8/(performance.now()-started)/1000,latency_ms:latency,error};
   }catch(e){probe.error=e.name}probes.push(probe);
  }
  captureStream.getTracks().forEach(t=>t.stop());return {method:'getDisplayMedia + WebCodecs',supported:true,settings,probes};
 });receipt.methods.push(capture);receipt.status='PASS_SOURCE_COMPARISON';
}catch(error){receipt.error=error.message;process.exitCode=1}
finally {
 try{await browser?.close();await stopGroup(chrome);await stopGroup(display);if(root)await rm(root,{recursive:true,force:true});receipt.cleanup.push('owned process groups/private profile removed')}catch(e){receipt.cleanup.push(e.message);receipt.status='RED';process.exitCode=1}
 if(server)await new Promise(r=>server.close(r));await mkdir(output,{recursive:true});await writeFile(path.join(output,'results.json'),JSON.stringify(receipt,null,2));console.log(JSON.stringify({item:receipt.item,status:receipt.status,methods:receipt.methods.map(m=>({method:m.method,format:m.format,fps:m.fps,supported:m.supported}))}));
}

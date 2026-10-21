#!/usr/bin/env node
// MP-08/MP-10/MP-11: actual Cloud View + shared producer network-change drill.
// Authenticate/select the Room with normal product UI in the lane-owned profile.
// The network helper changes the real client link and reports public measurements:
// {wire_rtt_ms,uplink_bits_per_second}. Never use viewer CDP to drive the Room.
// MP-10: pipe the home kernel's stderr into home-request-timing.mjs with this
// drill's evidence root. Enable CHARIOX_BROWSER_DISPLAY_TIMING=1 on that kernel.
// Raw kernel stderr must not be copied into public drill evidence.
import assert from 'node:assert/strict';
import {spawn,execFileSync} from 'node:child_process';
import {mkdir,writeFile,readFile,stat} from 'node:fs/promises';
import {homeRequestTimingSummary} from './home-request-timing.mjs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
const required=name=>{const value=process.env[name];assert(value,`MP-08/MP-10: set ${name}`);return value};
const url=new URL(required('CHARIOX_ROOM_DISPLAY_DRILL_VIEW_URL'));
assert.equal(url.protocol,'https:','MP-10: use the actual hosted View route');
const profile=required('CHARIOX_ROOM_DISPLAY_DRILL_PROFILE');
const evidence=required('CHARIOX_ROOM_DISPLAY_DRILL_EVIDENCE');
const helper=required('CHARIOX_ROOM_DISPLAY_NETWORK_CHANGE_HELPER');
const expectInitialResume=process.env.CHARIOX_ROOM_DISPLAY_EXPECT_INITIAL_RESUME==='1';
for(const value of [profile,evidence,helper])assert(path.isAbsolute(value)&&!value.startsWith(process.cwd()+path.sep),'MP-11: state, helper and evidence must be absolute and outside the checkout');
const {chromium}=await import(pathToFileURL(required('PLAYWRIGHT_MODULE')).href);
const cells=[[1440,900,1],[1440,900,2],[1920,1080,1],[1920,1080,2]];
const report={scope:'MP-08/MP-10/MP-11 Room carry network changes',head:execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),origin:url.origin,path:url.pathname,started:new Date().toISOString(),cells:[],usability_review:'PENDING: watch every recording before acceptance',acceptance:false};
await mkdir(evidence,{recursive:true,mode:0o700});
async function changeNetwork(){
 const child=spawn(helper,[],{stdio:['ignore','pipe','pipe']});
 let stdout='',outputBytes=0,errorBytes=0;
 const timer=setTimeout(()=>{assert(Number.isSafeInteger(child.pid)&&child.pid>1,'MP-11: unsafe owned network helper PID');child.kill('SIGTERM')},30000);
 try{
  child.stdout.on('data',data=>{outputBytes+=data.length;if(outputBytes<=4096)stdout+=data});child.stderr.on('data',data=>{errorBytes+=data.length});
  const code=await new Promise((resolve,reject)=>{child.once('error',reject);child.once('exit',resolve)});
  assert.equal(code,0,`MP-10: network helper failed (stdout=${outputBytes} bytes, stderr=${errorBytes} bytes)`);
  assert(outputBytes<=4096,'MP-11: network helper output bound');
  const measured=JSON.parse(stdout);
  assert(Number.isFinite(measured.wire_rtt_ms)&&measured.wire_rtt_ms>=60,'MP-10: measured wire RTT must be at least 60ms');
  assert(Math.abs(measured.uplink_bits_per_second-8000000)<=500000,'MP-10: shape the actual client uplink to about8Mbit/s');
  return {wire_rtt_ms:measured.wire_rtt_ms,uplink_bits_per_second:measured.uplink_bits_per_second};
 }finally{clearTimeout(timer)}
}
try{
 for(const [width,height,deviceScaleFactor]of cells){
  const name=`${width}x${height}-dpr${deviceScaleFactor}`,directory=path.join(evidence,name);await mkdir(directory,{recursive:true});
  const cell={name,width,height,deviceScaleFactor,status:'FAIL',changes:[],pageErrors:0,consoleErrors:0,closedSockets:0,initialResumeWrites:0};report.cells.push(cell);
  const context=await chromium.launchPersistentContext(profile,{headless:false,executablePath:process.env.VALENV_CHROMIUM??'/opt/google/chrome/chrome',viewport:{width,height},deviceScaleFactor,recordVideo:{dir:directory,size:{width,height}},args:['--no-sandbox',`--force-device-scale-factor=${deviceScaleFactor}`,`--window-size=${width},${height+87}`]});
  try{
   const page=context.pages()[0]??await context.newPage();
   page.on('pageerror',()=>{cell.pageErrors++});page.on('console',message=>{if(message.type()==='error')cell.consoleErrors++});
   page.on('websocket',socket=>{
    socket.on('close',()=>{cell.closedSockets++});
    socket.on('framesent',frame=>{
     // Observe only the envelope kind. Never retain grants, keys or payloads.
     if(typeof frame.payload==='string'){try{if(JSON.parse(frame.payload)?.kind==='client_connect_display_resume')cell.initialResumeWrites++}catch{}}
    });
   });
   await page.goto(url.href,{waitUntil:'domcontentloaded'});
   const canvas=page.locator('[data-view-display-canvas]').first();
   await canvas.waitFor({state:'visible',timeout:30000});
   await page.waitForFunction(()=>document.querySelector('[data-view-display-canvas]')?.dataset.paintMode==='video',{},{timeout:30000});
   // Observe the real viewer compositor. A native-track write/stream marker
   // alone is not evidence that Chromium painted its first resumed frame.
   await page.evaluate(()=>{
    const paints=[],seen=new WeakSet();
    const scan=()=>{for(const video of document.querySelectorAll('video[data-display-sink="track"]')){
     if(seen.has(video)||!video.requestVideoFrameCallback)continue;
     const canvas=document.querySelector('[data-view-display-canvas]');
     if(video.parentElement!==canvas?.parentElement)continue;
     seen.add(video);video.requestVideoFrameCallback(now=>{if(video.isConnected&&canvas.isConnected&&video.parentElement===canvas.parentElement)paints.push({now,stream:canvas.dataset.displayStreamId})});
    }};
    const observer=new MutationObserver(scan);observer.observe(document.body,{childList:true,subtree:true});scan();
    window.__charioxRoomPaintProbe={paints,observer};
   });
   assert.equal(await page.evaluate(()=>devicePixelRatio),deviceScaleFactor);
   await page.screenshot({path:path.join(directory,'before.png')});
   for(let index=0;index<3;index++){
    const previous=await canvas.getAttribute('data-display-stream-id'),closedBefore=cell.closedSockets,initialBefore=cell.initialResumeWrites,started=await page.evaluate(()=>performance.now());
    const measurements=await changeNetwork();
    await page.waitForFunction(old=>{const canvas=document.querySelector('[data-view-display-canvas]');return canvas?.dataset.paintMode==='video'&&canvas.dataset.displayStreamId!==old},previous,{timeout:1000});
    const stream=await canvas.getAttribute('data-display-stream-id');
    const track=await canvas.getAttribute('data-display-sink')==='track';
    let painted_ms;
    if(track){
     await page.waitForFunction(({stream,started})=>window.__charioxRoomPaintProbe.paints.some(paint=>paint.stream===stream&&paint.now>=started),{stream,started},{timeout:1000});
     painted_ms=await page.evaluate(({stream,started})=>window.__charioxRoomPaintProbe.paints.find(paint=>paint.stream===stream&&paint.now>=started).now-started,{stream,started});
    }else{
     // The canvas path commits in rAF; a second rAF bounds the next paint.
     painted_ms=await page.evaluate(started=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(performance.now()-started)))),started);
    }
    const initial_resume_write=cell.initialResumeWrites>initialBefore;
    cell.changes.push({index,painted_ms,paint_observer:track?'requestVideoFrameCallback':'two viewer animation frames',...measurements,socket_closed:cell.closedSockets>closedBefore,initial_resume_write});
    await page.screenshot({path:path.join(directory,`after-${index}.png`)});
    assert(cell.closedSockets>closedBefore,'MP-10: the network change did not retire the actual client socket');
    if(expectInitialResume)assert(initial_resume_write,'MP-10: recovery did not use the negotiated connect+initial535 path');
    assert(painted_ms<=1000,`MP-10: network change to painted frame ${painted_ms.toFixed(1)}ms >1s`);
   }
   assert.equal(cell.pageErrors,0,'MP-10: a real viewer page error is a failure');cell.status='PASS-NETWORK-CELL';
  }finally{await context.close()}
 }
}catch(error){report.failure=error instanceof Error?error.message:'Room display drill failed';process.exitCode=1}
finally{
 try{
  const file=path.join(evidence,'home-request-timing.jsonl');
  assert((await stat(file)).size<=64*1024*1024,'MP-11: home timing evidence exceeds bound');
  report.home_request_timing={file:'home-request-timing.jsonl',...homeRequestTimingSummary(await readFile(file,'utf8'))};
 }catch{
  report.instrumentation_failure='MP-10: missing or invalid home receive/end capture';process.exitCode=1;
 }
 report.finished=new Date().toISOString();await writeFile(path.join(evidence,'report.json'),JSON.stringify(report,null,2)+'\n',{mode:0o600});console.log(`MP-08/MP-10/MP-11 Room network drill evidence: ${evidence}; acceptance requires recording review`)
}

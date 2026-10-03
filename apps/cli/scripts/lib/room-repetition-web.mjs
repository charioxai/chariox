// MP-07/MP-08/MP-10: production View in a disposable browser container.
import assert from 'node:assert/strict'
import {assertStableDecodedViews} from '/room-repetition-gates.mjs'
import {readFile, writeFile} from 'node:fs/promises'
import {createLocalIdleBrowserWebHarness} from '/cloud/scripts/lib/local-idle-browser-web-harness.mjs'
import {chromium} from '/cloud/node_modules/playwright-core/index.mjs'
const config=JSON.parse(await readFile('/runtime/web-config.json','utf8'))
// Keep Chromium's renderer sandbox: the drill runs this container as the image's
// non-root user under the production Chromium seccomp profile.
const browser=await chromium.launch({headless:true,executablePath:'/usr/bin/chromium',chromiumSandbox:true,args:['--disable-dev-shm-usage']})
const context=await browser.newContext({viewport:{width:1440,height:1000}})
const harness=createLocalIdleBrowserWebHarness({context,slowClientMessageDelayMs:0,useCloudApiServer:false,browserSessionToken:'loops-synthetic-session',userId:'local',accountId:'local-dev-account',relayUrl:config.relayUrl,relayToken:'fixture',daemonId:config.daemonId,daemonAlias:config.daemonId,machineId:config.machineId,networkEvents:[],networkRequests:new Map(),websocketFrames:[],transportLogs:[]})
await harness.installLocalWebRoutes()
await context.route('**/browser/relay-kernel/bootstrap',async route=>{
 const response=await fetch(`${config.baseUrl}/loops/bootstrap`,{method:'POST',headers:{'content-type':'application/json'},body:route.request().postData()??'{}'})
 await route.fulfill({status:response.status,contentType:'application/json',body:await response.text()})
})
await context.addInitScript(()=>{
 const Base=window.WebSocket;window.__loopsSocketCloses=[];window.__loopsDisplayStates=[]
 window.WebSocket=class extends Base{constructor(...args){super(...args);this.loopPath=new URL(String(args[0]),location.href).origin+new URL(String(args[0]),location.href).pathname;this.addEventListener('close',event=>window.__loopsSocketCloses.push({at:Date.now(),pathname:this.loopPath,code:event.code,reason:event.reason.slice(0,200),requestedClose:this.loopClose??null}))}close(code,reason){this.loopClose={code:code??null,reason:['invalid display channel','display closed'].includes(reason)?reason:'other'};return super.close(code,reason)}}
 document.addEventListener('DOMContentLoaded',()=>{
  let previous='';const observer=new MutationObserver(()=>{const message=document.querySelector('.view-display-empty:not(.view-display-status-overlay) span')?.textContent??'',state=document.querySelector('.view-connection-state')?.getAttribute('data-state')??'';const next=state+'|'+message;if(next!==previous){window.__loopsDisplayStates.push({at:Date.now(),state,message:message.slice(0,200)});previous=next}});observer.observe(document.documentElement,{subtree:true,childList:true,attributes:true,characterData:true})
 })
})
const pages=[]
const viewSample=page=>page.evaluate(()=>{
 const canvas=document.querySelector('[data-view-display-canvas]');let frameHash=null
 if(canvas){const data=canvas.getContext('2d').getImageData(0,0,canvas.width,canvas.height).data;let hash=2166136261;for(let i=0;i<data.length;i+=4){hash=Math.imul(hash^data[i],16777619);hash=Math.imul(hash^data[i+1],16777619);hash=Math.imul(hash^data[i+2],16777619)}frameHash=hash>>>0}
 return {connected:document.querySelector('.view-connection-state')?.getAttribute('data-state')==='connected',ready:document.querySelector('[data-room-environment-frame-input-ready]')?.getAttribute('data-room-environment-frame-input-ready')==='true',canvasCount:document.querySelectorAll('[data-view-display-canvas]').length,streamId:canvas?.getAttribute('data-view-display-stream-id'),width:canvas?.width,height:canvas?.height,frameHash,displayMessage:document.querySelector('.view-display-empty:not(.view-display-status-overlay) span')?.textContent,socketCloses:window.__loopsSocketCloses,displayStates:window.__loopsDisplayStates}
})
const stable=async requireFrameChange=>{
 const samples=[]
 try{for(let i=0;i<=8;i++){samples.push(await Promise.all(pages.map(viewSample)));if(i)assertStableDecodedViews(samples,false);if(i<8)await new Promise(r=>setTimeout(r,250))};assertStableDecodedViews(samples,requireFrameChange);return samples}
 catch(error){await writeFile('/evidence/web-stability-failure.json',JSON.stringify({mpItems:['MP-08','MP-10'],requireFrameChange,samples,error:error.message},null,2));throw error}
}
const ready=async page=>{
 await page.locator(`[data-view-slice-start-target=${JSON.stringify(config.target)}]`).waitFor({timeout:60000})
 await page.locator("[data-room-environment-status='ready']").waitFor({timeout:60000})
 await page.locator(".view-connection-state[data-state='connected']").waitFor({timeout:60000})
 await page.locator('[data-room-environment-frame-input-ready="true"]').waitFor({timeout:60000})
 assert.equal(await page.locator('[data-view-display-canvas]').count(),1)
 assert.ok((await page.locator('[data-room-environment]').textContent()).includes(config.environmentId))
 const canvas=await page.locator('[data-view-display-canvas]').evaluate(c=>({width:c.width,height:c.height,streamId:c.getAttribute('data-view-display-stream-id')}))
 assert.ok(canvas.streamId&&canvas.width>0&&canvas.height>0)
 return canvas
}
let exit=0
try{
 for(let i=0;i<2;i++) {const page=await context.newPage();pages.push(page);await page.goto(`${config.baseUrl}/slices?view_target=${encodeURIComponent(config.target)}`,{waitUntil:'domcontentloaded'});await ready(page)}
 await stable(false)
 await writeFile('/runtime/web-ready.json',JSON.stringify({mpItems:['MP-07','MP-08','MP-10'],viewers:2,canvases:await Promise.all(pages.map(ready))}))
 let sequence=0
 while(true){
  const command=JSON.parse(await readFile('/runtime/web-command.json','utf8').catch(()=>'{}'))
  if(!command.sequence||command.sequence<=sequence){await new Promise(r=>setTimeout(r,100));continue}
  sequence=command.sequence
  if(command.action==='stop')break
  const started=performance.now()
  if(command.action==='reconnect') {
   // Close every actual Web relay socket, then wait for the product's recovery.
   const closed=await pages[0].evaluate(()=>{const records=window.__charioxDrillKernelSockets?.snapshot()??[]; return records.filter(r=>r.readyState===1).length})
   // A reload disconnects all transports and reconstructs the same Room View.
   assert.ok(closed>0,'Web has no connected transport before disconnect')
   await pages[0].reload({waitUntil:'domcontentloaded'})
  }
  const canvases=await Promise.all(pages.map(ready))
  const stabilitySamples=await stable(command.action==='reconnect')
  await pages[0].screenshot({path:`/evidence/web-${command.action}-${sequence}.png`})
  await writeFile('/runtime/web-result.json',JSON.stringify({mpItems:['MP-07','MP-08','MP-10'],sequence,ok:true,elapsedMs:performance.now()-started,viewers:2,canvases,stabilitySamples}))
 }
}catch(error){exit=1;await writeFile('/runtime/web-error.json',JSON.stringify({mpItems:['MP-07','MP-08','MP-10'],message:error.message,stack:error.stack}));for(let i=0;i<pages.length;i++){await writeFile(`/evidence/web-failure-diagnostic-${i}.json`,JSON.stringify({mpItems:['MP-08','MP-10'],...(await pages[i].evaluate(()=>({pathname:location.pathname,environmentStatus:document.querySelector('[data-room-environment-status]')?.getAttribute('data-room-environment-status'),connectionState:document.querySelector('.view-connection-state')?.getAttribute('data-state'),displayMessage:document.querySelector('.view-display-empty:not(.view-display-status-overlay) span')?.textContent,canvasCount:document.querySelectorAll('[data-view-display-canvas]').length,socketCloses:window.__loopsSocketCloses,displayStates:window.__loopsDisplayStates})).catch(()=>({unavailable:true})))}));await pages[i].screenshot({path:`/evidence/web-failure-${i}.png`}).catch(()=>{})}}
finally{await context.close().catch(()=>{});await browser.close().catch(()=>{})}
process.exitCode=exit

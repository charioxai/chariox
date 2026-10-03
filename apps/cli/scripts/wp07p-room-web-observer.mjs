// MP-08/MP-10: actual compiled Cloud View, one decoded stream, no hosted contact.
import assert from 'node:assert/strict';
import {readFile,writeFile} from 'node:fs/promises';
import {createLocalIdleBrowserWebHarness} from '/cloud/scripts/lib/local-idle-browser-web-harness.mjs';
import {chromium} from '/cloud/node_modules/playwright-core/index.mjs';
const mp_items=['MP-08','MP-10'], config=JSON.parse(await readFile('/runtime/web-config.json','utf8'));
const browser=await chromium.launch({headless:true,executablePath:'/usr/bin/chromium',args:['--no-sandbox','--disable-dev-shm-usage']});
const context=await browser.newContext({viewport:{width:1440,height:1000}}), page=await context.newPage();
const harness=createLocalIdleBrowserWebHarness({context,slowClientMessageDelayMs:0,useCloudApiServer:false,browserSessionToken:'wp07-loopback-fixture',userId:'local',accountId:'local-dev-account',relayUrl:config.relayUrl,relayToken:'fixture',daemonId:config.daemonId,daemonAlias:config.daemonId,machineId:config.machineId,networkEvents:[],networkRequests:new Map(),websocketFrames:[],transportLogs:[]});
await harness.installLocalWebRoutes();
await context.route('**/browser/relay-kernel/bootstrap',async route=>{const r=await fetch(config.baseUrl+'/wp07/bootstrap',{method:'POST',headers:{'content-type':'application/json'},body:route.request().postData()??'{}'});await route.fulfill({status:r.status,contentType:'application/json',body:await r.text()})});
const sample=()=>page.evaluate(()=>{const room=document.querySelector('[data-room-environment]'), c=document.querySelector('[data-view-display-canvas]');let frameHash=null;if(c){const d=c.getContext('2d').getImageData(0,0,c.width,c.height).data;let h=2166136261;for(let i=0;i<d.length;i+=4)h=Math.imul(h^d[i],16777619);frameHash=h>>>0}return {environmentId:room?.getAttribute('data-room-environment-id'),generation:room?.getAttribute('data-room-environment-runtime-generation'),tabId:room?.getAttribute('data-room-environment-focused-tab-id'),connected:document.querySelector('.view-connection-state')?.getAttribute('data-state')==='connected',ready:document.querySelector('[data-room-environment-frame-input-ready]')?.getAttribute('data-room-environment-frame-input-ready')==='true',canvasCount:document.querySelectorAll('[data-view-display-canvas]').length,width:c?.width,height:c?.height,streamId:c?.getAttribute('data-view-display-stream-id'),frameHash,displayMessage:document.querySelector('.view-display-empty:not(.view-display-status-overlay) span')?.textContent,mode:document.querySelector('[data-room-environment-display-mode]')?.getAttribute('data-room-environment-display-mode')}});
// MP-08/MP-10: runtime interactions and prompt input belong to the terminal route.
let terminal;
async function terminalPage(){
 if(terminal)return terminal;
 const [sessionId,agentId]=config.target.split(':');
 terminal=page;
 await terminal.goto(config.baseUrl+'/waiting-room',{waitUntil:'domcontentloaded'});
 const session=terminal.locator('[data-waiting-session-id='+JSON.stringify(sessionId)+']').first();
 await session.waitFor({timeout:45000});
 await terminal.locator('[data-waiting-row-key="join"]').first().click();
 const picker=terminal.locator('[data-session-picker-session-id='+JSON.stringify(sessionId)+']').first();
 await picker.waitFor({timeout:30000});
 for(let attempt=0;attempt<3;attempt++){
  await picker.locator('.session-table-project-cell').first().click();
  try{await terminal.waitForURL('**/terminal**',{timeout:5000});break}catch(error){if(attempt===2)throw error;await picker.waitFor({timeout:10000})}
 }
 await terminal.locator('.freeform-workspace').waitFor({timeout:45000});
 const pane=terminal.locator('.freeform-agent-pane[data-agent-id='+JSON.stringify(agentId)+']');
 await pane.waitFor({timeout:15000});await pane.click();
 await terminal.locator('.freeform-agent-pane.focused[data-agent-id='+JSON.stringify(agentId)+']').waitFor({timeout:15000});
 return terminal;
}
let exit=0;
try{
 await page.goto(config.baseUrl+'/slices?view_target='+encodeURIComponent(config.target),{waitUntil:'domcontentloaded'});
 await page.locator('[data-room-environment-status="ready"]').waitFor({timeout:60000});
 await page.locator('[data-room-environment-frame-input-ready="true"]').waitFor({timeout:60000});
 await page.locator('[data-room-environment-view="computer"]').click();
 const ready=await sample();assert.equal(ready.environmentId,config.environmentId);assert.equal(ready.canvasCount,1);assert.ok(ready.frameHash&&ready.streamId&&ready.width>0);
 await writeFile('/runtime/web-ready.json',JSON.stringify({mp_items,sample:ready}));
 let sequence=0;
 while(true){const cmd=JSON.parse(await readFile('/runtime/web-command.json','utf8').catch(()=>'{}'));if(!cmd.sequence||cmd.sequence<=sequence){await new Promise(r=>setTimeout(r,100));continue}sequence=cmd.sequence;if(cmd.action==='stop')break;
  if(cmd.action.startsWith('permission-')){const id=cmd.action.slice(11);await (await terminalPage()).locator('[data-interaction-id='+JSON.stringify(id)+']').waitFor({timeout:15000})}
  if(cmd.action.startsWith('approve-')){const id=cmd.action.slice(8);await (await terminalPage()).locator('[data-terminal-interaction-id='+JSON.stringify(id)+'][data-terminal-interaction-choice="allow_once"]').click()}
  if(cmd.action==='web-attachment'){const t=await terminalPage();await t.locator('[data-terminal-attachment-input]').setInputFiles('/runtime/provider-workspace/wp07p-web-attachment.txt');await t.locator('[data-terminal-attachment-remove]').waitFor({timeout:15000});const composer=t.locator('[data-terminal-prompt]').first();await composer.fill('Read the attached text file. Reply exactly its single line. No other work.');await composer.press('Enter')}
  if(cmd.action==='web-prompt'){const t=await terminalPage();const composer=t.locator('[data-terminal-prompt]').first();await composer.fill('MP-08/MP-10: Reply exactly WP07P_WEB_OK. No tools.');await composer.press('Enter')}
  const terminalCommand=/^(permission-|approve-|web-prompt|web-attachment)/.test(cmd.action);
  if(cmd.action==='return-view'||cmd.action==='return-view-takeover'){terminal=null;await page.goto(config.baseUrl+'/slices?view_target='+encodeURIComponent(config.target),{waitUntil:'domcontentloaded'});await page.locator('[data-room-environment-frame-input-ready="true"]').waitFor({timeout:45000});await page.locator('[data-room-environment-view="computer"]').click()}
  if(cmd.action==='takeover'||cmd.action==='return-view-takeover') {await page.locator('[data-room-environment-takeover]').click();await page.locator('[data-room-environment-release]').waitFor({timeout:10000});await page.locator('[data-room-environment-frame-input-ready="true"]').waitFor({timeout:15000});const canvas=page.locator('[data-view-display-canvas]'),box=await canvas.boundingBox();assert.ok(box);const input=page.locator('[data-room-environment-pointers][data-room-environment-input-enabled="true"]');const inputBox=await input.boundingBox();assert.ok(inputBox);await input.click({position:{x:box.x+box.width/2-inputBox.x,y:box.y+box.height/2-inputBox.y}});await new Promise(r=>setTimeout(r,500));await page.locator('[data-room-environment-release]').click()}
  if(!terminalCommand)await page.locator('[data-room-environment-frame-input-ready="true"]').waitFor({timeout:15000});
  const samples=[];for(let i=0;i<4;i++){const s=await sample();samples.push(s);assert.ok(terminalCommand||(s.connected&&s.ready&&s.canvasCount===1&&s.frameHash&&s.environmentId===config.environmentId),'Web decoded stream unavailable or Room changed');await new Promise(r=>setTimeout(r,250))}
  await (terminalCommand?terminal:page).screenshot({path:'/evidence/web-'+cmd.action+'-'+sequence+'.png'});if(!terminalCommand){const decoded=await page.locator('[data-view-display-canvas]').evaluate(c=>c.toDataURL('image/png'));await writeFile('/evidence/decoded-'+cmd.action+'-'+sequence+'.png',Buffer.from(decoded.split(',')[1],'base64'));}
  const report={mp_items,sequence,ok:true,samples,source:terminalCommand?'production Cloud C terminal prompt/interaction':'production Cloud C View decoded canvas'};await writeFile('/evidence/web-'+cmd.action+'-'+sequence+'.json',JSON.stringify(report,null,2));await writeFile('/runtime/web-result.json',JSON.stringify(report));
 }
}catch(error){exit=1;const result={mp_items,error:error.message,terminalRoute:terminal?.url()?.split('?')[0],sample:await sample().catch(()=>null)};await writeFile('/runtime/web-error.json',JSON.stringify(result));await writeFile('/evidence/web-error.json',JSON.stringify(result,null,2));await (terminal??page).screenshot({path:'/evidence/web-error.png'}).catch(()=>{})}
finally{await context.close();await browser.close()}
process.exitCode=exit;

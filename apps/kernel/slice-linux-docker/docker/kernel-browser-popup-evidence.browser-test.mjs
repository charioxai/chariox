// MP-08/MP-10/MP-11: real Chromium regression, supplementary to live acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {createServer} from 'node:http';
import {spawn} from 'node:child_process';
import {connectCdpPipe} from './kernel-browser-cdp-pipe.mjs';
import {BrowserCdpClient} from './browser-controller-cdp.mjs';
async function launchChromium({executable,root,dpr}) {
 const child=spawn(executable,[`--user-data-dir=${root}/chrome`,'--remote-debugging-pipe','--headless=new','--no-sandbox','--disable-background-networking',`--force-device-scale-factor=${dpr}`],{stdio:['ignore','ignore','ignore','pipe','pipe']});
 const connection=connectCdpPipe(child.stdio[3],child.stdio[4],10000),browser=new BrowserCdpClient({connectionFactory:()=>connection});await browser.ensureConnection();
 return {connection,browser,async close(){await connection.send('Browser.close').catch(()=>{});for(let i=0;i<50&&child.exitCode===null&&child.signalCode===null;i++)await new Promise(r=>setTimeout(r,20));if(child.exitCode===null&&child.signalCode===null&&Number.isSafeInteger(child.pid)&&child.pid>1)child.kill('SIGKILL')}};
}
import {BrowserPopupEvidence} from './kernel-browser-popup-evidence.mjs';
for(const framed of [false,true])for(const dpr of [1,2])test(`MP-08 DPR${dpr} ${framed?'child frame':'top frame'}: delayed popup retains activation evidence and subsequent native input clears it`,async()=>{
 const root=await mkdtemp(path.join(tmpdir(),'displayopus-popup-'));
 const server=createServer((req,res)=>{res.setHeader('content-type','text/html');res.end(framed&&req.url==='/'?'<iframe src="/frame" style="position:absolute;left:0;top:0;width:400px;height:200px;border:0"></iframe>':'<button style="position:absolute;left:0;top:0;width:200px;height:100px" onclick="setTimeout(()=>window.open(\'/popup\'),150)">Open asynchronously</button>')});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const evidence=new BrowserPopupEvidence();let chrome;
 try{
  chrome=await launchChromium({executable:process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE,root,dpr});const {browser,connection}=chrome;
  const {targetId}=await connection.send('Target.createTarget',{url:`http://127.0.0.1:${server.address().port}/`});const sessionId=await browser.ensureTargetSession(connection,targetId);
  for(let i=0;i<100;i++){const {result}=await connection.send('Runtime.evaluate',{expression:'document.readyState==="complete"&&!!document.querySelector("button")',returnByValue:true},sessionId);if(result.value)break;await new Promise(r=>setTimeout(r,20))}
  const click=async()=>{await connection.send('Input.dispatchMouseEvent',{type:'mousePressed',x:60,y:40,button:'left',clickCount:1},sessionId);await connection.send('Input.dispatchMouseEvent',{type:'mouseReleased',x:60,y:40,button:'left',clickCount:1},sessionId)};
  await evidence.capture(browser,{target_id:targetId},'agent-delayed',async dispatch=>{const end=dispatch();try{await click()}finally{end()}});
  await new Promise(r=>setTimeout(r,400));const first=(await connection.send('Target.getTargets')).targetInfos.filter(t=>t.openerId===targetId);assert.equal(first.length,1);
  assert.deepEqual(evidence.inventory(first.map(t=>({target_id:t.targetId,tab_id:t.targetId}))),{[first[0].targetId]:'agent-delayed'});
  // A trusted input outside admitted dispatch is a native user's gesture.
  await connection.send('Page.bringToFront',{},sessionId);await click();await new Promise(r=>setTimeout(r,400));const all=(await connection.send('Target.getTargets')).targetInfos.filter(t=>t.openerId===targetId);assert.equal(all.length,2);
  assert.deepEqual(evidence.inventory(all.map(t=>({target_id:t.targetId,tab_id:t.targetId}))),{[first[0].targetId]:'agent-delayed'});
 }finally{evidence.clear();await chrome?.close();await new Promise(resolve=>server.close(resolve));await rm(root,{recursive:true,force:true})}
});

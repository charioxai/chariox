// MP-08/MP-10/MP-11: real Chromium regression, supplementary to live acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { launchChromium } from './browser-protection-fixture.mjs';
import { measureBrowserProtection } from './browser-protection-regions.mjs';
import { captureProtectedPage, decodePng } from './kernel-browser-pixels.mjs';
const value = 'MP11-disposable-fill-value';
const hash = v => createHash('sha256').update(v).digest('hex');
const html = '<!doctype html><body style="margin:0;background:white"><input id=plain style="position:absolute;left:80px;top:70px;width:220px;height:40px;background:magenta;border:0"><input id=password type=password style="position:absolute;left:80px;top:150px;width:220px;height:40px;background:cyan;border:0"><textarea id=area style="position:absolute;left:80px;top:230px"></textarea><div id=editor contenteditable style="position:absolute;left:80px;top:300px;width:220px;height:40px"></div><p>MP11-disposable-fill-value</p><canvas width=200 height=80></canvas>';
async function setup(dpr, run) {
  const root = await mkdtemp(path.join(tmpdir(), 'protxform-fill-'));
  const server = createServer((req,res) => {res.setHeader('content-type','text/html');res.end(req.url==='/frame'?html:html+`<iframe src="http://localhost:${server.address().port}/frame" style="position:absolute;left:500px;top:120px;width:400px;height:430px;border:0"></iframe>`);});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  let chrome;
  try {
    chrome = await launchChromium({executable:process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE,root,dpr});
    const {browser,connection}=chrome, {targetInfos}=await connection.send('Target.getTargets');
    const targetId=targetInfos.find(t=>t.type==='page').targetId, {sessionId}=await browser.resolvePageTarget(targetId);
    await connection.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:dpr,mobile:false},sessionId);
    const url=`http://127.0.0.1:${server.address().port}/`;
    await connection.send('Page.navigate',{url},sessionId);
    for(let i=0;i<100;i++) {const {result}=await connection.send('Runtime.evaluate',{expression:'document.readyState',returnByValue:true},sessionId);if(result.value==='complete')break;await new Promise(r=>setTimeout(r,30));}
    const documentId=(await connection.send('Page.getFrameTree',{},sessionId)).frameTree.frame.loaderId;
    const evaluate=expression=>connection.send('Runtime.evaluate',{expression,returnByValue:true},sessionId);
    const ref=async selector=>{const {root}=await connection.send('DOM.getDocument',{},sessionId);const {nodeId}=await connection.send('DOM.querySelector',{nodeId:root.nodeId,selector},sessionId);return `backend:${(await connection.send('DOM.describeNode',{nodeId},sessionId)).node.backendNodeId}`;};
    const policy={unknown:false,values:[value],targets:[]};
    const fill=async(selector)=>{const node_ref=await ref(selector);const result=await browser.performAction({target_id:targetId,document_id:documentId,node_ref,action:{kind:'fill',text:value,expected_document_url:url}});policy.targets.push({kind:'browser',target_id:targetId,document_id:documentId,node_ref,value_hash:hash(value)});return result;};
    const collect=async()=> (await measureBrowserProtection(browser,policy)).pages[0].regions;
    const capture=async label=>{const data=await captureProtectedPage(browser,{target_id:targetId,document_id:documentId},policy.values,policy.targets,async()=>(await connection.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false},sessionId)).data,dpr);if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`fill-dpr${dpr}-${label}.png`),Buffer.from(data,'base64'));return decodePng(data,dpr);};
    await run({browser,connection,sessionId,targetId,documentId,evaluate,ref,policy,fill,collect,capture,url,dpr});
  } finally {await chrome?.close();server.closeAllConnections();await new Promise(r=>server.close(r));await rm(root,{recursive:true,force:true});}
}
for(const dpr of [1,2]) {
 test(`MP-08/MP-11 DPR${dpr}: only Vault filled plain fields, password toggle, clear and navigation`,()=>setup(dpr,async({fill,collect,capture,evaluate,policy,connection,sessionId})=>{
   assert.deepEqual(await collect(),[],'registration alone never masks password, media, frames or echoes');
   await fill('#plain');
   let boxes=await collect();assert.equal(boxes.length,1);
   const frame=await capture('plain');const pixel=(x,y)=>[...frame.pixels.subarray(((y*dpr)*frame.width+x*dpr)*4,((y*dpr)*frame.width+x*dpr)*4+3)];
   assert.deepEqual(pixel(100,90),[0,0,0]);assert.deepEqual(pixel(100,170),[0,255,255]);assert.deepEqual(pixel(400,400),[255,255,255]);
   await fill('#password');assert.equal((await collect()).length,1,'password dots stay visible');
   await evaluate("document.querySelector('#password').type='text'");assert.equal((await collect()).length,2,'show password checked on every capture');await capture('toggle');
   await evaluate("document.querySelector('#plain').value='user replacement'");assert.equal((await collect()).length,1);
   await evaluate("document.querySelector('#plain').value='MP11-disposable-fill-value'");assert.equal((await collect()).length,1,'retired target never resurrects');
   await evaluate("document.querySelector('#password').value=''");assert.deepEqual(await collect(),[]);
   await fill('#area');await fill('#editor');assert.equal((await collect()).length,2,'textarea and contenteditable are fill targets');
   await evaluate("document.querySelector('#area').remove();document.querySelector('#editor').remove()");assert.deepEqual(await collect(),[]);
   await connection.send('Page.navigate',{url:'about:blank'},sessionId);assert.deepEqual(await collect(),[],'navigation retires targets');
 }));
 test(`MP-08/MP-11 DPR${dpr}: Vault fill in cross-origin renderer masks its own field`,()=>setup(dpr,async({browser,connection,sessionId,targetId,documentId,policy,collect,capture,url})=>{
   const child=(await connection.send('Target.getTargets')).targetInfos.find(t=>t.type==='iframe');assert(child);
   const {sessionId:cs}=await connection.send('Target.attachToTarget',{targetId:child.targetId,flatten:true});
   let node_ref;
   try {const {frameTree}=await connection.send('Page.getFrameTree',{},cs),{root}=await connection.send('DOM.getDocument',{},cs),{nodeId}=await connection.send('DOM.querySelector',{nodeId:root.nodeId,selector:'#plain'},cs);const {node}=await connection.send('DOM.describeNode',{nodeId},cs);node_ref=`frame:${frameTree.frame.id}:${frameTree.frame.loaderId}:backend:${node.backendNodeId}`;}
   finally {await connection.send('Target.detachFromTarget',{sessionId:cs});}
   await browser.performAction({target_id:targetId,document_id:documentId,node_ref,action:{kind:'fill',text:value,expected_document_url:url.replace('127.0.0.1','localhost')+'frame'}});
   policy.targets.push({kind:'browser',target_id:targetId,document_id:documentId,node_ref,value_hash:hash(value)});
   const boxes=await collect();assert.equal(boxes.length,1);assert(boxes[0][0]>500*dpr);const frame=await capture('cross-origin');const i=((210*dpr)*frame.width+600*dpr)*4;assert.deepEqual([...frame.pixels.subarray(i,i+3)],[0,0,0]);
 }));
}

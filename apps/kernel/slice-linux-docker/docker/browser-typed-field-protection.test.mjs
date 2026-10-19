// MP-08/MP-10/MP-11: physical browser regression, never live acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {launchChromium} from './browser-protection-fixture.mjs';
import {measurePageProtection} from './browser-protection-regions.mjs';
import {captureBrowserSnapshot} from './browser-controller-snapshot.mjs';
import {captureProtectedPage,decodePng} from './kernel-browser-pixels.mjs';
import {typedBrowserTargets} from './browser-typed-field-protection.mjs';
test('MP-11 password autocomplete survives a Show password type change',()=>{
 const strings=['input','type','text','autocomplete','current-password','section-register new-password','frame'];
 const snapshot={strings,documents:[{frameId:6,nodes:{nodeName:[0,0],attributes:[[1,2,3,4],[1,2,3,5]],backendNodeId:[11,12]}}]};
 assert.deepEqual(typedBrowserTargets(snapshot,{prefix:''},'target','doc').map(target=>target.node_ref),['backend:11','backend:12']);
});
test('MP-11 CDP empty attribute sentinels preserve sensitive field discovery',()=>{
 const strings=['input','required','autofocus','type','password','autocomplete','cc-number','frame'];
 const snapshot={strings,documents:[{frameId:7,nodes:{nodeName:[0,0,0],attributes:[[1,-1,2,-1,3,4],[3,-1,5,6],[5,-1]],backendNodeId:[11,12,13]}}]};
 assert.deepEqual(typedBrowserTargets(snapshot,{prefix:''},'target','doc').map(target=>target.node_ref),['backend:11','backend:12']);
});
const executable=process.env.CHARIOX_PROTECTION_TEST_CHROME;
for(const dpr of [1,2])test(`MP-11 typed sensitive fields mask model captures only at DPR ${dpr}`,{skip:!executable,timeout:30000},async()=>{
 const root=await mkdtemp(path.join(os.tmpdir(),'typed-fields-'));let chrome;
 try{
  chrome=await launchChromium({executable,dpr,root});const {connection,browser}=chrome;
  const {targetInfos}=await connection.send('Target.getTargets'),target=targetInfos.find(t=>t.type==='page');
  const {sessionId}=await connection.send('Target.attachToTarget',{targetId:target.targetId,flatten:true});
  const html='<style>input{display:block;background:magenta;width:240px;height:32px;margin:12px;color:black}</style><input aria-label="Card" autocomplete="cc-number" required><input aria-label="OTP" autocomplete="one-time-code"><input aria-label="Password" type="password" required autofocus><input autocomplete="cc-exp"><input id="ordinary"><textarea aria-label="Textarea OTP" autocomplete="one-time-code"></textarea><div id="shadow"></div><iframe srcdoc="&lt;input autocomplete=cc-csc style=background:magenta;width:160px;height:32px&gt;"></iframe>';
  await connection.send('Page.navigate',{url:'data:text/html,'+encodeURIComponent(html)},sessionId);
  for(let n=0;n<100;n++){const {result}=await connection.send('Runtime.evaluate',{expression:'document.readyState==="complete"',returnByValue:true},sessionId);if(result.value)break;await new Promise(r=>setTimeout(r,20));}
  // Private setup only; the actual product input path is exercised in live drills.
  await connection.send('Runtime.evaluate',{expression:`document.querySelectorAll('input,textarea').forEach((f,i)=>{if(i!==3)f.value='public-test-'+i});document.querySelector('#shadow').attachShadow({mode:'open'}).innerHTML='<input autocomplete="cc-name" value="public-test-shadow" style="background:magenta;width:160px;height:32px">';document.querySelector('iframe').contentDocument.querySelector('input').value='123'`},sessionId);
  const policy={unknown:false,values:[],targets:[]};
  const owner=await measurePageProtection(connection,sessionId,target.targetId,policy,{includeHidden:true});
  assert.equal(owner.regions.length,0,'owner has no typed-field masks');
  const model=await measurePageProtection(connection,sessionId,target.targetId,policy,{includeHidden:true,modelCapture:true});
  assert.equal(model.regions.length,6,'password, card, OTP, shadow and child-frame values all masked; empty and ordinary fields readable');
  await connection.send('Runtime.evaluate',{expression:"const password=document.querySelector('[type=password]');password.autocomplete='current-password';password.type='text'"},sessionId);
  assert.equal((await measurePageProtection(connection,sessionId,target.targetId,policy,{includeHidden:true,modelCapture:true})).regions.length,6,'Show password keeps semantic password protection');
  assert.equal((await measurePageProtection(connection,sessionId,target.targetId,policy,{includeHidden:true})).regions.length,0,'owner can see the revealed password field');
  const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
  const text=await captureBrowserSnapshot({connection,sessionId,targetId:target.targetId,documentId:frameTree.frame.loaderId,browserGeneration:1,snapshotRevision:1});
  for(const label of ['Card','OTP','Textarea OTP','Password'])assert.equal(text.accessibility_nodes.find(node=>node.name===label)?.value,'[redacted]','sensitive field AX value is withheld');
  const shoot=async()=>(await connection.send('Page.captureScreenshot',{format:'png'},sessionId)).data;
  const raw=await shoot(),masked=await captureProtectedPage(browser,{target_id:target.targetId},[],[],shoot,dpr,null,()=>{},null,true);
  assert.notEqual(raw,masked,'model raster differs from owner raster');
  const pixels=decodePng(masked,dpr,{width:model.viewport[0],height:model.viewport[1]});
  for(const [x,y,w,h] of model.regions){const offset=(Math.floor(y+h/2)*pixels.width+Math.floor(x+w/2))*4;assert.deepEqual([...pixels.pixels.subarray(offset,offset+3)],[0,0,0]);}
  await connection.send('Runtime.evaluate',{expression:"document.querySelectorAll('input,textarea').forEach(f=>f.value='');document.querySelector('#shadow').shadowRoot.querySelector('input').value='';document.querySelector('iframe').contentDocument.querySelector('input').value=''"},sessionId);
  assert.equal((await measurePageProtection(connection,sessionId,target.targetId,policy,{includeHidden:true,modelCapture:true})).regions.length,0,'cleared values retire masks');
 }finally{await chrome?.close();await rm(root,{recursive:true,force:true});}
});

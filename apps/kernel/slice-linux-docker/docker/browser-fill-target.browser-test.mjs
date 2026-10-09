// MP-08/MP-10/MP-11: real Chromium regression, supplementary to live acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { PortableEncoder } from './kernel-browser-display.mjs';
import { launchChromium } from './browser-protection-fixture.mjs';
import { measureBrowserProtection } from './browser-protection-regions.mjs';
import { captureProtectedPage, decodePng } from './kernel-browser-pixels.mjs';
import { MirrorService } from './kernel-browser-mirror.mjs';
import { mirrorHash } from './kernel-browser-mirror-resources.mjs';
async function videoPixels(png,dpr,label) {
  if(process.env.CHARIOX_FILL_VIDEO!=='1')return null;
  const encoder=new PortableEncoder();
  try {
    const packet=await encoder.encode(png,8000000,true,'avc1.420033');
    if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`fill-dpr${dpr}-${label}.h264`),Buffer.from(packet.data_base64,'base64'));
    return await new Promise((resolve,reject)=>{
      const child=execFile(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON??'python3',['-c',"import av,sys; frame=av.CodecContext.create('h264','r').decode(av.Packet(sys.stdin.buffer.read()))[0];sys.stdout.buffer.write(frame.to_ndarray(format='rgba').tobytes())"],{encoding:'buffer',maxBuffer:2560*1600*4+4096,timeout:10000},(error,stdout)=>error?reject(Error('MP-11 video decode failed')):resolve(stdout));
      child.stdin.end(Buffer.from(packet.data_base64,'base64'));
    });
  }finally{await encoder.close();}
}
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
    const capture=async label=>{const data=await captureProtectedPage(browser,{target_id:targetId,document_id:documentId},policy.values,policy.targets,async()=>(await connection.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false},sessionId)).data,dpr);if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`fill-dpr${dpr}-${label}.png`),Buffer.from(data,'base64'));const decoded=decodePng(data,dpr);decoded.video=await videoPixels(data,dpr,label);return decoded;};
    await run({browser,connection,sessionId,targetId,documentId,evaluate,ref,policy,fill,collect,capture,url,dpr});
  } finally {await chrome?.close();server.closeAllConnections();await new Promise(r=>server.close(r));await rm(root,{recursive:true,force:true});}
}
for(const dpr of [1,2]) {
 test(`MP-08/MP-11 DPR${dpr}: image artifacts report actual plain-field redaction`,()=>setup(dpr,async({browser,connection,fill,evaluate,targetId,documentId})=>{
   connection.browserInstanceId='MP11-public-artifact-fixture';
   const request={target_id:targetId,document_id:documentId,browser_generation:browser.browserGeneration,kind:'image',guid:null,viewport:{css_width:1280,css_height:800,device_scale_factor:dpr,desktop_pixel_width:1280*dpr,desktop_pixel_height:800*dpr,revision:1,last_actor_id:null}};
   await browser.reconcile(request.viewport,{browserBarVisible:false});
   await fill('#password');
   let image=await browser.captureArtifact(request);
   assert.equal(image.redaction,'none','MP-11 password dots have no pixel redaction');
   if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`artifact-dpr${dpr}-password.json`),JSON.stringify(image));
   await fill('#plain');image=await browser.captureArtifact(request);
   assert.equal(image.redaction,'fill_targets');
   if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`artifact-dpr${dpr}-plain.json`),JSON.stringify(image));
   const frame=decodePng(image.data_base64,dpr),index=((90*dpr)*frame.width+100*dpr)*4;
   assert.deepEqual([...frame.pixels.subarray(index,index+3)],[0,0,0]);
   await evaluate("document.querySelector('#plain').value=''");
   image=await browser.captureArtifact(request);assert.equal(image.redaction,'none','MP-11 retired/plain and password targets do not claim redaction');
   if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`artifact-dpr${dpr}-retired.json`),JSON.stringify(image));
 }));
 test(`MP-08/MP-11 DPR${dpr}: mirror packets mask the exact top and same-origin frame fields`,()=>setup(dpr,async({browser,connection,sessionId,targetId,documentId,evaluate,fill,policy,url})=>{
   await evaluate(`document.querySelector('iframe').src=${JSON.stringify(url+'frame')};document.querySelector('iframe').style.left='700px';document.querySelector('iframe').style.top='350px'`);
   for(let i=0;i<100;i++){const {result}=await evaluate("!!document.querySelector('iframe').contentDocument?.querySelector('#plain')");if(result.value)break;await new Promise(r=>setTimeout(r,30));}
   await evaluate("document.querySelector('#plain').style.left='550px';document.querySelector('#plain').style.top='160px'");
   await evaluate("document.querySelector('#password').style.appearance='none'");
   const tab={tab_id:'t',target_id:targetId,document_id:documentId};
   const host={generation:1,scales:new Map(),protection:policy,browser,async target(){return tab},async displayTarget(){return tab},async screenshot(_tab,clip){return {data_base64:await captureProtectedPage(browser,tab,policy.values,policy.targets,async()=>(await connection.send('Page.captureScreenshot',{format:'png',...(clip?{clip}:{}),captureBeyondViewport:false},sessionId)).data,dpr,clip),protected_regions:[]}}};
   const service=new MirrorService(host);
   try {
     const sub=await service.subscribe({tab_id:'t',generation:1,device_scale_factor:dpr},'test');
     await fill('#password');
     await fill('#plain');
     const {result}=await connection.send('Runtime.evaluate',{expression:"document.querySelector('iframe').contentDocument.querySelector('#editor')",returnByValue:false},sessionId);
     const {node}=await connection.send('DOM.describeNode',{objectId:result.objectId},sessionId);
     const node_ref=`backend:${node.backendNodeId}`;
     await browser.performAction({target_id:targetId,document_id:documentId,node_ref,action:{kind:'fill',text:value,expected_document_url:url+'frame'}});
     policy.targets.push({kind:'browser',target_id:targetId,document_id:documentId,node_ref,value_hash:hash(value)});
     await evaluate("document.querySelector('iframe').contentDocument.querySelector('#editor').innerHTML='<span>MP11-disposable-</span><b>fill-value</b>'");
     const packet=await service.next({subscription_id:sub.subscription_id,generation:1,after_sequence:0,drift_nodes:[]},'test');
     const masks=packet.nodes.filter(n=>n.kind==='mask');
     assert.equal(masks.length,2,'MP-11 top CSS box and child-document contenteditable must both be placeholders');
     for(const masked of masks){assert.deepEqual(masked.children,[]);assert.equal(masked.form,undefined);}
     assert(!packet.nodes.some(n=>n.kind==='text'&&/MP11-disposable-|fill-value/.test(n.text)&&n.parent===masks[1].id),'MP-11 split field text is absent');
     assert.equal(packet.hash,mirrorHash({root:packet.root,nodes:packet.nodes,fonts:packet.fonts,scroll:packet.scroll,focused:packet.focused,selection:packet.selection}));
     const bundleRoot=await mkdtemp(path.join(tmpdir(),'protxform-mirror-client-'));
     try {
       const bundle=path.join(bundleRoot,'client.js');
       const built=spawnSync('bun',['build',fileURLToPath(new URL('../../../../packages/kernel-client/src/browser-mirror.ts',import.meta.url)),'--target=browser',`--outfile=${bundle}`],{encoding:'utf8'});
       assert.equal(built.status,0,built.stderr);
       const code=await readFile(bundle,'utf8');
       const clientTarget=(await connection.send('Target.createTarget',{url})).targetId;
       try {
         const {sessionId:clientSession}=await browser.resolvePageTarget(clientTarget);
         const render=await connection.send('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`(async()=>{document.body.replaceChildren();const module=await import(URL.createObjectURL(new Blob([${JSON.stringify(code)}],{type:'text/javascript'})));const container=document.createElement('div');document.body.append(container);const renderer=new module.BrowserMirrorRenderer(container,async()=>{},error=>{throw error});await renderer.ready();await renderer.apply(${JSON.stringify(packet)});const result={overlays:renderer.overlays.filter(n=>n.getAttribute('aria-label')==='Protected content').length,masked:renderer.overlays.filter(n=>n.getAttribute('aria-label')==='Protected content').map(n=>n.style.background)};globalThis.__charioxMirrorFixtureRenderer=renderer;return result})()`},clientSession);
         assert.equal(render.exceptionDetails,undefined,'MP-11 real mirror client accepts the protected packet/hash');
         const passwordTile=packet.nodes.find(n=>n.tag==='input'&&n.box?.x===80&&n.box?.y===150);
         assert.equal(passwordTile.kind,'tile','MP-11 password dots use a native control tile even with custom appearance');
         assert.equal(passwordTile.attributes,undefined);assert.equal(passwordTile.form,undefined);
         const opaque=packet.nodes.filter(n=>['cross_origin_frame','opaque_shadow'].includes(n.reason)).length;
         assert.equal(render.result.value.overlays,2+opaque);
         assert(render.result.value.masked.every(color=>color==='black'));
         await connection.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:dpr,mobile:false},clientSession);
         const screenshot=(await connection.send('Page.captureScreenshot',{format:'png'},clientSession)).data;
         const rendered=decodePng(screenshot,dpr);
         for(const [x,y] of [[560,170],[790,660]]){const index=((y*dpr)*rendered.width+x*dpr)*4;assert.deepEqual([...rendered.pixels.subarray(index,index+3)],[0,0,0],'MP-11 client pixels cover top and child fields');}
         {const index=((440*dpr)*rendered.width+790*dpr)*4;assert.deepEqual([...rendered.pixels.subarray(index,index+3)],[255,0,255],'MP-11 unfilled child field stays visible');}
         {const index=((170*dpr)*rendered.width+280*dpr)*4;assert.deepEqual([...rendered.pixels.subarray(index,index+3)],[0,255,255],'MP-11 filled password control remains visible');}
         let dots=0;for(let y=160*dpr;y<180*dpr;y++)for(let x=85*dpr;x<260*dpr;x++){const i=(y*rendered.width+x)*4;if(rendered.pixels[i]<80&&rendered.pixels[i+1]<80&&rendered.pixels[i+2]<80)dots++;}
         assert(dots>10*dpr*dpr,'MP-11 the client retains actual password dots');
         if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`mirror-client-dpr${dpr}.png`),Buffer.from(screenshot,'base64'));
         await connection.send('Runtime.evaluate',{expression:'globalThis.__charioxMirrorFixtureRenderer.close()'},clientSession);
       }finally{await connection.send('Target.closeTarget',{targetId:clientTarget});}
     }finally{await rm(bundleRoot,{recursive:true,force:true});}
   }finally{clearInterval(service.expiry);service.clear();}
 }));
 test(`MP-08/MP-11 DPR${dpr}: only Vault filled plain fields, password toggle, clear and navigation`,()=>setup(dpr,async({browser,fill,collect,capture,evaluate,policy,connection,sessionId})=>{
   assert.deepEqual(await collect(),[],'registration alone never masks password, media, frames or echoes');
   await evaluate("document.querySelector('iframe').style.transform='perspective(500px) rotateY(35deg)'");
   await fill('#plain');
   let boxes=await collect();assert.equal(boxes.length,1);
   const frame=await capture('plain');const pixel=(x,y)=>[...frame.pixels.subarray(((y*dpr)*frame.width+x*dpr)*4,((y*dpr)*frame.width+x*dpr)*4+3)];
   assert.deepEqual(pixel(100,90),[0,0,0]);assert.deepEqual(pixel(100,170),[0,255,255]);assert.deepEqual(pixel(400,400),[255,255,255]);
   if(frame.video){const sample=(x,y)=>[...frame.video.subarray(((y*dpr)*frame.width+x*dpr)*4,((y*dpr)*frame.width+x*dpr)*4+3)];assert(sample(100,90).every(v=>v<5),'MP-11 plain field stays covered in decoded browser video');assert(sample(100,170)[1]>240,'MP-11 password dots/background stays visible in decoded browser video');}
   await fill('#password');assert.equal((await collect()).length,1,'password dots stay visible');
   await evaluate("document.querySelector('#password').type='text'");assert.equal((await collect()).length,2,'show password checked on every capture');await capture('toggle');
   await evaluate("document.querySelector('#plain').value='user replacement'");assert.equal((await collect()).length,1);
   await evaluate("document.querySelector('#plain').value='MP11-disposable-fill-value'");assert.equal((await collect()).length,1,'retired target never resurrects');
   await evaluate("document.querySelector('#password').value=''");assert.deepEqual(await collect(),[]);
   await fill('#area');await fill('#editor');assert.equal((await collect()).length,2,'textarea and contenteditable are fill targets');
   await evaluate("document.querySelector('#area').remove();document.querySelector('#editor').remove()");assert.deepEqual(await collect(),[]);
   await connection.send('Page.navigate',{url:'about:blank'},sessionId);assert.deepEqual(await collect(),[],'navigation retires targets');assert.equal(browser.fillTargets.size,0,'MP-11 retired targets leave the tracking registry');
 }));
 test(`MP-08/MP-11 DPR${dpr}: Vault fill in cross-origin renderer masks its own field`,()=>setup(dpr,async({browser,connection,sessionId,targetId,documentId,policy,collect,capture,url,evaluate})=>{
   const child=(await connection.send('Target.getTargets')).targetInfos.find(t=>t.type==='iframe');assert(child);
   const {sessionId:cs}=await connection.send('Target.attachToTarget',{targetId:child.targetId,flatten:true});
   let node_ref;
   try {const {frameTree}=await connection.send('Page.getFrameTree',{},cs),{root}=await connection.send('DOM.getDocument',{},cs),{nodeId}=await connection.send('DOM.querySelector',{nodeId:root.nodeId,selector:'#plain'},cs);const {node}=await connection.send('DOM.describeNode',{nodeId},cs);node_ref=`frame:${frameTree.frame.id}:${frameTree.frame.loaderId}:backend:${node.backendNodeId}`;}
   finally {await connection.send('Target.detachFromTarget',{sessionId:cs});}
   await browser.performAction({target_id:targetId,document_id:documentId,node_ref,action:{kind:'fill',text:value,expected_document_url:url.replace('127.0.0.1','localhost')+'frame'}});
   policy.targets.push({kind:'browser',target_id:targetId,document_id:documentId,node_ref,value_hash:hash(value)});
   const boxes=await collect();assert.equal(boxes.length,1);assert(boxes[0][0]>500*dpr);const frame=await capture('cross-origin');const i=((210*dpr)*frame.width+600*dpr)*4;assert.deepEqual([...frame.pixels.subarray(i,i+3)],[0,0,0]);
   await evaluate("document.querySelector('iframe').style.transform='scaleX(-1)'");
   const reflected=await collect();assert(reflected[0][0]>=590*dpr,'MP-11 exact field follows reflected frame geometry');
   const flipped=await capture('reflected');
   const pixel=x=>[...flipped.pixels.subarray(((210*dpr)*flipped.width+x*dpr)*4,((210*dpr)*flipped.width+x*dpr)*4+3)];
   assert.deepEqual(pixel(810),[0,0,0]);assert.deepEqual(pixel(570),[255,255,255]);
   if(flipped.video){const i=((210*dpr)*flipped.width+810*dpr)*4;assert([...flipped.video.subarray(i,i+3)].every(v=>v<5),'MP-11 cross-origin reflected fill stays covered in decoded video');}
 }));
}

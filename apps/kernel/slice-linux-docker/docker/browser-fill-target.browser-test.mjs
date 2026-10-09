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
import * as regions from './browser-protection-regions.mjs';
const { measureBrowserProtection, recordBrowserFill } = regions;
import { captureProtectedPage, decodePng } from './kernel-browser-pixels.mjs';
import { MirrorService } from './kernel-browser-mirror.mjs';
import { mirrorHash } from './kernel-browser-mirror-resources.mjs';
async function videoPixels(png,dpr,label,required=false,regions=[]) {
  if(!required&&process.env.CHARIOX_FILL_VIDEO!=='1')return null;
  const encoder=new PortableEncoder();
  try {
    const packet=await encoder.encode(png,8000000,true,'avc1.420033',regions.map(([x,y,width,height])=>({x,y,width,height})));
    const data=typeof packet==='string'?packet:packet.data_base64,codec=typeof packet==='string'?'vp9':'h264';
    if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`fill-dpr${dpr}-${label}.${codec}`),Buffer.from(data,'base64'));
    return await new Promise((resolve,reject)=>{
      const child=execFile(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON??'python3',['-c',"import av,sys; frame=av.CodecContext.create(sys.argv[1],'r').decode(av.Packet(sys.stdin.buffer.read()))[0];sys.stdout.buffer.write(frame.to_ndarray(format='rgba').tobytes())",codec],{encoding:'buffer',maxBuffer:2560*1600*4+4096,timeout:10000},(error,stdout)=>error?reject(Error('MP-11 video decode failed')):resolve(stdout));
      child.stdin.end(Buffer.from(data,'base64'));
    });
  }finally{await encoder.close();}
}
const value = 'MP11-disposable-fill-value';
const hash = v => createHash('sha256').update(v).digest('hex');
async function waitForClientDocument(connection,sessionId,url) {
  for(let attempt=0;attempt<100;attempt++) {
    try {const {result}=await connection.send('Runtime.evaluate',{expression:`document.URL===${JSON.stringify(url)}&&document.readyState==="complete"&&!!document.body`,returnByValue:true},sessionId);if(result.value)return;}catch(error){if(!error.message.includes('context was destroyed'))throw error;}
    await new Promise(resolve=>setTimeout(resolve,30));
  }
  throw Error('MP-11 fixture client document did not load');
}
const html = '<!doctype html><body style="margin:0;background:white"><input id=plain style="position:absolute;left:80px;top:70px;width:220px;height:40px;background:magenta;border:0"><input id=password type=password style="position:absolute;left:80px;top:150px;width:220px;height:40px;background:cyan;border:0"><textarea id=area style="position:absolute;left:80px;top:230px"></textarea><div id=editor contenteditable style="position:absolute;left:80px;top:300px;width:220px;height:40px"></div><p>MP11-disposable-fill-value</p><canvas width=200 height=80></canvas>';
function mirrorHost({browser,connection,sessionId,targetId,documentId,policy,dpr}) {
  const tab={tab_id:'fixture-tab',target_id:targetId,document_id:documentId};
  return {generation:1,browser,protection:policy,scales:new Map([[tab.tab_id,dpr]]),async target(){return tab},async displayTarget(){return tab},async screenshot(_tab,clip){return {data_base64:await captureProtectedPage(browser,tab,policy.values,policy.targets,async()=> (await connection.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,...(clip?{clip}:{})},sessionId)).data,dpr,clip)}}};
}
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
    const collect=async()=> (await measureBrowserProtection(browser,policy)).pages.find(page=>page.target_id===targetId)?.regions??[];
    const capture=async label=>{const data=await captureProtectedPage(browser,{target_id:targetId,document_id:documentId},policy.values,policy.targets,async()=>(await connection.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false},sessionId)).data,dpr);if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`fill-dpr${dpr}-${label}.png`),Buffer.from(data,'base64'));const decoded=decodePng(data,dpr);decoded.video=await videoPixels(data,dpr,label);return decoded;};
    await run({browser,connection,sessionId,targetId,documentId,evaluate,ref,policy,fill,collect,capture,url,dpr});
  } finally {await chrome?.close();server.closeAllConnections();await new Promise(r=>server.close(r));await rm(root,{recursive:true,force:true});}
}
for(const dpr of [1,2]) {
 for (const field of ['plain', 'password']) {
   test(`MP-08/MP-11 DPR${dpr}: canonical 1920x1080 image survives ${field} fill`, () => setup(dpr, async ({browser,connection,sessionId,targetId,documentId,fill}) => {
     connection.browserInstanceId='MP11-canonical-artifact-fixture';
     const viewport={css_width:1920,css_height:1080,device_scale_factor:dpr,desktop_pixel_width:1920*dpr,desktop_pixel_height:1080*dpr,revision:1,last_actor_id:null};
     await browser.reconcile(viewport,{browserBarVisible:false});
     const request={target_id:targetId,document_id:documentId,browser_generation:browser.browserGeneration,kind:'image',guid:null,viewport};
     const before=Buffer.from((await browser.captureArtifact(request)).data_base64,'base64');
     assert.equal(before.readUInt32BE(16),1920*dpr);
     assert.equal(before.readUInt32BE(20),1080*dpr);
     await fill(`#${field}`);
     const image=await browser.captureArtifact(request);
     const frame=decodePng(image.data_base64,dpr,{width:1920*dpr,height:1080*dpr});
     assert.equal(frame.width,1920*dpr);assert.equal(frame.height,1080*dpr);
     const top=field==='plain'?90:180,index=(top*dpr*frame.width+280*dpr)*4;
     assert.deepEqual([...frame.pixels.subarray(index,index+4)],field==='plain'?[0,0,0,255]:[0,255,255,255]);
     assert.equal(image.redaction,field==='plain'?'fill_targets':'none');
     assert.deepEqual([...frame.pixels.subarray((1050*dpr*frame.width+1800*dpr)*4,(1050*dpr*frame.width+1800*dpr)*4+4)],[255,255,255,255]);
     if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`canonical-${field}-dpr${dpr}.png`),Buffer.from(image.data_base64,'base64'));
   }));
 }
 test(`MP-08/MP-11 DPR${dpr}: registration fill retirement and incremental packets cross the kernel mirror boundary`,()=>setup(dpr,async context=>{
   const {browser,connection,sessionId,fill,evaluate,url}=context;
   await evaluate("Object.assign(document.querySelector('p').style,{position:'absolute',left:'80px',top:'400px',width:'420px',height:'40px',margin:'0',font:'20px monospace',color:'magenta'})");
   const service=new MirrorService(mirrorHost(context)),root=await mkdtemp(path.join(tmpdir(),'protxform-wire-'));
   try {
     const sub=await service.subscribe({tab_id:'fixture-tab',generation:1,device_scale_factor:dpr},'test');let sequence=0;const packets=[];
     const next=async()=>{const packet=await service.next({subscription_id:sub.subscription_id,generation:1,after_sequence:sequence,drift_nodes:[]},'test');sequence=packet.sequence;packets.push(packet);return packet};
     const registered=await next();
     assert(!JSON.stringify(registered.nodes).includes(value),'MP-11 registered matching paragraph is absent from structured bytes');
     assert(registered.nodes.some(n=>n.reason==='protected_text'),'MP-08 ordinary paragraph uses visible compositor pixels');
     await fill('#plain');const filled=await next();assert(!filled.reset);assert.equal(filled.nodes.filter(n=>n.kind==='mask').length,1);
     await evaluate("document.querySelector('#plain').value='';document.querySelector('p').setAttribute('title','MP11-disposable-fill-value');document.querySelector('p').textContent='Changed MP11-disposable-fill-value'");
     const retired=await next();assert(!retired.reset);assert.equal(retired.base_sequence,filled.sequence);assert(!JSON.stringify(retired.nodes).includes(value));
     const fixture=path.join(root,'mirror-wire.json');await writeFile(fixture,JSON.stringify({value,packets}));
     let wire=packets;
     if(process.env.CHARIOX_MIRROR_WIRE_TEST_BINARY) {
       const result=spawnSync(process.env.CHARIOX_MIRROR_WIRE_TEST_BINARY,['runtime::state::kernel_browser_secret_runtime::tests::mp08_mp11_mirror_wire_tree_survives_kernel_scrub_boundary','--exact','--test-threads=1'],{encoding:'utf8',env:{...process.env,CHARIOX_MIRROR_WIRE_FIXTURE:fixture},timeout:60000});
       if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`mirror-kernel-boundary-dpr${dpr}.log`),result.stdout+result.stderr);
       assert.equal(result.status,0,'MP-11 actual compiled kernel boundary replay succeeds');
       assert.match(result.stdout,/1 passed/);wire=JSON.parse(await readFile(`${fixture}.wire.json`,'utf8'));
     }
     const bundle=path.join(root,'client.js'),built=spawnSync('bun',['build',process.env.CHARIOX_MIRROR_CLIENT_SOURCE??fileURLToPath(new URL('../../../../packages/kernel-client/src/browser-mirror.ts',import.meta.url)),'--target=browser',`--outfile=${bundle}`],{encoding:'utf8'});assert.equal(built.status,0,built.stderr);
     const code=await readFile(bundle,'utf8'),target=(await connection.send('Target.createTarget',{url})).targetId;
     try {
       const {sessionId:client}=await browser.resolvePageTarget(target);
       await waitForClientDocument(connection,client,url);
       await connection.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:dpr,mobile:false},client);
       const initialized=await connection.send('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`(async()=>{document.body.replaceChildren();const module=await import(URL.createObjectURL(new Blob([${JSON.stringify(code)}],{type:'text/javascript'})));const container=document.createElement('div');document.body.append(container);globalThis.renderer=new module.BrowserMirrorRenderer(container,async()=>{},error=>{throw error});await renderer.ready();return true})()`},client);assert.equal(initialized.exceptionDetails,undefined);
       for(let index=0;index<wire.length;index++) {
         const applied=await connection.send('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`(async()=>{await renderer.apply(${JSON.stringify(wire[index])});return renderer.overlays.filter(n=>n.getAttribute('aria-label')==='Protected content').length})()`},client);
         assert.equal(applied.exceptionDetails,undefined,'MP-11 BrowserMirrorRenderer accepts post-kernel reset and incremental hashes');assert.equal(applied.result.value,index===1?1:0);
         const png=(await connection.send('Page.captureScreenshot',{format:'png'},client)).data,frame=decodePng(png,dpr);let ink=0;
         for(let y=400*dpr;y<440*dpr;y++)for(let x=80*dpr;x<500*dpr;x++){const i=(y*frame.width+x)*4;if(frame.pixels[i]>120&&frame.pixels[i+1]<80&&frame.pixels[i+2]>120)ink++;}
         assert(ink>100,'MP-08 ordinary matching paragraph stays visibly rendered');
         for(const [x,y] of [[100,540],[700,600]]){const i=((y*dpr)*frame.width+x*dpr)*4;assert.deepEqual([...frame.pixels.subarray(i,i+3)],[255,255,255],'MP-08 positioned tiles must not leave displaced black placeholders in ordinary content');}
         if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`mirror-wire-dpr${dpr}-${index}.png`),Buffer.from(png,'base64'));
       }
       await connection.send('Runtime.evaluate',{expression:'renderer.close()'},client);
     }finally{await connection.send('Target.closeTarget',{targetId:target});}
   }finally{clearInterval(service.expiry);service.clear();await rm(root,{recursive:true,force:true});}
 }));
 test(`MP-08/MP-11 DPR${dpr}: registered overflowing pseudo text survives real-client incremental replay`,()=>setup(dpr,async context=>{
   const {browser,connection,sessionId,evaluate,url,fill}=context;
   await evaluate(`(()=>{const style=document.createElement('style');style.textContent='#generated {position:absolute;left:80px;top:460px;width:40px;height:24px;white-space:nowrap;overflow:visible;font:20px monospace;color:magenta} #generated::after {content:"${value}"}';document.head.append(style);const element=document.createElement('div');element.id='generated';document.body.append(element)})()`);
   const service=new MirrorService(mirrorHost(context)),root=await mkdtemp(path.join(tmpdir(),'protxform-pseudo-'));
   let clientTarget;
   try {
     const sub=await service.subscribe({tab_id:'fixture-tab',generation:1,device_scale_factor:dpr},'test');let sequence=0;const packets=[];
     for(let stage=0;stage<2;stage++){
       if(stage){await fill('#plain');await evaluate("document.querySelector('#generated').style.top='490px'");}
       const packet=await service.next({subscription_id:sub.subscription_id,generation:1,after_sequence:sequence,drift_nodes:[]},'test');sequence=packet.sequence;packets.push(packet);
     }
     assert(!packets[1].reset,'MP-11 pseudo compositor fallback retains incremental state');
     const bundle=path.join(root,'client.js'),built=spawnSync('bun',['build',fileURLToPath(new URL('../../../../packages/kernel-client/src/browser-mirror.ts',import.meta.url)),'--target=browser',`--outfile=${bundle}`],{encoding:'utf8'});assert.equal(built.status,0,built.stderr);
     const code=await readFile(bundle,'utf8');clientTarget=(await connection.send('Target.createTarget',{url})).targetId;
     const {sessionId:client}=await browser.resolvePageTarget(clientTarget);await waitForClientDocument(connection,client,url);
     await connection.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:dpr,mobile:false},client);
     const initialized=await connection.send('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`(async()=>{document.body.replaceChildren();const module=await import(URL.createObjectURL(new Blob([${JSON.stringify(code)}],{type:'text/javascript'})));const container=document.createElement('div');document.body.append(container);globalThis.renderer=new module.BrowserMirrorRenderer(container,async()=>{},error=>{throw error});await renderer.ready();return true})()`},client);assert.equal(initialized.exceptionDetails,undefined);
     for(let stage=0;stage<packets.length;stage++){
       const packet=packets[stage];assert(!JSON.stringify(packet.nodes).includes(value),'MP-11 registered pseudo text is absent from structured bytes');
       const applied=await connection.send('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`renderer.apply(${JSON.stringify(packet)})`},client);assert.equal(applied.exceptionDetails,undefined,'MP-11 actual client accepts reset/delta hashes');
       const png=(await connection.send('Page.captureScreenshot',{format:'png'},client)).data,frame=decodePng(png,dpr);let ink=0;const top=stage?490:460;
       for(let y=top*dpr;y<(top+30)*dpr;y++)for(let x=130*dpr;x<430*dpr;x++){const i=(y*frame.width+x)*4;if(frame.pixels[i]>120&&frame.pixels[i+1]<80&&frame.pixels[i+2]>120)ink++;}
       if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`pseudo-client-dpr${dpr}-${stage}.png`),Buffer.from(png,'base64'));
       assert(ink>100,'MP-08 actual mirror preserves generated text beyond the element crop');
       if(stage){const i=(90*dpr*frame.width+100*dpr)*4;assert.deepEqual([...frame.pixels.subarray(i,i+3)],[0,0,0],'MP-11 compositor fallback still protects the filled field');}
     }
     await connection.send('Runtime.evaluate',{expression:'renderer.close()'},client);
   }finally{if(clientTarget)await connection.send('Target.closeTarget',{targetId:clientTarget});clearInterval(service.expiry);service.clear();await rm(root,{recursive:true,force:true});}
 }));
 test(`MP-08/MP-11 DPR${dpr}: hidden filled fields remain tracked across production image video and mirror capture`,()=>setup(dpr,async context=>{
   const {browser,fill,evaluate,collect,capture,connection,sessionId,targetId,documentId}=context;
   const service=new MirrorService(mirrorHost(context));
   try {
     const sub=await service.subscribe({tab_id:'fixture-tab',generation:1,device_scale_factor:dpr},'test');let sequence=0;
     const mirror=async()=>{const packet=await service.next({subscription_id:sub.subscription_id,generation:1,after_sequence:sequence,drift_nodes:[]},'test');sequence=packet.sequence;return packet};
     for(const selector of ['#plain','#editor']) {
       await fill(selector);assert.equal((await collect()).length,1);
       for(const ancestor of [false,true]) {
         await evaluate(`(()=>{const field=document.querySelector(${JSON.stringify(selector)});if(${ancestor}){const wrapper=document.createElement('section');field.before(wrapper);wrapper.append(field);wrapper.style.display='none'}else field.style.display='none'})()`);
         const hidden=await capture(`hidden-${selector.slice(1)}-${ancestor}`);
         assert.deepEqual(await collect(),[],'MP-11 confirmed non-rendered field contributes no pixels');
         assert.equal(browser.fillTargets.size,1,'MP-11 hidden retained value is not retired');
         assert.equal((await mirror()).nodes.filter(n=>n.kind==='mask').length,0,'MP-11 hidden field has no mirror mask');
         const video=await videoPixels((await connection.send('Page.captureScreenshot',{format:'png'},sessionId)).data,dpr,`hidden-${selector.slice(1)}-${ancestor}`,true);
         assert.equal(video.length,hidden.width*hidden.height*4);
         await evaluate(`(()=>{const field=document.querySelector(${JSON.stringify(selector)});(${ancestor}?field.parentElement:field).style.display=''})()`);
         const revealed=await capture(`revealed-${selector.slice(1)}-${ancestor}`);
         assert.equal((await collect()).length,1,'MP-11 reveal remasks the same retained fill');
         assert.equal((await mirror()).nodes.filter(n=>n.kind==='mask').length,1);
         const y=selector==='#plain'?90:310,i=((y*dpr)*revealed.width+100*dpr)*4;
         assert.deepEqual([...revealed.pixels.subarray(i,i+3)],[0,0,0]);
         const protectedPng=await captureProtectedPage(browser,{target_id:targetId,document_id:documentId},context.policy.values,context.policy.targets,async()=>(await connection.send('Page.captureScreenshot',{format:'png'},sessionId)).data,dpr);
         // MP-11: production sends trusted device-pixel mask metadata with its protected raster.
         const revealedVideo=await videoPixels(protectedPng,dpr,`revealed-${selector.slice(1)}-${ancestor}`,true,await collect());
         assert([...revealedVideo.subarray(i,i+3)].every(channel=>channel<5),'MP-11 revealed field stays covered in decoded production video');
       }
       await evaluate(`document.querySelector(${JSON.stringify(selector)}).${selector==='#editor'?'textContent':'value'}=''`);assert.deepEqual(await collect(),[]);
     }
     // Layout presence is not proof of an available measurement: still refuse
     // a genuine box failure for a visible, filled field.
     await fill('#plain');const send=connection.send.bind(connection);
     connection.send=(method,...args)=>method==='DOM.getBoxModel'?Promise.reject(Error('fixture box failure')):send(method,...args);
     try {await assert.rejects(capture('unknown-box'),/unavailable/);await assert.rejects(mirror(),/fixture box failure/);}finally{connection.send=send;}
   }finally{clearInterval(service.expiry);service.clear();}
 }));
 test(`MP-08/MP-11 DPR${dpr}: overflowing filled contenteditable text is covered in image and video`,()=>setup(dpr,async({browser,connection,sessionId,targetId,documentId,fill,evaluate,url})=>{
   await evaluate("Object.assign(document.querySelector('#editor').style,{width:'40px',height:'24px',whiteSpace:'nowrap',overflow:'visible',font:'20px monospace',color:'magenta',background:'white'})");
   await fill('#editor');
   const raw=decodePng((await connection.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false},sessionId)).data,dpr);
   const ink=(pixels,frame,[left,top,width,height]=[130,300,300,30])=>{let count=0;for(let y=top*dpr;y<(top+height)*dpr;y++)for(let x=left*dpr;x<(left+width)*dpr;x++){const i=(y*frame.width+x)*4;if(pixels[i]>120&&pixels[i+1]<80&&pixels[i+2]>120)count++;}return count;};
   assert(ink(raw.pixels,raw)>100,'MP-11 baseline exposes actual field glyphs outside its border box');
   connection.browserInstanceId='MP11-overflow-artifact-fixture';
   const request={target_id:targetId,document_id:documentId,browser_generation:browser.browserGeneration,kind:'image',guid:null,viewport:{css_width:1280,css_height:800,device_scale_factor:dpr,desktop_pixel_width:1280*dpr,desktop_pixel_height:800*dpr,revision:1,last_actor_id:null}};
   await browser.reconcile(request.viewport,{browserBarVisible:false});
   const image=await browser.captureArtifact(request),frame=decodePng(image.data_base64,dpr);
   if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`overflow-image-dpr${dpr}.png`),Buffer.from(image.data_base64,'base64'));
   const video=await videoPixels(image.data_base64,dpr,'overflow',true);
   if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`overflow-analysis-dpr${dpr}.json`),JSON.stringify({items:['MP-08','MP-11'],dpr,raw_overflow_ink:ink(raw.pixels,raw),image_overflow_ink:ink(frame.pixels,frame),video_overflow_ink:ink(video,frame)}));
   assert.equal(ink(frame.pixels,frame),0,'MP-11 image covers visible text belonging to the exact filled editor');
   assert.equal(ink(video,frame),0,'MP-11 decoded production video covers the same overflow glyphs');
   await evaluate("document.querySelector('#editor').style.overflow='hidden'");
   const clipped=await browser.captureArtifact(request),clippedFrame=decodePng(clipped.data_base64,dpr),i=((310*dpr)*clippedFrame.width+180*dpr)*4;
   assert.deepEqual([...clippedFrame.pixels.subarray(i,i+3)],[255,255,255],'MP-11 clipped text does not mask adjacent unfilled page content');
   await evaluate(`document.querySelector('iframe').src=${JSON.stringify(url+'frame')};Object.assign(document.querySelector('iframe').style,{top:'450px',height:'200px',transform:'rotate(4deg)'})`);
   for(let n=0;n<100;n++){const {result}=await evaluate("!!document.querySelector('iframe').contentDocument?.querySelector('#editor')");if(result.value)break;await new Promise(r=>setTimeout(r,30));}
   const nested=await connection.send('Runtime.evaluate',{expression:`(()=>{const doc=document.querySelector('iframe').contentDocument;doc.body.innerHTML='<div id=editor contenteditable style="position:absolute;left:20px;top:30px;width:40px;height:24px;white-space:nowrap;overflow:visible;font:20px monospace;color:magenta"></div>';return doc.querySelector('#editor')})()`,returnByValue:false},sessionId);
   const {node}=await connection.send('DOM.describeNode',{objectId:nested.result.objectId},sessionId);
   await browser.performAction({target_id:targetId,document_id:documentId,node_ref:`backend:${node.backendNodeId}`,action:{kind:'fill',text:value,expected_document_url:url+'frame'}});
   const nestedRaw=decodePng((await connection.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false},sessionId)).data,dpr),bounds=[450,400,650,340];
   assert(ink(nestedRaw.pixels,nestedRaw,bounds)>100,'MP-11 baseline includes the transformed child-field text');
   const nestedImage=await browser.captureArtifact(request),nestedFrame=decodePng(nestedImage.data_base64,dpr);
   const nestedVideo=await videoPixels(nestedImage.data_base64,dpr,'overflow-frame',true);
   assert.equal(ink(nestedFrame.pixels,nestedFrame,bounds),0,'MP-11 child CSS text bounds map through the frame transform once');
   assert.equal(ink(nestedVideo,nestedFrame,bounds),0,'MP-11 child text is covered in decoded video');
   if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`overflow-frame-image-dpr${dpr}.png`),Buffer.from(nestedImage.data_base64,'base64'));
 }));
 test(`MP-08/MP-11 DPR${dpr}: visible editor overflow beneath a hidden ancestor stays protected`,()=>setup(dpr,async({browser,connection,sessionId,targetId,documentId,fill,evaluate})=>{
   await evaluate("(()=>{const editor=document.querySelector('#editor'),wrapper=document.createElement('section');editor.before(wrapper);wrapper.append(editor);wrapper.style.visibility='hidden';Object.assign(editor.style,{visibility:'visible',width:'40px',height:'24px',whiteSpace:'nowrap',overflow:'visible',font:'20px monospace',color:'magenta',background:'white'})})()");
   await fill('#editor');
   const ink=frame=>{let count=0;for(let y=300*dpr;y<330*dpr;y++)for(let x=130*dpr;x<430*dpr;x++){const i=(y*1280*dpr+x)*4;if(frame[i]>120&&frame[i+1]<80&&frame[i+2]>120)count++;}return count;};
   const raw=(await connection.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false},sessionId)).data;
   assert(ink(decodePng(raw,dpr).pixels)>100,'MP-11 descendant visibility restores actual overflow glyphs');
   connection.browserInstanceId='MP11-visible-descendant-fixture';
   const request={target_id:targetId,document_id:documentId,browser_generation:browser.browserGeneration,kind:'image',guid:null,viewport:{css_width:1280,css_height:800,device_scale_factor:dpr,desktop_pixel_width:1280*dpr,desktop_pixel_height:800*dpr,revision:1,last_actor_id:null}};
   await browser.reconcile(request.viewport,{browserBarVisible:false});
   const image=await browser.captureArtifact(request),frame=decodePng(image.data_base64,dpr),video=await videoPixels(image.data_base64,dpr,'visible-descendant',true);
   if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)for(const [label,data] of [['raw',raw],['protected',image.data_base64]])await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`visible-descendant-${label}-dpr${dpr}.png`),Buffer.from(data,'base64'));
   assert.equal(ink(frame.pixels),0,'MP-11 image covers visible filled descendant text beyond its box');
   assert.equal(ink(video),0,'MP-11 decoded production video covers visible filled descendant overflow');
   await evaluate("document.querySelector('#editor').style.visibility='hidden'");
   const hidden=decodePng((await browser.captureArtifact(request)).data_base64,dpr),index=(310*dpr*hidden.width+180*dpr)*4;
   assert.deepEqual([...hidden.pixels.subarray(index,index+3)],[255,255,255],'MP-08 hidden text does not mask ordinary pixels outside its field box');
   await evaluate(`document.querySelector('#editor').innerHTML='<span style="visibility:visible">${value}</span>'`);
   const child=await browser.captureArtifact(request);
   assert.equal(ink(decodePng(child.data_base64,dpr).pixels),0,'MP-11 visible text descendants of a hidden editor stay covered');
 }));
 // MP-08/MP-11: the host's screencast-triggered captures run outside the kernel's Vault
 // input barrier; one landing between recording and the completed value must not retire it.
 test(`MP-08/MP-11 DPR${dpr}: a capture during an in-flight fill keeps the field tracked`,()=>setup(dpr,async({browser,connection,sessionId,targetId,documentId,ref,policy,collect,evaluate})=>{
   const node_ref=await ref('#plain');
   policy.targets.push({kind:'browser',target_id:targetId,document_id:documentId,node_ref,value_hash:hash(value)});
   const target=await recordBrowserFill(connection,{sessionId,targetId,documentId,nodeRef:node_ref,browserGeneration:browser.browserGeneration,action:{kind:'fill'}},value,1);
   browser.fillTargets.set(`${targetId}:${node_ref}`,target);
   assert.equal((await collect()).length,1,'MP-11 an in-flight fill stays covered');
   await evaluate(`document.querySelector('#plain').value=${JSON.stringify(value)}`);
   regions.finishBrowserFill?.(connection,target);
   assert.equal((await collect()).length,1,'MP-11 the completed fill is still masked');
 }));

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
   const foreign=new URL(url+'frame');foreign.hostname='localhost';
   await connection.send('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`(async()=>{const frame=document.createElement('iframe');frame.id='foreign';frame.style.cssText='position:absolute;left:1000px;top:350px;width:240px;height:300px;border:0';const loaded=new Promise(resolve=>frame.onload=resolve);frame.src=${JSON.stringify(foreign.href)};document.body.append(frame);await loaded;const widget=document.createElement('mp11-widget');widget.style.cssText='position:absolute;display:block;left:1000px;top:70px;width:150px;height:80px';widget.attachShadow({mode:'closed'}).innerHTML='<div style="width:150px;height:80px;background:cyan"></div>';document.body.append(widget);return true})()`},sessionId);
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
       const built=spawnSync('bun',['build',process.env.CHARIOX_MIRROR_CLIENT_SOURCE??fileURLToPath(new URL('../../../../packages/kernel-client/src/browser-mirror.ts',import.meta.url)),'--target=browser',`--outfile=${bundle}`],{encoding:'utf8'});
       assert.equal(built.status,0,built.stderr);
       const code=await readFile(bundle,'utf8');
       const clientTarget=(await connection.send('Target.createTarget',{url})).targetId;
       try {
         const {sessionId:clientSession}=await browser.resolvePageTarget(clientTarget);
         await waitForClientDocument(connection,clientSession,url);
         const render=await connection.send('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`(async()=>{document.body.replaceChildren();const module=await import(URL.createObjectURL(new Blob([${JSON.stringify(code)}],{type:'text/javascript'})));const container=document.createElement('div');document.body.append(container);const renderer=new module.BrowserMirrorRenderer(container,async()=>{},error=>{throw error});await renderer.ready();await renderer.apply(${JSON.stringify(packet)});const result={overlays:renderer.overlays.filter(n=>n.getAttribute('aria-label')==='Protected content').length,masked:renderer.overlays.filter(n=>n.getAttribute('aria-label')==='Protected content').map(n=>n.style.background)};globalThis.__charioxMirrorFixtureRenderer=renderer;return result})()`},clientSession);
         assert.equal(render.exceptionDetails,undefined,'MP-11 real mirror client accepts the protected packet/hash');
         const passwordTile=packet.nodes.find(n=>n.tag==='input'&&n.box?.x===80&&n.box?.y===150);
         assert.equal(passwordTile.kind,'tile','MP-11 password dots use a native control tile even with custom appearance');
         assert.equal(passwordTile.attributes,undefined);assert.equal(passwordTile.form,undefined);
         const opaque=packet.nodes.filter(n=>['cross_origin_frame','opaque_shadow'].includes(n.reason));
         assert.equal(opaque.length,2,'MP-11 fixture includes actual foreign-frame and closed-shadow tiles');
         assert(opaque.every(node=>packet.tiles.some(tile=>tile.node_id===node.id)),'MP-11 ordinary opaque subtrees have protected compositor pixels');
         assert(render.result.value.masked.every(color=>color==='black'));
         await connection.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:dpr,mobile:false},clientSession);
         const screenshot=(await connection.send('Page.captureScreenshot',{format:'png'},clientSession)).data;
         const rendered=decodePng(screenshot,dpr);
         if(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE)await writeFile(path.join(process.env.CHARIOX_PROTECTION_TEST_EVIDENCE,`mirror-opaque-client-dpr${dpr}.png`),Buffer.from(screenshot,'base64'));
         assert.equal(render.result.value.overlays,2,'MP-11 only actual filled-field mask records receive black overlays');
         {const index=((90*dpr)*rendered.width+1020*dpr)*4;assert.deepEqual([...rendered.pixels.subarray(index,index+3)],[0,255,255],'MP-11 ordinary closed-shadow pixels remain visible');}
         {const index=((440*dpr)*rendered.width+1110*dpr)*4;assert.deepEqual([...rendered.pixels.subarray(index,index+3)],[255,0,255],'MP-11 ordinary foreign-frame pixels remain visible');}
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
 test(`MP-08/MP-11 DPR${dpr}: capture cannot retire the fill before insertion completes`,()=>setup(dpr,async({browser,connection,fill,collect})=>{
   const send=connection.send.bind(connection);let intercepted=false;
   connection.send=async(method,params,session)=>{
     if(!intercepted&&method==='Runtime.callFunctionOn'&&params.functionDeclaration?.includes('expectedDocumentUrl')){
       intercepted=true;
       assert.equal((await collect()).length,1,'MP-11 pending plain fill is covered, never retired');
     }
     return send(method,params,session);
   };
   await fill('#plain');assert(intercepted);assert.equal((await collect()).length,1,'completed fill remains tracked');
 }));
 test(`MP-08/MP-11 DPR${dpr}: registering a target without a Vault fill never masks it`,()=>setup(dpr,async({evaluate,ref,policy,targetId,documentId,collect})=>{
   const node_ref=await ref('#plain');policy.targets.push({kind:'browser',target_id:targetId,document_id:documentId,node_ref});
   await evaluate("document.querySelector('#plain').value="+JSON.stringify(value));
   assert.deepEqual(await collect(),[],'user-authored matching field is not a Vault fill');
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

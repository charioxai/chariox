// MP-08/MP-10/MP-11: headless reference client + real sandboxed host Chromium.
// Component drill. It does not substitute for the production encrypted relay,
// provider model, Cloud UI, signed release, Room, WAN or macOS acceptance.
import { createServer } from 'node:http';
import { readFile,writeFile,mkdir,cp,mkdtemp,chmod,chown,rm,readdir } from 'node:fs/promises';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { spawn,execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import assert from 'node:assert/strict';
import {mirrorRasterMetrics} from './browser-mirror-metrics.mjs';
const here=path.dirname(fileURLToPath(import.meta.url));
if(process.argv[2]!=='child') {
  const [tools,compiled,output]=process.argv.slice(2);
  if(![tools,compiled,output].every(v=>v&&path.isAbsolute(v)))throw Error('MP-10: absolute tools, compiled-client and external evidence paths required');
  const root=await mkdtemp(path.join(tmpdir(),'chariox-mdmirror-'));
  await chmod(root,0o755);await mkdir(output,{recursive:true});
  const source=execFileSync('git',['rev-parse','HEAD'],{cwd:path.join(here,'../..'),encoding:'utf8'}).trim();
  const dirty=execFileSync('git',['status','--porcelain'],{cwd:path.join(here,'../..'),encoding:'utf8'}).trim();
  const trackedDirty=execFileSync('git',['status','--porcelain','--untracked-files=no'],{cwd:path.join(here,'../..'),encoding:'utf8'}).trim();
  let code=1, sandboxProfile=null;
  try {
    await cp(path.join(here,'browser-mirror-drill.mjs'),path.join(root,'run.mjs'));
    await cp(path.join(here,'browser-mirror-metrics.mjs'),path.join(root,'browser-mirror-metrics.mjs'));
    await mkdir(path.join(root,'controller'));for(const name of await readdir(path.join(here,'../../apps/kernel/slice-linux-docker/docker')))if(name.endsWith('.mjs')&&!name.includes('.test.'))await cp(path.join(here,'../../apps/kernel/slice-linux-docker/docker',name),path.join(root,'controller',name));
    await cp(compiled,path.join(root,'client'),{recursive:true});
    for(const name of ['playwright-core','ws','pngjs'])await cp(path.join(tools,'node_modules',name),path.join(root,'node_modules',name),{recursive:true});
    await cp(path.join(tools,'browsers'),path.join(root,'browsers'),{recursive:true});
    await cp(path.join(tools,'sysroot'),path.join(root,'sysroot'),{recursive:true});
    const chromeDir=path.join(root,'browsers',(await readdir(path.join(root,'browsers'))).find(n=>n.startsWith('chromium-')),'chrome-linux64');
    // Ubuntu restricts unprivileged user namespaces for unknown private binaries.
    // Register ONLY this disposable executable so Chromium can build its real
    // renderer sandbox. No global sysctl or existing AppArmor profile changes.
    sandboxProfile=path.join(root,'chromium.apparmor');
    await writeFile(sandboxProfile,`abi <abi/4.0>,\ninclude <tunables/global>\nprofile chariox-mdmirror-${path.basename(root)} ${chromeDir}/chrome flags=(unconfined) {\n  userns,\n}\n`);
    execFileSync('/usr/sbin/apparmor_parser',['-r',sandboxProfile]);
    await writeFile(path.join(root,'fonts.conf'),`<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd"><fontconfig><dir>${root}/sysroot/usr/share/fonts</dir><cachedir>${root}/home/font-cache</cachedir></fontconfig>`);
    await writeFile(path.join(root,'chromium-launcher'),`#!/bin/sh\nexport LD_LIBRARY_PATH='${root}/sysroot/usr/lib/x86_64-linux-gnu'\nexport FONTCONFIG_FILE='${root}/fonts.conf'\nexec '${chromeDir}/chrome' "$@" 2>>'${root}/evidence/chromium.log'\n`,{mode:0o755});
    for(const name of ['home','evidence']){await mkdir(path.join(root,name),{mode:0o700});await chown(path.join(root,name),65534,65534);}
    const child=spawn(process.execPath,[path.join(root,'run.mjs'),'child',root],{uid:65534,gid:65534,cwd:root,env:{PATH:'/usr/bin:/bin',HOME:path.join(root,'home'),TMPDIR:path.join(root,'home'),CHARIOX_KERNEL_BROWSER_HEADLESS:'1',CHARIOX_KERNEL_BROWSER_MIRROR:'1',CHARIOX_BROWSER_DISPLAY_TIMING:'1'},stdio:['ignore','pipe','pipe']});
    let logs='';for(const stream of [child.stdout,child.stderr])stream.on('data',chunk=>{logs+=chunk.toString();if(logs.length>65536)logs=logs.slice(-65536)});
    code=await new Promise(resolve=>child.once('exit',exit=>resolve(exit??1)));
    await cp(path.join(root,'evidence'),output,{recursive:true});
    await writeFile(path.join(output,'run.log'),logs);
    const codeHashes={};for(const dir of ['controller','client'])for(const name of await readdir(path.join(root,dir)))if(name.endsWith('.mjs')||name.endsWith('.js'))codeHashes[`${dir}/${name}`]=createHash('sha256').update(await readFile(path.join(root,dir,name))).digest('hex');
    for(const name of ['run.mjs','browser-mirror-metrics.mjs'])codeHashes[name]=createHash('sha256').update(await readFile(path.join(root,name))).digest('hex');
    await writeFile(path.join(output,'execution-hashes.json'),JSON.stringify(codeHashes,null,2));
    await writeFile(path.join(output,'provenance.json'),JSON.stringify({items:['MP-08','MP-10','MP-11'],source,dirty:Boolean(dirty),tracked_dirty:Boolean(trackedDirty),command:process.argv.slice(1),exit_code:code,topology:'isolated Node host controller + real Chromium + shared renderer, no Rust or relay',cleanup:'host-owned Chromium stopped; exact disposable state removed'},null,2));
  }finally{if(sandboxProfile)execFileSync('/usr/sbin/apparmor_parser',['-R',sandboxProfile]);await rm(root,{recursive:true,force:true});}
  process.exitCode=code;
}else{
  const root=process.argv[3],require=createRequire(import.meta.url),{chromium}=require('playwright-core'),{PNG}=require('pngjs');
  const {KernelBrowserHost}=await import(path.join(root,'controller/kernel-browser-host.mjs'));
  const {HostChromium}=await import(path.join(root,'controller/kernel-browser-process.mjs'));
  const {encodePng,maskPng}=await import(path.join(root,'controller/kernel-browser-pixels.mjs'));
  const {mirrorCanonicalJson}=await import(path.join(root,'controller/kernel-browser-mirror-resources.mjs'));
  const chrome=path.join(root,'browsers',(await readdir(path.join(root,'browsers'))).find(n=>n.startsWith('chromium-')),'chrome-linux64/chrome');
  process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE=path.join(root,'chromium-launcher');
  const source=new KernelBrowserHost(path.join(root,'home/source')),viewerHost=new HostChromium(path.join(root,'home/viewer'));
  const receipt={items:['MP-08','MP-10','MP-11'],status:'RED',measurement:'input start -> native client compositor readback of expected binary acknowledgement -> rAF; software presentation proxy, no source-capture round trip in endpoint',rows:[],security:[],resources:[],cleanup:[]};
  const ownedCpu=async()=>{const pairs=execFileSync('ps',['-eo','pid=,ppid='],{encoding:'utf8'}).trim().split('\n').map(line=>line.trim().split(/\s+/).map(Number));const owned=new Set([process.pid]);for(let i=0;i<16;i++)for(const [pid,parent]of pairs)if(owned.has(parent)&&pid>1)owned.add(pid);let ticks=0;for(const pid of owned)try{const stat=await readFile(`/proc/${pid}/stat`,'utf8'),parts=stat.slice(stat.lastIndexOf(')')+2).split(' ');ticks+=Number(parts[11])+Number(parts[12]);}catch{}return {ticks,clock_ticks_per_second:Number(execFileSync('getconf',['CLK_TCK'],{encoding:'utf8'})),process_count:owned.size,scope:'live descendants of this synthetic drill, short-lived exited processes excluded'};};
  const resource=async()=>{const mem=await readFile('/proc/meminfo','utf8'),free=Number(/MemAvailable:\s+(\d+)/.exec(mem)[1])*1024;const disk=Number(execFileSync('df',['-B1','--output=avail','/'],{encoding:'utf8'}).trim().split('\n').at(-1));receipt.resources.push({at:new Date().toISOString(),mem_available_bytes:free,disk_available_bytes:disk,cpu:process.cpuUsage(),owned_cpu:await ownedCpu()});assert(free>=9*1024**3&&disk>=10*1024**3,'MP-10 resource floor');};
  const image=Buffer.from(encodePng(20,20,Buffer.alloc(20*20*4,200)),'base64');
  const fixture=kind=>`<!doctype html><html><head><style>*{box-sizing:border-box}html,body{margin:0;font:16px Arial,sans-serif;background:#fff;color:#172033}main{padding:20px;width:1100px}p{margin:6px 0;line-height:24px}p:first-of-type{max-width:360px}h1{-webkit-text-stroke-width:1px;-webkit-text-stroke-color:#172033}button,input{font:inherit;margin:4px;padding:6px}#counter{display:inline-block;width:60px;background:#cfe8ff}canvas{display:block;width:90px;height:60px}iframe{width:320px;height:100px;border:1px solid #aaa}x-card{display:block}</style></head><body><main><h1>${kind}</h1><button onclick="document.querySelector('#counter').textContent=++window.counter;window.paintAck(window.counter)">Increment</button><span id="counter">0</span><div id="ack" style="position:fixed;right:20px;top:20px;width:64px;height:16px"><span style="float:left;width:16px;height:16px;background:#f30"></span><span style="float:left;width:16px;height:16px;background:#f30"></span><span style="float:left;width:16px;height:16px;background:#f30"></span><span style="float:left;width:16px;height:16px;background:#f30"></span></div>${kind==='docs'||kind==='long'?Array.from({length:kind==='long'?100:12},(_,i)=>`<p>Paragraph ${i}: kernel-owned mirrored text, selection and mutation evidence.</p>`).join(''):''}${kind==='forms'?'<input id="ordinary" placeholder="Ordinary field"><input type="password" value="MASK-ME"><section data-chariox-observation-protected>PROTECTED-FIXTURE</section>':''}${kind==='shadow'?'<x-card></x-card><script>document.querySelector("x-card").attachShadow({mode:"open"}).innerHTML="<p>Shadow text</p><button>Shadow action</button>"</script>':''}${kind==='frames'?'<iframe src="/nested"></iframe><iframe src="https://invalid.example/"></iframe>':''}${kind==='media'?'<canvas></canvas><img src="/image.png"><video></video><script>let ctx=document.querySelector("canvas").getContext("2d");ctx.fillStyle="#f30";ctx.fillRect(0,0,90,60)</script>':''}${kind==='spa'?'<div id="spa"></div><script>for(let i=0;i<60;i++){let n=document.createElement("p");n.textContent="SPA "+i;document.querySelector("#spa").append(n)}</script>':''}</main><script>window.counter=0;window.paintAck=value=>document.querySelectorAll('#ack span').forEach((n,i)=>n.style.background=(value>>i)&1?'#19a':'#f30');window.pageForged=typeof globalThis.__charioxMirror;window.updateFixture=()=>{document.querySelector("h1").textContent="Changed heading";document.querySelector("main").setAttribute("title","changed");let n=document.createElement("p");n.textContent="new child";document.querySelector("main").append(n);document.querySelector("p")?.remove()};</script></body></html>`;
  const origin=createServer((req,res)=>{if(req.url==='/image.png'){res.setHeader('content-type','image/png');res.end(image);}else if(req.url==='/nested-protected'){res.end('<!doctype html><html><body style="margin:0;font:16px Arial"><button style="width:210px;height:45px;margin:0">Under field</button><input type="TEXT" autocomplete="CC-NUMBER" value="SYNTHETIC-CARD" style="position:absolute;left:10px;top:0;width:140px;height:40px;box-sizing:border-box;margin:0"></body></html>');}else if(req.url==='/nested'){res.end('<!doctype html><html><body style="margin:0;font:16px Arial">Nested text <button>Frame button</button></body></html>');}else{res.setHeader('content-type','text/html');res.end(fixture(req.url.slice(1)));}});
  // MP-10: bounded loopback JSON transport, no recursive automation RPC.
  const kernelRpc=async request=>{
      const command=request.KernelBrowser.command;
      if(command.op==='mirror_input'){const {action,subscription_id,sequence,...binding}=command;return mirrorCanonicalJson({KernelBrowser:{result:await source.request({...binding,op:'input',input:{kind:'mirror',action,subscription_id,sequence},observed_by:'drill'})}});}
      return mirrorCanonicalJson({KernelBrowser:{result:await source.request({...command,observed_by:'drill'})}});
  };
  const serve=createServer(async(req,res)=>{if(req.method==='POST'&&req.url==='/kernel'){try{let body='';for await(const chunk of req){body+=chunk;assert(Buffer.byteLength(body)<=65536,'MP-10 bounded synthetic request');}const response=await kernelRpc(JSON.parse(body));res.setHeader('content-type','application/json');res.end(response);}catch(error){res.writeHead(500,{'content-type':'application/json'});res.end(JSON.stringify({error:error.message}));}}else if(req.url==='/'){res.setHeader('content-type','text/html');res.end('<!doctype html><html><body style="margin:0"><div id="mirror"></div><script type="module">import {attachBrowserMirror} from "/browser-mirror.js";window.start=async binding=>{window.mirror=await attachBrowserMirror({protocolVersion:433,request:async request=>{const reply=await fetch("/kernel",{method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify(request)});const result=await reply.json();if(!reply.ok)throw Error(result.error);return result;}},document.querySelector("#mirror"),binding,()=>{});};</script></body></html>');}else try{const name=path.basename(req.url);if(!name.endsWith('.js'))throw Error();res.setHeader('content-type','text/javascript');res.end(await readFile(path.join(root,'client',name)));}catch{res.writeHead(404);res.end();}});
  let viewer,browser;const closeServer=server=>new Promise(resolve=>server.close(resolve));
  try{
    await resource();await new Promise(resolve=>origin.listen(0,'127.0.0.1',resolve));await new Promise(resolve=>serve.listen(0,'127.0.0.1',resolve));
    const originUrl=`http://127.0.0.1:${origin.address().port}`,viewerUrl=`http://127.0.0.1:${serve.address().port}`;
    const endpoint=await viewerHost.start();browser=await chromium.connectOverCDP(endpoint);viewer=await browser.contexts()[0].newPage();await viewer.setViewportSize({width:1280,height:800});
    let forbiddenRequests=0;viewer.on('request',req=>{if(req.url().startsWith(originUrl))forbiddenRequests++});
    const viewerCdp=await browser.contexts()[0].newCDPSession(viewer);
    // MP-10: send JSON bytes across the synthetic bridge, as production does.
    // Playwright's recursive JS-object serializer is not a product transport.
    const nextPacket=async()=>JSON.parse(await viewer.evaluate(async()=>JSON.stringify(await window.mirror.next())));
    receipt.environment={node:process.version,chromium:browser.version(),bridge:'bounded same-origin loopback HTTP with recursively sorted JSON maps; no Playwright RPC bridge, Rust or relay'};
    for(const dpr of [1,2])for(const kind of ['docs','forms','spa','shadow','frames','media','long']){
      const opened=await source.request({op:'open',url:`${originUrl}/${kind}`,observed_by:'drill'}),generation=opened.generation,tab_id=opened.tab_id;
      const prepared=await source.request({op:'mirror_subscribe',tab_id,generation,device_scale_factor:dpr,observed_by:'drill'});
      await source.request({op:'navigate',tab_id,generation,url:`${originUrl}/${kind}`,observed_by:'drill'});
      await source.request({op:'mirror_close',subscription_id:prepared.subscription_id,generation,observed_by:'drill'});
      await viewer.goto(viewerUrl);
      await viewerCdp.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:dpr,mobile:false});await viewer.waitForFunction(()=>typeof window.start==='function');await viewer.evaluate(binding=>window.start(binding),{tab_id,generation,device_scale_factor:dpr});
      const at=performance.now();let packet=await nextPacket();const initial=packet,bootstrapMs=performance.now()-at,bootstrapBytes=JSON.stringify(packet).length;
      const nativeClient=await viewerCdp.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,fromSurface:true,optimizeForSpeed:true});const clientPng=Buffer.from(nativeClient.data,'base64');
      await viewer.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>resolve())));const firstMeaningfulPaintMs=performance.now()-at;
      const connection=await source.browser.ensureConnection(),tab=source.tabs.get(tab_id),sessionId=await source.browser.ensureTargetSession(connection,tab.target_id);
      const textReady=await connection.send('Runtime.evaluate',{expression:'(()=>{const r=document.createRange();r.selectNodeContents(document.querySelector("h1"));return r.getBoundingClientRect().width>0})()',returnByValue:true},sessionId);assert.equal(textReady.result.value,true,'MP-10: native fixture must have visible text glyphs');
      const pageWorld=await connection.send('Runtime.evaluate',{expression:'window.pageForged',returnByValue:true},sessionId);assert.equal(pageWorld.result.value,'undefined');
      // Pixel reference masks generic protected fields before comparison.
      const captured=await source.screenshot(tab),masked=initial.nodes.filter(n=>n.kind==='mask'||n.reason==='cross_origin_frame'||n.reason==='opaque_shadow').flatMap(n=>n.box?[[n.box.x*dpr,n.box.y*dpr,n.box.width*dpr,n.box.height*dpr]]:[]),sourcePng=Buffer.from(maskPng(captured.data_base64,masked,dpr),'base64');
      await viewer.evaluate(()=>{window.mirror.renderer.frame.style.width='1280px';window.mirror.renderer.frame.style.height='800px';});

      // At DPR2 launch viewer CDP emulation explicitly for native raster pairs.
      await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-initial.json`),JSON.stringify(initial));
      const sourceLayout=await connection.send('Runtime.evaluate',{expression:'JSON.stringify({dpr:devicePixelRatio,width:innerWidth,height:innerHeight,rects:Array.from(document.querySelectorAll("p,button,input,canvas,img,video")).slice(0,5).map(n=>({tag:n.tagName,rect:n.getBoundingClientRect().toJSON(),font:getComputedStyle(n).font,zoom:getComputedStyle(n).zoom}))})',returnByValue:true},sessionId);
      const clientLayout=await viewer.evaluate(()=>{const w=window.mirror.renderer.frame.contentWindow;return {dpr:w.devicePixelRatio,width:w.innerWidth,height:w.innerHeight,tiles:window.mirror.renderer.overlays.map(n=>({rect:n.getBoundingClientRect().toJSON(),style:n.style.cssText,naturalWidth:n.naturalWidth,naturalHeight:n.naturalHeight})),rects:Array.from(w.document.querySelectorAll('p,button,input,canvas,img,video')).slice(0,5).map(n=>({tag:n.tagName,rect:n.getBoundingClientRect().toJSON(),font:w.getComputedStyle(n).font,zoom:w.getComputedStyle(n).zoom}))}});
      await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-layout.json`),JSON.stringify({source:JSON.parse(sourceLayout.result.value),client:clientLayout}));
      const clientPixels=PNG.sync.read(clientPng),sourcePixels=PNG.sync.read(sourcePng);const raster=mirrorRasterMetrics(sourcePixels,clientPixels);let mse=null,mismatch=null;
      mse=raster.pixel_mse;mismatch=raster.pixel_mismatch_fraction;
      const layout=await viewer.evaluate(()=>window.mirror.renderer.layoutFidelity());
      await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-source.png`),sourcePng);await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-client.png`),clientPng);
      const hydrationAt=performance.now();let hydrationBytes=0,hydrationMs=0;
      if(initial.nodes.some(n=>n.reason==='viewport_deferred')){packet=await nextPacket();hydrationBytes=JSON.stringify(packet).length;hydrationMs=performance.now()-hydrationAt;assert(!packet.nodes.some(n=>n.reason==='viewport_deferred'));}
      const settledLayout=await viewer.evaluate(()=>window.mirror.renderer.layoutFidelity());
      const clientRuns=await viewer.evaluate(()=>window.mirror.renderer.textRunGeometry());
      const isolated=await source.mirror.world(tab),sourceRuns=await source.mirror.evaluate(isolated,`globalThis.__charioxMirror.textRuns(${JSON.stringify(clientRuns.map(n=>n.id))})`);
      let textRunMaxError=0,textRunCount=0;assert.equal(sourceRuns.length,clientRuns.length);
      for(let i=0;i<sourceRuns.length;i++){assert.equal(sourceRuns[i].id,clientRuns[i].id);assert.equal(sourceRuns[i].rects.length,clientRuns[i].rects.length,'MP-10 text line count');for(let j=0;j<sourceRuns[i].rects.length;j++){textRunCount++;for(const key of ['x','y','width','height'])textRunMaxError=Math.max(textRunMaxError,Math.abs(sourceRuns[i].rects[j][key]-clientRuns[i].rects[j][key]));}}
      settledLayout.text_runs=textRunCount;settledLayout.text_run_max_error_css_px=textRunMaxError;
      await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-fidelity.json`),JSON.stringify({layout,settledLayout,drift:await viewer.evaluate(()=>window.mirror.renderer.driftNodes())}));
      const counter=initial.nodes.find(n=>n.kind==='text'&&n.text==='0');
      const acknowledgement=initial.nodes.find(n=>n.tag==='div'&&n.style?.position==='fixed'&&n.box?.width===64&&n.box?.height===16);assert(acknowledgement,'MP-10 source acknowledgement geometry');
      const counterClip={...acknowledgement.box,scale:1};
      const verifiedPresentation=async (expected,visual=expected)=>{
        const text=await viewer.evaluate(id=>window.mirror.renderer.dom.get(id)?.textContent,counter.parent);if(text===undefined){await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-drift-failure.json`),JSON.stringify({packet,layout:await viewer.evaluate(()=>window.mirror.renderer.layoutFidelity())}));}assert.equal(text,String(expected),'MP-10: rendered counter remains mirrored');
        let matched=false,readbacks=0;const deadline=performance.now()+500;
        while(!matched&&readbacks<12&&performance.now()<deadline){
          const visible=await viewerCdp.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,optimizeForSpeed:true,clip:counterClip});readbacks++;
          const pixels=PNG.sync.read(Buffer.from(visible.data,'base64'));assert.equal(pixels.width,64*dpr);assert.equal(pixels.height,16*dpr);matched=true;
          for(let y=0;y<pixels.height&&matched;y++)for(let x=0;x<pixels.width&&matched;x++){const expectedColor=(visual>>Math.floor(x/(16*dpr)))&1?[17,153,170]:[255,51,0];for(let c=0;c<3;c++)if(pixels.data[(y*pixels.width+x)*4+c]!==expectedColor[c]){matched=false;break;}}
          if(!matched)await viewer.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>resolve())));
        }
        if(!matched){const actual=await viewerCdp.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,optimizeForSpeed:true});await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-ack-failure.png`),Buffer.from(actual.data,'base64'));await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-ack-failure.json`),JSON.stringify({expected,visual,packet,records:await viewer.evaluate(()=>[...window.mirror.renderer.records.values()]),layout:await viewer.evaluate(()=>window.mirror.renderer.layoutFidelity())}));}
        assert(matched,'MP-10: visible binary counter raster acknowledgement within bounded readbacks');
        await viewer.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>resolve())));return readbacks;
      };
      const trials=[];
      const button=initial.nodes.find(n=>n.tag==='button');const latencies=[],patchBytes=[];
      for(let i=0;i<10;i++){const started=performance.now(),wall=Date.now();await viewer.evaluate(node_id=>window.mirror.input({kind:'click',node_id}),button.id);const inputDone=performance.now();packet=await nextPacket();const applyDone=performance.now();const readbacks=await verifiedPresentation(i+1);const ended=performance.now();latencies.push(ended-started);patchBytes.push(JSON.stringify(packet).length);trials.push({kind:'input',readbacks,started_ms:wall,ended_ms:Date.now(),input_request_ms:inputDone-started,next_and_apply_ms:applyDone-inputDone,visible_readback_and_raf_ms:ended-applyDone,total_ms:ended-started});}
      const effects=await connection.send('Runtime.evaluate',{expression:'window.counter',returnByValue:true},sessionId);assert.equal(effects.result.value,10);
      await connection.send('Runtime.evaluate',{expression:'window.updateFixture()',returnByValue:true},sessionId);const mutationAt=performance.now();const mutated=await nextPacket();const mutationMs=performance.now()-mutationAt;assert(mutated.nodes.some(n=>n.text==='Changed heading'));
      const mutationLatencies=[];for(let i=0;i<10;i++){const at=performance.now(),wall=Date.now();await connection.send('Runtime.evaluate',{expression:`document.querySelector('h1').textContent='Mutation ${i}';window.paintAck(${i});${kind==='spa'?`for(const [j,node] of Array.from(document.querySelector('#spa').children).entries()){node.textContent='SPA mutation '+j+' tick ${i}';node.style.color=${i%2?"'#172033'":"'#174ea6'"};}`:''}`,returnByValue:true},sessionId);const sourceDone=performance.now(),delta=await nextPacket(),applyDone=performance.now();assert(delta.nodes.some(n=>n.text===`Mutation ${i}`));const readbacks=await verifiedPresentation(10,i),ended=performance.now();mutationLatencies.push(ended-at);trials.push({kind:'mutation',readbacks,mutation_batch_nodes:kind==='spa'?60:1,started_ms:wall,ended_ms:Date.now(),source_mutate_ms:sourceDone-at,next_and_apply_ms:applyDone-sourceDone,visible_readback_and_raf_ms:ended-applyDone,total_ms:ended-at})}mutationLatencies.sort((a,b)=>a-b);
      // MP-10: settled mutation fidelity complements the initial viewport pair.
      const mutationSource=Buffer.from(maskPng((await source.screenshot(tab)).data_base64,masked,dpr),'base64');
      const mutationClient=Buffer.from((await viewerCdp.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,fromSurface:true,optimizeForSpeed:true})).data,'base64');
      const mutationRaster=mirrorRasterMetrics(PNG.sync.read(mutationSource),PNG.sync.read(mutationClient)),mutationLayout=await viewer.evaluate(()=>window.mirror.renderer.layoutFidelity());
      await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-mutation-source.png`),mutationSource);await writeFile(path.join(root,'evidence',`${kind}-dpr${dpr}-mutation-client.png`),mutationClient);
      if(kind==='forms'){
        assert(!JSON.stringify(packet).includes('MASK-ME')&&!JSON.stringify(packet).includes('PROTECTED-FIXTURE'));
        const input=initial.nodes.find(n=>n.tag==='input'&&n.form);await viewer.evaluate(node_id=>window.mirror.input({kind:'text',node_id,text:'typed fixture'}),input.id);await nextPacket();
        const effect=await connection.send('Runtime.evaluate',{expression:'document.querySelector("#ordinary").value',returnByValue:true},sessionId);assert.equal(effect.result.value,'typed fixture');receipt.security.push({check:'protected fields/regions absent; ordinary text input',dpr,passed:true});
      }
      if(kind==='docs'){
        assert.equal(await viewer.evaluate(()=>{const doc=window.mirror.renderer.frame.contentDocument;return doc.defaultView.getComputedStyle(doc.querySelector('h1')).webkitTextStrokeWidth}),'1px','MP-08 sorted JSON preserves vendor paint styles');receipt.security.push({check:'sorted Rust-style maps preserve vendor text stroke paint',dpr,passed:true});
        const text=initial.nodes.find(n=>n.kind==='text'&&n.text.startsWith('Paragraph 1:'));
        await viewer.evaluate(id=>window.mirror.input({kind:'selection',anchor_id:id,anchor_offset:0,focus_id:id,focus_offset:9}),text.id);
        const note=await source.browser.observeNote({target_id:tab.target_id,document_id:tab.document_id});if(!note.selection){const diagnostic=await connection.send('Runtime.evaluate',{expression:'(()=>{const s=document.getSelection(),r=s.getRangeAt(0);return {ranges:s.rangeCount,collapsed:s.isCollapsed,length:s.toString().length,range_length:r.toString().length,a_type:s.anchorNode.nodeType,a_length:s.anchorNode.length,a_offset:s.anchorOffset,b_type:s.focusNode.nodeType,b_length:s.focusNode.length,b_offset:s.focusOffset,rects:r.getClientRects().length,range_width:r.getBoundingClientRect().width,composed:s.getComposedRanges().map(r=>({a:r.startContainer.nodeType,start:r.startOffset,b:r.endContainer.nodeType,end:r.endOffset}))}})()',returnByValue:true},sessionId);throw Error('MP-10: source note selection missing '+JSON.stringify(diagnostic.result.value));}assert.equal(note.selection.quote.exact,'Paragraph');await nextPacket();assert.equal(await viewer.evaluate(()=>window.mirror.renderer.frame.contentDocument.getSelection().toString()),'Paragraph');receipt.security.push({check:'source notes selection through stable mirrored text IDs',dpr,passed:true});
        await viewer.evaluate(()=>{window.mirror.renderer.frame.contentDocument.querySelector('p').style.transform='translateX(12px)'});const drifted=await nextPacket();assert(drifted.nodes.some(n=>n.reason==='layout_drift'&&n.kind==='tile'));receipt.security.push({check:'automatic geometric drift falls back to protected compositor tiles',dpr,passed:true});
        await viewer.evaluate(()=>{window.mirror.renderer.frame.contentDocument.querySelector('h1').style.color='rgb(255, 0, 255)'});const colorFallback=await nextPacket();assert(colorFallback.nodes.some(n=>n.reason==='layout_drift'));receipt.security.push({check:'exact-color drift falls back to protected compositor tiles',dpr,passed:true});
        await viewer.evaluate(()=>{window.mirror.renderer.frame.contentDocument.documentElement.style.transform='translateX(12px)'});const full=await nextPacket();assert(full.nodes.some(n=>n.reason==='observer_bounds_or_unavailable'));await nextPacket();receipt.security.push({check:'root drift restores a stable protected full-video mirror',dpr,passed:true});
      }
      if(kind==='long'){const visible=initial.nodes.find(n=>n.tag==='p'&&n.box.y>170&&n.box.y<300);await viewer.evaluate(id=>window.mirror.input({kind:'scroll',node_id:id,delta_x:0,delta_y:300}),visible.id);await new Promise(resolve=>setTimeout(resolve,60));const scrolled=await nextPacket();assert(scrolled.scroll.y>0);assert.equal(await viewer.evaluate(()=>window.mirror.renderer.frame.contentWindow.scrollY),scrolled.scroll.y);receipt.security.push({check:'source scrolling mirrored with canonical coordinates',dpr,passed:true});const deferred=initial.nodes.filter(n=>n.reason==='viewport_deferred').at(-20);assert(deferred,'MP-10 viewport-first bootstrap');await connection.send('Runtime.evaluate',{expression:`Array.from(document.querySelectorAll('p')).find(n=>n.textContent.startsWith('Paragraph 80:')).style.color='#090'`,returnByValue:true},sessionId);await nextPacket();await viewer.evaluate(()=>window.mirror.input({kind:'coordinate',input:{kind:'scroll',x:100,y:300,delta_x:0,delta_y:2400}}));await new Promise(resolve=>setTimeout(resolve,60));await nextPacket();const visibleColor=await viewer.evaluate(()=>{const p=[...window.mirror.renderer.records.values()].find(n=>n.kind==='text'&&n.text.startsWith('Paragraph 80:'));const node=window.mirror.renderer.dom.get(p.parent);return {color:node.ownerDocument.defaultView.getComputedStyle(node).color,y:node.getBoundingClientRect().y}});assert(visibleColor.y>=0&&visibleColor.y<800);assert.equal(visibleColor.color,'rgb(0, 153, 0)');receipt.security.push({check:'offscreen changed CSS is fully refreshed before scrolling into presentation',dpr,passed:true});}
      const stages=await viewer.evaluate(()=>window.mirror.renderer.timings);
      latencies.sort((a,b)=>a-b);receipt.rows.push({fixture:kind,dpr,bootstrap_ms:bootstrapMs,first_meaningful_paint_ms:firstMeaningfulPaintMs,bootstrap_bytes:bootstrapBytes,pixel_mse:mse,pixel_mismatch_fraction:mismatch,source_size:[sourcePixels.width,sourcePixels.height],client_size:[clientPixels.width,clientPixels.height],input_effect_p50_ms:latencies[4],input_effect_p95_ms:latencies[9],mutation_paint_ms:mutationMs,mutation_p50_ms:mutationLatencies[4],mutation_p95_ms:mutationLatencies[9],patch_bytes_mean:patchBytes.reduce((a,b)=>a+b,0)/patchBytes.length,client_stages:stages,trials,raster,mutation_raster:mutationRaster,mutation_layout:mutationLayout,mutation_batch_nodes:kind==='spa'?60:1,layout,settled_layout:settledLayout,hydration_ms:hydrationMs,hydration_bytes:hydrationBytes,fidelity_status:raster.outside_edge_mismatch_fraction<=0.001&&mutationRaster.outside_edge_mismatch_fraction<=0.001&&mutationLayout.max_error_css_px<=0.5&&mutationLayout.text_mismatches===0&&mutationLayout.color_mismatches===0&&settledLayout.max_error_css_px<=0.5&&textRunMaxError<=0.5&&settledLayout.text_mismatches===0&&settledLayout.color_mismatches===0?'PASS_GATE':'RED_FIDELITY',latency_status:latencies[4]<=80&&latencies[9]<=120&&mutationLatencies[4]<=80&&firstMeaningfulPaintMs<=500?'PASS_GATE':'RED_LATENCY',effects:10});
      await viewer.evaluate(()=>window.mirror.close());await source.request({op:'close',tab_id,generation,observed_by:'drill'});await resource();
    }
    // MP-11: trusted same-origin geometry must mask pixels inside another tile.
    {
      const opened=await source.request({op:'open',url:`${originUrl}/frames`,observed_by:'drill'}),tab=source.tabs.get(opened.tab_id),connection=await source.browser.ensureConnection(),sessionId=await source.browser.ensureTargetSession(connection,tab.target_id);
      await connection.send('Runtime.evaluate',{expression:`new Promise(resolve=>{const frame=document.querySelector('iframe');frame.onload=resolve;frame.src='/nested-protected';})`,awaitPromise:true,returnByValue:true},sessionId);
      const subscribed=await source.request({op:'mirror_subscribe',tab_id:tab.tab_id,generation:opened.generation,device_scale_factor:1,observed_by:'drill'});
      const packet=await source.request({op:'mirror_next',subscription_id:subscribed.subscription_id,generation:opened.generation,after_sequence:0,drift_nodes:[],observed_by:'drill'}),byId=new Map(packet.nodes.map(n=>[n.id,n]));
      assert(!JSON.stringify(packet).includes('SYNTHETIC-CARD'),'MP-11 nested protected form state absent');
      const field=packet.nodes.find(n=>n.kind==='mask'&&n.tag==='input');assert(field,'MP-11 uppercase payment field is protected');
      const globalBox={...field.box};for(let parent=byId.get(field.parent);parent;parent=byId.get(parent.parent))if(parent.kind==='frame'){globalBox.x+=parent.box.x+(parseFloat(parent.style?.['border-left-width'])||0);globalBox.y+=parent.box.y+(parseFloat(parent.style?.['border-top-width'])||0);}
      const {captureRegionMasks}=await import(path.join(root,'controller/kernel-browser-region-protection.mjs'));const masks=await captureRegionMasks(connection,sessionId,{mirrorStructured:true}),trusted=await masks.afterCapture();
      await writeFile(path.join(root,'evidence','protected-frame-geometry.json'),JSON.stringify({expected:globalBox,trusted}));
      assert(trusted.some(b=>['x','y','width','height'].every(key=>Math.abs(b[key]-globalBox[key])<=0.5)),'MP-11 trusted nested mask uses root compositor coordinates');
      const button=packet.nodes.find(n=>n.reason==='native_control'&&byId.get(n.parent)?.tag==='body'&&n.box.width===210),tile=packet.tiles.find(t=>t.node_id===button?.id);assert(tile,'MP-11 protected pixels inside native button tile');const pixels=PNG.sync.read(Buffer.from(tile.data_base64,'base64'));
      const x=Math.ceil(field.box.x-button.box.x-tile.x)+2,y=Math.ceil(field.box.y-button.box.y-tile.y)+2,w=Math.floor(field.box.width)-4,h=Math.floor(field.box.height)-4;
      assert(x>=0&&y>=0&&x+w<=pixels.width&&y+h<=pixels.height);for(let yy=y;yy<y+h;yy++)for(let xx=x;xx<x+w;xx++)for(let c=0;c<3;c++)assert.equal(pixels.data[(yy*pixels.width+xx)*4+c],0,'MP-11 masked payload pixels inside same-origin tile');
      receipt.security.push({check:'uppercase nested payment field absent; root-coordinate masks cover overlapping tile payload',passed:true});
      await viewer.goto(viewerUrl);await viewerCdp.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});await viewer.waitForFunction(()=>typeof window.start==='function');await viewer.evaluate(binding=>window.start(binding),{tab_id:tab.tab_id,generation:opened.generation,device_scale_factor:1});await viewer.evaluate(serial=>window.mirror.renderer.apply(JSON.parse(serial)),JSON.stringify(packet));
      const frame=packet.nodes.find(n=>n.kind==='frame');await connection.send('Runtime.evaluate',{expression:`document.querySelector('iframe').setAttribute('data-observation-protected','')`,returnByValue:true},sessionId);
      const opaque=await source.request({op:'mirror_next',subscription_id:subscribed.subscription_id,generation:opened.generation,after_sequence:packet.sequence,drift_nodes:[],observed_by:'drill'});assert(opaque.nodes.some(n=>n.id===frame.id&&n.kind==='mask'&&!n.children.length));assert(!JSON.stringify(opaque).includes('SYNTHETIC-CARD'));
      const protectedFrame=await captureRegionMasks(connection,sessionId,{mirrorStructured:true}),opaqueBounds=await protectedFrame.afterCapture();assert(opaqueBounds.some(b=>['x','y','width','height'].every(key=>Math.abs(b[key]-frame.box[key])<=0.5)),'MP-11 explicit frame marker retains its full trusted mask');
      await viewer.evaluate(serial=>window.mirror.renderer.apply(JSON.parse(serial)),JSON.stringify(opaque));const maskedLayout=await viewer.evaluate(()=>window.mirror.renderer.layoutFidelity());assert(maskedLayout.max_error_css_px<=0.5,'MP-10 protected inline frame placeholder preserves surrounding geometry');assert.equal(maskedLayout.text_mismatches,0);assert.equal(maskedLayout.color_mismatches,0);await viewer.evaluate(()=>window.mirror.close());
      receipt.security.push({check:'explicitly protected same-origin iframe retains full-region pixel mask, drops descendants and preserves surrounding layout',passed:true});
      await source.request({op:'close',tab_id:tab.tab_id,generation:opened.generation,observed_by:'drill'});
    }
    // MP-11: synthetic Vault policy exercises the actual isolated observer and
    // trusted protection hooks. No provider/profile/account credentials used.
    const opened=await source.request({op:'open',url:`${originUrl}/forms`,observed_by:'drill'}),tab=source.tabs.get(opened.tab_id),connection=await source.browser.ensureConnection(),sessionId=await source.browser.ensureTargetSession(connection,tab.target_id);
    await connection.send('Runtime.evaluate',{expression:`(()=>{const p=document.createElement('p');p.innerHTML='<span>SYNTHETIC-</span><span>VAULT-</span><span>MARKER</span>';document.querySelector('main').append(p);document.querySelector('main').setAttribute('title','SYNTHETIC-VAULT-MARKER');})()`,returnByValue:true},sessionId);
    await source.protect({values:['SYNTHETIC-VAULT-MARKER'],targets:[],unknown:false});
    const subscribed=await source.request({op:'mirror_subscribe',tab_id:tab.tab_id,generation:opened.generation,device_scale_factor:1,observed_by:'drill'});
    const masked=await source.request({op:'mirror_next',subscription_id:subscribed.subscription_id,generation:opened.generation,after_sequence:0,drift_nodes:[],observed_by:'drill'});
    assert(!JSON.stringify(masked).includes('SYNTHETIC-')&&!JSON.stringify(masked).includes('VAULT-')&&!JSON.stringify(masked).includes('MARKER'));assert(masked.nodes.some(n=>n.kind==='mask'));
    await source.protect({values:[],targets:[],unknown:true});await assert.rejects(source.request({op:'mirror_next',subscription_id:subscribed.subscription_id,generation:opened.generation,after_sequence:1,drift_nodes:[],observed_by:'drill'}),/registry unavailable/);
    receipt.security.push({check:'synthetic Vault split-text/attribute echoes absent; unknown observation registry fences DOM packets',passed:true});await source.protect({values:[],targets:[],unknown:false});await source.request({op:'close',tab_id:tab.tab_id,generation:opened.generation,observed_by:'drill'});
    assert.equal(forbiddenRequests,0);receipt.security.push({check:'no viewer origin requests; page world cannot forge isolated observer',passed:true});receipt.status=receipt.rows.every(r=>r.fidelity_status==='PASS_GATE'&&r.latency_status==='PASS_GATE')?'PASS_LOCAL_COMPONENT':'RED_GATE';process.exitCode=receipt.status==='PASS_LOCAL_COMPONENT'?0:1;
  }catch(error){receipt.failure={name:error.name,message:error.message,stack:error.stack};process.exitCode=1;}
  finally{
    await source.stop();await viewerHost.stop();await browser?.close().catch(()=>{});await closeServer(origin);await closeServer(serve);await cp(path.join(root,'home/source/display-timing.jsonl'),path.join(root,'evidence/host-timing.jsonl')).catch(()=>{});receipt.cleanup.push('source/viewer browser host shutdown; fixture listeners closed');await writeFile(path.join(root,'evidence/results.json'),JSON.stringify(receipt,null,2));
  }
}

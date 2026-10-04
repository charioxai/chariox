#!/usr/bin/env node
// MD-DISPLAY-02: no product protocol, identities, accounts or external relay.
import { createRequire } from 'node:module';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { fixture } from './fixtures.mjs';
import { compare, distribution } from './metrics.mjs';
import { display, headedBrowser, stopGroup, resources, guard, until, git, pause, requestStop } from './runtime.mjs';
import { baseline } from './selkies.mjs';
const require = createRequire(path.resolve(process.env.MD_TOOLS || '/root/.chariox/dev/browser-resume-20260930/agents/display/tools','package.json'));
const {chromium}=require('playwright-core'),{WebSocketServer}=require('ws'),{PNG}=require('pngjs');
const here=path.dirname(fileURLToPath(import.meta.url));
const output=path.resolve(process.env.MD_OUTPUT || `/root/.codex/evidence/browser-resume-20260930/display/run-${Date.now()}`);
if(output.startsWith(git('rev-parse','--show-toplevel')+'/'))throw Error('MD-DISPLAY evidence must be external');
await mkdir(output,{recursive:true});
const result={items:['MD-DISPLAY-01','MD-DISPLAY-02'],start:new Date().toISOString(),source:{commit:git('rev-parse','HEAD'),base:'9334141d420f8a32393f206102c5b8b4a1b0b609',dirty:git('status','--porcelain'),prototype_diff_sha256: null},topology:'headed host Chromium → CDP PNG → browser WebCodecs encoder → loopback plaintext WebSocket → browser decoder; isolated-world DOM → loopback WebSocket → scriptless iframe',scope:'credential-free display research; no production or hosted relay admission proof',runs:[],resource_samples:[],cleanup:{}};
const {createHash}=await import('node:crypto');result.source.prototype_diff_sha256=createHash('sha256').update(git('diff','HEAD')).digest('hex');
result.source.prototype_files={};for(const file of ['run.mjs','runtime.mjs','fixtures.mjs','mirror-source.js','metrics.mjs','viewer.html','selkies.mjs','package.json'])result.source.prototype_files[file]=createHash('sha256').update(await readFile(path.join(here,file))).digest('hex');
const servers=[],screens=[],browsers=[],roles=new Map(),events=[],groups=[];let monitor,sourcePage,sourceCDP,world,activeMode,pending=new Map(),received=new Map(),byteCount={},inputStats=[],installedBaseline,activeProbe,ingress=new Map(),ingressLatency=new Map();
const reportError=e=>events.push({kind:'harness-error',message:e.message});
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{requestStop();process.exitCode=130});
const deadline=setTimeout(()=>{requestStop();process.exitCode=124},15*60*1000);
const count=(key,n)=>{byteCount[key]=(byteCount[key]||0)+n};
const send=(role,value)=>{const ws=roles.get(role);if(!ws||ws.readyState!==1)return false;const data=typeof value==='string'||Buffer.isBuffer(value)?value:JSON.stringify(value);if(ws.bufferedAmount>4*1024*1024)throw Error('MD-DISPLAY bounded socket queue exceeded');ws.send(data);count(role,Buffer.byteLength(data));return true};
function pack(meta,bytes){const header=Buffer.from(JSON.stringify(meta)),length=Buffer.alloc(4);length.writeUInt32BE(header.length);return Buffer.concat([length,header,bytes])}
async function input(m){
  if(m.kind!=='input')return;
  if(m.action==='coordinate'){await click(m.x,m.y);return}
  const target=await evaluate(`mdMirror.target(${Number(m.id)})`);
  if(!target||target.password)throw Error('MD-DISPLAY invalid/stale/protected target');
  const x=target.x+target.width*(m.fx??.5),y=target.y+target.height*(m.fy??.5);
  await click(x,y);
  if(m.action==='fill'){
    if(!['input','textarea'].includes(target.tag)||typeof m.value!=='string'||m.value.length>1024)throw Error('MD-DISPLAY invalid fill');
    await sourceCDP.send('Input.dispatchKeyEvent',{type:'keyDown',key:'a',code:'KeyA',modifiers:2,windowsVirtualKeyCode:65});
    await sourceCDP.send('Input.dispatchKeyEvent',{type:'keyUp',key:'a',code:'KeyA',modifiers:2,windowsVirtualKeyCode:65});
    await sourceCDP.send('Input.insertText',{text:m.value});
  }
}
async function click(x,y){await sourceCDP.send('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await sourceCDP.send('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1})}
async function evaluate(expression){const r=await sourceCDP.send('Runtime.evaluate',{expression,contextId:world,returnByValue:true});if(r.exceptionDetails)throw Error('MD-DISPLAY isolated evaluation failed');return r.result.value}
async function listen(handler){const server=createServer(handler);await new Promise(r=>server.listen(0,'127.0.0.1',r));servers.push(server);return server}
const viewerHTML=await readFile(path.join(here,'viewer.html'));
const mirrorJS=await readFile(path.join(here,'mirror-source.js'),'utf8');
const documentText=await readFile(path.resolve('docs/PROTOCOL.md'),'utf8');
let otherOrigin;
const handler=(req,res)=>{if(req.url.startsWith('/viewer')){res.setHeader('Content-Type','text/html');res.end(viewerHTML);return}const page=fixture(req.url.split('?')[0],otherOrigin,documentText);res.setHeader('Content-Type','text/html');res.statusCode=page?200:404;res.end(page||'Not found')};
try{
  result.resource_samples.push(await resources());guard(result.resource_samples.at(-1));
  const server=await listen(handler),other=await listen(handler);otherOrigin=`http://127.0.0.1:${other.address().port}`;const origin=`http://127.0.0.1:${server.address().port}`;
  const wss=new WebSocketServer({server,path:'/socket',maxPayload:4*1024*1024});
  wss.on('connection',(ws,request)=>{const role=new URL(request.url,origin).searchParams.get('role');roles.set(role,ws);ws.on('message',(data,binary)=>{
    if(binary){if(role==='encoder'){count('encoded',data.length);send('video',data)}return}
    const m=JSON.parse(data);events.push({...m,role,at_ms:performance.now()});
    if(m.kind==='input'){if(pending.has(activeProbe)&&!ingress.has(activeProbe))ingress.set(activeProbe,performance.now());input(m).catch(reportError)}
    if(m.kind==='probe'){const t=pending.get(m.seq);if(t!==undefined){received.set(m.seq,performance.now()-t);if(ingress.has(m.seq))ingressLatency.set(m.seq,performance.now()-ingress.get(m.seq));pending.delete(m.seq)}}
    if(m.kind==='painted'&&role==='video')send('encoder',{kind:'ack',timestamp:m.timestamp});
  })});
  let sourceScreen;
  if(process.env.MD_BASELINE!=='1'){sourceScreen=await display();screens.push(sourceScreen);groups.push(sourceScreen.child.pid)}
  const viewerScreen=await display();screens.push(viewerScreen);groups.push(viewerScreen.child.pid);
  const sandbox=process.env.MD_NO_SANDBOX!=='1';
  const source=process.env.MD_BASELINE==='1'?await baseline(chromium,origin,output):await headedBrowser(chromium,output,'source',sourceScreen.number,sandbox);browsers.push(source);if(source.child)groups.push(source.child.pid);
  if(process.env.MD_BASELINE==='1'){installedBaseline=source;result.baseline=source.metadata;result.items.push('MD-DISPLAY-03');result.container_samples=[];result.topology='installed slice Xvfb/Chromium → existing read-only Selkies stdio adapter → loopback WebSocket → browser WebCodecs decoder; no hosted relay';}
  const viewer=await headedBrowser(chromium,output,'client',viewerScreen.number,sandbox);browsers.push(viewer);groups.push(viewer.child.pid);
  result.browser={version:source.browser.version(),sandbox,dpr:2,canonical_css:[960,600],physical:[1920,1200],source_is_headed:true,client_is_headed:true};
  const sc=source.browser.contexts()[0],vc=viewer.browser.contexts()[0];sourcePage=sc.pages()[0];sourceCDP=await sc.newCDPSession(sourcePage);
  await sourceCDP.send('Emulation.setDeviceMetricsOverride',{width:960,height:600,deviceScaleFactor:2,mobile:false});
  const encoder=await vc.newPage(),video=await vc.newPage(),dom=await vc.newPage();
  for(const p of [encoder,video,dom]){await p.setViewportSize({width:960,height:600});const c=await vc.newCDPSession(p);await c.send('Emulation.setDeviceMetricsOverride',{width:960,height:600,deviceScaleFactor:2,mobile:false})}
  const codec=process.env.MD_CODEC || 'avc1.420033';
  await encoder.goto(`${origin}/viewer?role=encoder&codec=${codec}`);await video.goto(`${origin}/viewer?role=video`);await dom.goto(`${origin}/viewer?role=dom`);
  for(const p of [encoder,video,dom])await p.waitForFunction(()=>window.md.ready);
  const probeSupport=await encoder.evaluate(async()=>Promise.all(['avc1.420033','vp09.00.10.08','av01.0.08M.08'].map(async codec=>({codec,supported:(await VideoEncoder.isConfigSupported({codec,width:1920,height:1200,bitrate:8000000,framerate:30,latencyMode:'realtime',hardwareAcceleration:'prefer-software'})).supported}))));result.codec_support=probeSupport;
  let sampling=false;
  monitor=setInterval(async()=>{if(sampling)return;sampling=true;try{const r=await resources(groups);guard(r);result.resource_samples.push(r);if(installedBaseline)result.container_samples.push({at_ms:performance.now(),...(await installedBaseline.sample())})}catch(e){reportError(e);process.kill(process.pid,'SIGTERM')}finally{sampling=false}},1000);
  const capture=async()=>Buffer.from((await sourceCDP.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,clip:{x:0,y:0,width:960,height:600,scale:1}})).data,'base64');
  const domCDP=await vc.newCDPSession(dom);
  const domSnapshot=async()=>Buffer.from((await domCDP.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false})).data,'base64');
  const videoSnapshot=async()=>Buffer.from((await video.evaluate(()=>mdCapture())).split(',')[1],'base64');
  const saveComparison=async(name,reference,actual)=>{await writeFile(path.join(output,`${name}-source.png`),reference);await writeFile(path.join(output,`${name}-viewer.png`),actual);const {diff,...metrics}=compare(reference,actual,PNG);if(diff)await writeFile(path.join(output,`${name}-diff.png`),diff);return metrics};
  async function installMirror(){const tree=await sourceCDP.send('Page.getFrameTree');world=(await sourceCDP.send('Page.createIsolatedWorld',{frameId:tree.frameTree.frame.id,worldName:'chariox-md-display-prototype',grantUniveralAccess:false})).executionContextId;await evaluate(mirrorJS)}
  let previous=new Map();
  async function mirror(force=false,patches=true){const snapshot=await evaluate(`mdMirror.snapshot(${force})`);if(!snapshot)return;const next=new Map(snapshot.records.map(r=>[r.id,JSON.stringify(r)])),changed=snapshot.records.filter(r=>previous.get(r.id)!==next.get(r.id)),removed=[...previous.keys()].filter(id=>!next.has(id));previous=next;
    send('dom',{kind:'dom',root:snapshot.root,changed,removed,scrollX:snapshot.scrollX,scrollY:snapshot.scrollY,timestamp:Math.round(performance.now()*1000)});
    if(patches&&snapshot.opaque.length){
      // Region screenshot commands temporarily change Chromium's capture surface.
      // One full screenshot and local crops avoid overlapping capture overrides.
      const full=PNG.sync.read(await capture());
      for(const r of snapshot.opaque){const x=Math.max(0,Math.round(r.x*2)),y=Math.max(0,Math.round(r.y*2)),width=Math.min(full.width-x,Math.round(r.width*2)),height=Math.min(full.height-y,Math.round(r.height*2));if(width<=0||height<=0)continue;const crop=new PNG({width,height});PNG.bitblt(full,crop,x,y,width,height,0,0);send('dom',pack({kind:'patch',id:r.id,timestamp:Math.round(performance.now()*1000)},PNG.sync.write(crop)))}
    }
    return {node_count:snapshot.records.length,changed:changed.length,opaque:snapshot.opaque};
  }
  const frameHandler=async f=>{try{count('cdp_png',Buffer.from(f.data,'base64').length);send('encoder',pack({kind:'frame',timestamp:Math.round(performance.now()*1000)},Buffer.from(f.data,'base64')));await sourceCDP.send('Page.screencastFrameAck',{sessionId:f.sessionId})}catch(e){reportError(e)}};
  for(const name of (process.env.MD_PAGES||'docs,spa,form,media,iframe').split(',')){
    await sourcePage.goto(`${origin}/${name}`);await sourcePage.bringToFront();await installMirror();await pause(300);
    for(const mode of installedBaseline?['selkies']:(process.env.MD_MODES||'screencast,capture,dom').split(',')){
      activeMode=mode;byteCount={};pending.clear();received.clear();ingress.clear();ingressLatency.clear();inputStats=[];events.length=0;previous=new Map();send('dom',{kind:'reset'});send('encoder',{kind:'reset-encoder'});send('video',{kind:'reset-video'});
      for(const p of [video,dom,encoder])await p.evaluate(()=>{Object.assign(md,{lastSeq:0,errors:[],outputs:0,inputs:0,dimensions:[],codecSupport:[]})});
      // Reset only the test probe through its actual source page state by reloading.
      await sourceCDP.send('Emulation.setDeviceMetricsOverride',{width:960,height:600,deviceScaleFactor:2,mobile:false});await sourcePage.reload();await installMirror();await pause(200);
      send('video',{kind:'probe-geometry',rect:await sourcePage.locator('#probe').boundingBox()});
      console.log(`MD-DISPLAY-02 start ${name}/${mode}`);
      const start=performance.now(),resourceBefore=await resources(groups),resourceIndex=result.resource_samples.length;let pump,working=false,lastSnapshot;
      const captureFrame=async()=>{if(working)return;working=true;try{send('encoder',pack({kind:'frame',timestamp:Math.round(performance.now()*1000)},await capture()))}catch(e){reportError(e)}finally{working=false}};
      const mirrorFrame=async()=>{if(working)return;working=true;try{lastSnapshot=await mirror(name==='media'||name==='iframe',true)}catch(e){reportError(e)}finally{working=false}};
      if(mode==='selkies'){await installedBaseline.start((meta,bytes)=>send('video',pack(meta,bytes)),n=>count('selkies_video',n))}
      else if(mode==='screencast'){sourceCDP.on('Page.screencastFrame',frameHandler);await sourceCDP.send('Page.startScreencast',{format:'png',maxWidth:1920,maxHeight:1200,everyNthFrame:1})}
      else if(mode==='capture'){await captureFrame();pump=setInterval(captureFrame,100)}
      else {await mirrorFrame();pump=setInterval(mirrorFrame,50)}
      const client=mode==='dom'?dom:video;await client.bringToFront();await pause(600);
      const step=await sourcePage.locator('#step').boundingBox();
      for(let seq=1;seq<=20;seq++){
        activeProbe=seq;
        pending.set(seq,performance.now());
        if(mode==='dom')await dom.frameLocator('#mirror').locator('[data-source-id="step"]').click();
        else await video.locator('canvas').click({position:{x:step.x+step.width/2,y:step.y+step.height/2}});
        try{await until(()=>received.has(seq),`${mode}/${name} probe ${seq}`,5000)}catch(e){events.push({kind:'probe-timeout',message:e.message,seq});break}
        await pause(50);
      }
      // Exercise source SPA handlers and native form input through the mirror.
      const interactions={};
      if(mode==='dom'&&name==='spa'){
        await dom.frameLocator('#mirror').locator('[data-source-id="add"]').click();await until(async()=>await sourcePage.locator('#rows tr').count()===4,'SPA add');
        await dom.frameLocator('#mirror').locator('[data-source-id="route"]').click();await until(async()=>await sourcePage.locator('#route-name').textContent()==='Archive','SPA route');interactions.spa_add_and_route=true;
      }
      if(mode==='dom'&&name==='form'){
        const field=dom.frameLocator('#mirror').locator('[data-source-id="name"]');await field.fill('Grace café 日本語');await until(async()=>await sourcePage.locator('#name').inputValue()==='Grace café 日本語','Unicode fill');
        await dom.frameLocator('#mirror').locator('[data-source-id="submit"]').click();await until(async()=>await sourcePage.locator('#result').textContent()==='Saved Grace café 日本語','native submit');interactions.unicode_fill_and_submit=true;
        interactions.password_value_masked=await dom.frameLocator('#mirror').locator('[data-source-id="private"]').inputValue()==='••••';
      }
      if(mode==='dom'&&name==='iframe'){
        const box=await sourcePage.frameLocator('#foreign').locator('#child-action').boundingBox(),frameBox=await sourcePage.locator('#foreign').boundingBox();
        await dom.frameLocator('#mirror').locator('[data-source-id="foreign"]').click({position:{x:box.x+box.width/2-frameBox.x,y:box.y+box.height/2-frameBox.y}});
        await until(async()=>await sourcePage.frameLocator('#foreign').locator('#child-result').textContent()==='Approved','cross-origin patch click');interactions.cross_origin_coordinate_replay=true;
      }
      // Freeze live canvas/video before pixel comparison, retaining the active run above.
      if(name==='media')await sourcePage.evaluate(()=>pauseMedia());
      clearInterval(pump);await until(()=>!working,'pump settled before fidelity capture');
      await pause(250);
      if(mode==='dom'){await mirror(true,true);await pause(250)}else if(mode==='capture')await captureFrame();
      const beforeOutputs=await video.evaluate(()=>md.outputs);await pause(200);
      const ref=await capture();await pause(200);const actual=mode==='dom'?await domSnapshot():await videoSnapshot();
      const fidelity=await saveComparison(`${name}-${mode}`,ref,actual),duration=(performance.now()-start)/1000;
      clearInterval(pump);await until(()=>!working,'pump settled');
      if(mode==='screencast'){await sourceCDP.send('Page.stopScreencast');sourceCDP.off('Page.screencastFrame',frameHandler)}
      if(mode==='selkies')await installedBaseline.stop();
      await encoder.evaluate(()=>mdFinish());await pause(150);
      const clientStats=await client.evaluate(()=>window.md),encoderStats=await encoder.evaluate(()=>window.md);
      const row={page:name,mode,status:received.size===20?'PASS_PROTOTYPE':'RED',codec:mode==='dom'?null:mode==='selkies'?installedBaseline.metadata.decoder_config?.codec:codec,duration_s:duration,bytes:byteCount,bytes_per_s:Object.fromEntries(Object.entries(byteCount).map(([k,v])=>[k,v/duration])),latency:distribution([...received.values()]),latency_definition:'trusted viewer click submission on harness clock → matching source probe painted/read back after viewer rAF; excludes physical monitor scanout',fidelity,interactions,mirror:lastSnapshot,client:clientStats,encoder:{codecSupport:encoderStats.codecSupport,dimensions:encoderStats.dimensions.slice(-5),errors:encoderStats.errors},errors:events.filter(e=>['error','harness-error','probe-timeout'].includes(e.kind))};
      const resourceAfter=await resources(groups),ticks=Math.max(0,resourceAfter.cpu_ticks-resourceBefore.cpu_ticks),sampled=result.resource_samples.slice(resourceIndex);
      row.latency_from_input_ingress=distribution([...ingressLatency.values()]);row.ingress_definition='harness receives viewer input to matching pixels/DOM after viewer rAF; excludes client uplink and physical scanout';
      row.resources={cpu_percent_of_one_core:100*ticks/100/((resourceAfter.monotonic_ms-resourceBefore.monotonic_ms)/1000),peak_owned_host_rss_bytes:Math.max(resourceBefore.rss_bytes,resourceAfter.rss_bytes,...sampled.map(s=>s.rss_bytes)),sample_interval_ms:1000,cpu_clock_ticks_per_second:100,definition:'owned host Chrome/Xvfb process groups plus harness; container cost is separately sampled for Selkies; exiting process CPU may be undercounted'};
      result.runs.push(row);await writeFile(path.join(output,'results.json'),JSON.stringify(result,null,2));console.log(`MD-DISPLAY-02/03 ${name}/${mode}: p50=${row.latency.p50_ms?.toFixed(1)}ms PSNR=${fidelity.psnr_db?.toFixed(1)||'lossless'} bytes/s=${Math.round(byteCount[mode==='dom'?'dom':mode==='selkies'?'selkies_video':'encoded']/duration)}`);
      if(row.errors.length||clientStats.errors.length){row.status='RED';process.exitCode=1}
    }
  }
  // Public documentation is supplemental: actual network page, not a fixture.
  if(!installedBaseline)try{
    await sourcePage.goto('https://chromedevtools.github.io/devtools-protocol/tot/Page/',{waitUntil:'domcontentloaded',timeout:30000});await installMirror();previous=new Map();send('dom',{kind:'reset'});await mirror(true,true);await dom.bringToFront();await pause(700);
    result.public_docs={url:sourcePage.url(),title:await sourcePage.title(),fidelity:await saveComparison('public-docs-dom',await capture(),await domSnapshot()),limitations:'no source input suite; external assets and pseudo-elements may drift'};
  }catch(e){result.public_docs={status:'RED',first_failing_seam:e.message}}
  result.status=process.exitCode?'RED':'PASS_PROTOTYPE';
}catch(e){result.status='RED';result.first_failing_seam=e.message;result.failure_events=events;result.failure_bytes=byteCount;
  if(sourcePage)try{result.failure_source_probe=await sourcePage.locator('#probe').getAttribute('data-seq');await sourcePage.screenshot({path:path.join(output,'failure-source.png'),timeout:5000})}catch{}
  for(const b of browsers)for(const p of b.browser.contexts()[0].pages())if(p.url().includes('/viewer'))try{const role=new URL(p.url()).searchParams.get('role');result[`failure_${role}`]=await p.evaluate(()=>md);await p.screenshot({path:path.join(output,`failure-${role}.png`),timeout:5000})}catch{}
  console.error(e.message);process.exitCode=1}
finally{
  clearInterval(monitor);clearTimeout(deadline);
  for(const b of browsers.reverse())try{await b.close()}catch(e){result.cleanup.browser_error=e.message;process.exitCode=1}
  for(const s of screens)await stopGroup(s.child);
  for(const ws of roles.values())ws.terminate();
  for(const s of servers)await new Promise(r=>s.close(r));
  const final=await resources(groups);result.resource_samples.push(final);result.cleanup.live_owned_processes=final.processes.filter(p=>p.state!=='Z'&&p.pid!==process.pid);result.cleanup.profiles_removed=true;result.cleanup.harness_pid_exits_with_command=process.pid;result.cleanup.servers_closed=servers.every(s=>!s.listening);
  if(result.cleanup.live_owned_processes.length){result.status='RED';process.exitCode=1}
  result.finish=new Date().toISOString();result.exit_code=process.exitCode||0;await writeFile(path.join(output,'results.json'),JSON.stringify(result,null,2));console.log(`MD-DISPLAY-02 evidence ${output} exit=${result.exit_code}`);
}

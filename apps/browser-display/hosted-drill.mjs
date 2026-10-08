// MP-08/MP-10: real built Cloud entry, hosted WSS and public sites.
// Headless/shaped b3 evidence remains separate from geographic desktop acceptance.
import {createRequire} from 'node:module';
import {mkdir,writeFile,readFile} from 'node:fs/promises';
import {sourceIdentity} from './source-identity.mjs';
import {driveHostedWheel} from './hosted-wheel.mjs';
import {shapeHostedViewer} from './hosted-network.mjs';
const [output,tools]=process.argv.slice(2),require=createRequire(tools+'/package.json');
const {chromium}=require('playwright-core');
const origin=process.env.MD_HOSTED_ORIGIN??'https://pr893-perf.val.51-255-87-178.sslip.io';
const dpr=Number(process.env.MD_DPR??2),r={MP:'MP-08/MP-10/MP-11',origin,dpr,status:'RED',...await sourceIdentity(),kernel_build_source:process.env.MD_KERNEL_SOURCE,cloud_source:process.env.MD_CLOUD_SOURCE,sites:[],errors:[],wire:[],resources:[]};
const sites=[['article','https://en.wikipedia.org/wiki/Computer'],['portal','https://www.wikipedia.org/'],['github','https://github.com/charioxai/chariox'],['google','https://www.google.com/search?q=chromium+display+transport'],['bbc','https://www.bbc.com/news'],['mdn','https://developer.mozilla.org/en-US/docs/Web/JavaScript']].filter(([site])=>!process.env.MD_SITES||process.env.MD_SITES.split(',').includes(site));
await mkdir(output,{recursive:true});let browser,page,shaped,stage='launch';
const sample=async()=>{const text=await readFile('/proc/meminfo','utf8');const mem=Number(text.match(/^MemAvailable:\s+(\d+)/m)[1])*1024;r.resources.push({at:Date.now(),mem_available:mem});if(mem<12*1024**3)throw Error('MP-10: memory floor')};
const save=()=>writeFile(output+'/results.json',JSON.stringify(r,null,2)+'\n');
try{
 await sample();if(process.env.MD_HOSTED_NETNS){shaped=await shapeHostedViewer({namespace:process.env.MD_HOSTED_NETNS});r.network=shaped.info}
 browser=await chromium.launch({headless:true,executablePath:'/opt/google/chrome/chrome',args:['--no-sandbox','--disable-dev-shm-usage',...(shaped?['--host-resolver-rules='+shaped.resolverRules]:[])]});
 const ctx=await browser.newContext({viewport:{width:1440,height:1200},deviceScaleFactor:dpr});page=await ctx.newPage();page.setDefaultTimeout(30000);
 r.bootstrap_protocol=[];page.on('response',async response=>{if(new URL(response.url()).pathname!=='/browser/relay-kernel/bootstrap')return;try{const value=await response.json();r.bootstrap_protocol.push({at:Date.now(),status:response.status(),protocol:value.target?.localDaemonProtocolVersion,daemon_id:value.target?.daemonId})}catch{r.bootstrap_protocol.push({at:Date.now(),status:response.status(),code:'unreadable'})}});
 page.on('pageerror',e=>r.errors.push({stage,code:e.name}));page.on('console',async msg=>{if(!msg.text().startsWith('[chariox:kernel-transport]'))return;let fields=await msg.args()[1]?.jsonValue().catch(()=>({}));if(!fields){try{fields=JSON.parse(msg.text().slice(msg.text().indexOf('{')))}catch{fields={}}}r.errors.push({event:msg.text().split(' ')[1],stage,request_kind:fields?.requestKind,code:fields?.code,rtt_ms:fields?.rttMs,lane:fields?.lane})});
 const cdp=await ctx.newCDPSession(page);r.wire_instrumentation=process.env.MD_WIRE==='1';if(r.wire_instrumentation)await cdp.send('Network.enable');if(process.env.MD_DIAGNOSTIC==='1'){r.debug_exceptions=[];await cdp.send('Debugger.enable');await cdp.send('Debugger.setPauseOnExceptions',{state:'all'});cdp.on('Debugger.paused',async event=>{const line=String(event.data?.description??'').split('\n')[0];r.debug_exceptions.push({reason:event.reason,labels:['kernel_browser','MD-DISPLAY:','MP-11:','stale browser','Failed to fetch dynamically imported module'].filter(label=>line.includes(label)),location:event.callFrames?.[0]?.location});await cdp.send('Debugger.resume').catch(()=>{})})}
 if(r.wire_instrumentation)cdp.on('Network.webSocketFrameReceived',e=>{if(r.wire.length<100000)r.wire.push({at:Date.now(),opcode:e.response.opcode,bytes:e.response.opcode===2?Math.floor(e.response.payloadData.length*3/4):e.response.payloadData.length})});
 await page.addInitScript(()=>{
  globalThis.mdHosted={frames:[],inputs:[]};
  const stats=globalThis.mdHosted;
  new MutationObserver(records=>{for(const record of records)if(record.attributeName==='data-display-sequence'){
   const c=record.target;const sequence=Number(c.dataset.displaySequence),kind=c.dataset.displayKind,width=c.width,height=c.height;requestAnimationFrame(at=>{if(stats.frames.length<100000)stats.frames.push({at:performance.timeOrigin+at,sequence,kind,width,height})});
  }}).observe(document,{subtree:true,attributes:true,attributeFilter:['data-display-sequence']});
  for(const kind of ['click','wheel','keydown','beforeinput'])document.addEventListener(kind,e=>{if(e.isTrusted&&(e.target instanceof HTMLCanvasElement||e.target instanceof HTMLTextAreaElement))stats.inputs.push({at:performance.timeOrigin+performance.now(),kind})},true);
 });
 stage='waiting room';await page.goto(origin+'/waiting-room');await page.getByText('Waiting Room Ready',{exact:true}).waitFor({timeout:60000});await page.waitForTimeout(20000);
 // MP-08/MP-10/MP-11: bootstrap receipt alone does not select the UI client.
 for(const [field,id] of [['Machine',process.env.MD_MACHINE_ID],['Kernel',process.env.MD_KERNEL_ID]]){
  if(!/^[a-zA-Z0-9_-]+$/.test(id??''))throw Error('MP-11: public enrollment identifier required');
  await page.getByRole('button',{name:new RegExp('^'+field)}).click();
  await page.locator(`[data-option-picker-value="${id}"]`).click({timeout:60000});
 }
 await page.waitForTimeout(5000);
 const expectedKernel=process.env.MD_KERNEL_ID;const target=r.bootstrap_protocol.find(t=>t.daemon_id===expectedKernel&&t.protocol>=466);if(!target)throw Error('MP-10: selected kernel protocol not ready');r.enrollment={machine_id:process.env.MD_MACHINE_ID,kernel_id:target.daemon_id,protocol:target.protocol,admission:'real application bootstrap; Cloud selects fresh heartbeat targets'};
 await page.waitForTimeout(2000);
 const session=page.getByText(process.env.MD_SESSION_ALIAS??'display-phase31',{exact:true});if(process.env.MD_OPEN_SESSION==='1'&&await session.count()===1){await session.click();await page.waitForTimeout(5000);r.opened_session=true}
 await page.getByRole('button',{name:'Open Browser panel',exact:true}).click();
 for(const [site,url]of sites){
  stage=site;const row={site,url,status:'RED',screenshots:[],clicks:[],typing:[]};r.sites.push(row);await sample();
  try{
   row.step='address';await page.getByRole('textbox',{name:'Browser address'}).fill(url);row.step='open';await page.getByRole('button',{name:await page.getByRole('button',{name:'Close tab',exact:true}).count()?'Go':'Open tab',exact:true}).click();
   row.step='tab';await page.getByRole('button',{name:'Close tab',exact:true}).waitFor({timeout:30000});
   // MP-08/MP-10: a prior surface stays mounted during real navigation. Wait
   // for the user-visible busy control to settle before measuring this site.
   row.step='navigation_ready';await page.waitForTimeout(100);
   await page.waitForFunction(()=>{const button=[...document.querySelectorAll('button')].find(b=>b.textContent==='Close tab');return button&&!button.disabled},null,{timeout:60000});
   row.step='presentation';const readiness=Date.now();await page.waitForFunction(()=>{const mode=document.querySelector('[data-paint-mode]')?.getAttribute('data-paint-mode');return mode==='mirror'||mode==='video'&&Boolean(document.querySelector('[data-display-sequence]'))||document.querySelector('.kernel-browser-fallback')?.textContent?.includes('Image fallback')},{},{timeout:60000});row.mode_ready_ms=Date.now()-readiness;
   row.mode=await page.locator('[data-paint-mode]').count()?await page.locator('[data-paint-mode]').getAttribute('data-paint-mode'):'image';
   row.alerts=await page.locator('[role=alert]').allTextContents();
   const mirrored=page.frames().find(f=>f!==page.mainFrame()&&f.name()==='');
   row.mirror_text_chars=mirrored?await mirrored.locator('body').innerText().then(t=>t.length).catch(()=>0):0;
   const screen=output+'/'+site+'-dpr'+dpr+'-initial.png';await page.screenshot({path:screen,fullPage:true});row.screenshots.push(screen);
   const canvas=page.getByRole('img',{name:'Kernel browser video'});
   if(row.mode==='video'){
    if(row.alerts.some(a=>/input unavailable/i.test(a)))throw Error('MP-10: input refused before measurement');
    const b=await canvas.boundingBox();if(!b)throw Error('MP-10: video canvas missing');
    await page.mouse.move(b.x+b.width*.5,b.y+b.height*.5);
    const begin=await page.evaluate(()=>performance.timeOrigin+performance.now());
    const driver=await driveHostedWheel(cdp,b);const wheelCalls=driver.sent;
    const end=await page.evaluate(()=>performance.timeOrigin+performance.now());await page.waitForTimeout(1000);
    const frames=await page.evaluate(({begin,end})=>mdHosted.frames.filter(f=>f.at>=begin&&f.at<=end),{begin,end});
    const visible=[...new Map(frames.map(frame=>[frame.at,frame])).values()];row.scroll={begin,end,driver,wheel_calls:wheelCalls,driver_hz:wheelCalls*1000/(end-begin),presented:visible.length,decoded:frames.length,fps:visible.length*1000/(end-begin),kinds:frames.reduce((o,f)=>(o[f.kind]=(o[f.kind]??0)+1,o),{})};
    row.final_canvas=await canvas.evaluate(c=>({width:c.width,height:c.height,kind:c.dataset.displayKind,sequence:c.dataset.displaySequence}));
   }
   const screen2=output+'/'+site+'-dpr'+dpr+'-after-scroll.png';await page.screenshot({path:screen2,fullPage:true});row.screenshots.push(screen2);row.alerts=await page.locator('[role=alert]').allTextContents();row.thresholds={full_dom_mirror:row.mode==='mirror',streamed_scroll:row.mode!=='video'||row.scroll?.fps>=30,input_echo:'UNMEASURED',product_screenshot:'UNMEASURED'};row.status=!['mirror','video'].includes(row.mode)?'RED_RENDER_NOT_READY':row.alerts.some(a=>/input unavailable/i.test(a))?'RED_INPUT':row.mode==='video'&&row.scroll?.fps<30?'RED_SCROLL':'PUBLIC_SITE_CAPTURED';if(row.status==='RED_INPUT')row.scroll={...row.scroll,valid:false,reason:'MP-10: refused input, not scroll performance'};row.visible_text=(await page.locator('[aria-label="Kernel browser"]').last().innerText()).slice(-1000);
  }catch(e){row.failedStage=stage;row.failed_step=row.step;row.errorCode=e.name;row.alerts=await page.locator('[role=alert]').allTextContents().catch(()=>[]);await page.screenshot({path:output+'/'+site+'-failure.png',fullPage:true}).then(()=>row.screenshots.push(output+'/'+site+'-failure.png')).catch(()=>{})}
  await save();shaped?.check();
 }
 r.http_rtt=[];for(let i=0;i<10;i++)r.http_rtt.push(await page.evaluate(async()=>{const t=performance.now();await fetch('/validation/ready',{cache:'no-store'});return performance.now()-t}));
 r.observations=await page.evaluate(()=>mdHosted);r.acceptance='NOT_ACCEPTED: pixel screenshots and cadence alone do not establish full DOM coverage, click/type echo or duration';r.status=r.sites.every(s=>s.status==='PUBLIC_SITE_CAPTURED')?'MEASURED_HOSTED_B3':'RED';if(r.status==='RED')process.exitCode=1;
}catch(e){r.failedStage=stage;r.errorCode=e.name;process.exitCode=1;if(page)await page.screenshot({path:output+'/failure.png',fullPage:true}).catch(()=>{})}
finally{
 if(page)await page.getByRole('button',{name:'Close tab',exact:true}).click({timeout:5000}).catch(()=>{});await browser?.close();if(shaped)try{r.network_statistics=await shaped.close()}catch(e){r.cleanup_error=e.name;r.status='RED_CLEANUP';process.exitCode=1}await save();console.log(JSON.stringify({MP:r.MP,status:r.status,sites:r.sites.map(s=>({site:s.site,status:s.status,mode:s.mode,fps:s.scroll?.fps})),failedStage:r.failedStage,errorCode:r.errorCode}));
}

// MP-08/MP-10/MP-11: supplementary real WebCodecs/pixel regressions, not live acceptance.
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const [tools,output,cloud,mode]=process.argv.slice(2);
const checkout=cloud??fileURLToPath(new URL('../../',import.meta.url));
const assets=cloud?path.join(cloud,'apps/web/src/kernel-browser/display-assets'):fileURLToPath(new URL('./',import.meta.url));
if(!tools||!output||!path.isAbsolute(output)||path.relative(checkout,output).split(path.sep)[0]!=='..')throw Error('MP-10: absolute Playwright and external evidence paths required');
const styles=cloud?(await readFile(path.join(cloud,'apps/web/src/styles/screenshot.ts'),'utf8')).split('`')[1]:'';
const identity={head:execFileSync('git',['rev-parse','HEAD'],{cwd:checkout,encoding:'utf8'}).trim(),dirty:Boolean(execFileSync('git',['status','--porcelain'],{cwd:checkout,encoding:'utf8'}).trim()),files:{}};
for(const name of ['presenter.mjs','video-sink.mjs','scroll-prediction.mjs'])identity.files[name]=createHash('sha256').update(await readFile(path.join(assets,name))).digest('hex');
const {chromium}=await import(tools);
const server=createServer(async(req,res)=>{
 try{
  const name=new URL(req.url,'http://localhost').pathname.slice(1);
  if(/^[a-z-]+\.mjs$/.test(name)){res.setHeader('content-type','text/javascript');res.end(await readFile(path.join(assets,name)));return;}
  res.setHeader('content-type','text/html');
  res.end(`<style>${styles}\n:root{--surface:rgb(31,37,43)}body{margin:0}#host{position:relative;width:1280px;height:800px}#display{width:1280px;height:800px}</style><div id="host"><canvas class="screenshot-kernel-browser-surface" id="display" width="1280" height="800"></canvas></div>`);
 }catch{res.statusCode=404;res.end();}
});
await mkdir(output,{recursive:true});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
let browser;const rows=[];
try{
 browser=await chromium.launch({executablePath:'/opt/google/chrome/chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage']});
 for(const dpr of [1,2])for(const native of [false,true])for(const push of [false,true]){
  const context=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:dpr});const page=await context.newPage();
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  const name=`dpr${dpr}-${native?'track':'canvas'}-${push?'push':'credit'}`;
  try{
   await page.goto(`http://127.0.0.1:${server.address().port}`);
   const result=await page.evaluate(async({dpr,native,push,mode})=>{
    const {attachBrowserDisplay}=await import('/presenter.mjs');
    if(!native)globalThis.MediaStreamTrackGenerator=undefined;
    const canvas=document.querySelector('#display'),originalBackground=canvas.style.background;
    let listener;const ops=[],encoded=[];
    const source=new OffscreenCanvas(1280,800),draw=source.getContext('2d');
    const encoder=new VideoEncoder({output:chunk=>{const data=new Uint8Array(chunk.byteLength);chunk.copyTo(data);encoded.push({data,key:chunk.type==='key'});},error:e=>{throw e;}});
    encoder.configure({codec:'vp8',width:1280,height:800,bitrate:2000000,framerate:60,latencyMode:'realtime'});
    for(let n=0;n<6;n++){
     draw.fillStyle=n<4?'rgb(0,255,0)':'rgb(0,0,255)';draw.fillRect(0,0,1280,800);
     draw.fillStyle='rgb(255,255,0)';draw.fillRect(0,100,1280,100);
     const image=new VideoFrame(source,{timestamp:n*16667});encoder.encode(image,{keyFrame:n===0||n===3||n===5});image.close();
    }
    await encoder.flush();encoder.close();
    let nextFrame;
    const transport={kernelProtocolVersion:push?475:466,displayEventEncoding:'CXD1',onEvent:fn=>{listener=fn;return()=>{};},
     ...(push?{subscribeDisplay:async()=>{},unsubscribeDisplay:async()=>{}}:{}),
     request:async({KernelBrowser:{command}})=>{
      ops.push(command);
      if(command.op==='display_next'){const frame=nextFrame;queueMicrotask(()=>listener({event:'kernel_browser_frame',subscription_id:'s',frame}));return {KernelBrowser:{result:{frame_sent:true}}};}
      return {KernelBrowser:{result:command.op==='display_subscribe'?{subscription_id:'s',generation:1}:command.op==='display_ack'?{push:'running'}:{}}};
     }};
    const shown=[];const stream=await attachBrowserDisplay(canvas,transport,{tab_id:'t',generation:1},{codec:'vp8',deviceScaleFactor:dpr,idleMs:10,scrollPredictionRegion:{x:0,y:0,width:1280,height:800},onPresented:f=>shown.push(f.sequence)});
    const common={...stream.binding,document_id:'d',width:1280*dpr,height:800*dpr,css_width:1280,css_height:800,device_scale_factor:dpr};
    const video=(n,sequence)=>({...common,...encoded[n],kind:'video',codec:'vp8',sequence});
    const wait=async(predicate)=>{const until=performance.now()+5000;while(!predicate()){if(performance.now()>until)throw Error('MP-10: presentation timed out');await new Promise(r=>setTimeout(r,5));}};
    const deliver=async(frame)=>{
     if(push){listener({event:'kernel_browser_frame',subscription_id:'s',frame});await wait(()=>stream.presenter.sequence===frame.sequence||stream.error);if(stream.error)throw stream.error;}
     else{nextFrame=frame;await stream.next();}
    };
    try{
     // Force the actual worker output to be hidden after decode, before rAF.
     const raf=window.requestAnimationFrame;let queued=0;
     if(!native&&mode!=='repair')window.requestAnimationFrame=()=>++queued;
     const first=deliver(video(0,1));first.catch(()=>{});
     if(!native&&mode!=='repair'){await wait(()=>queued>0);Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'));}
     await first;
     if(!native&&mode!=='repair'){
      await deliver(video(1,2));
      if(shown.length)throw Error('MP-10: hidden outputs reported as visible');
      delete document.hidden;window.requestAnimationFrame=raf;document.dispatchEvent(new Event('visibilitychange'));
     }else await deliver(video(1,2));
     await deliver(video(2,3));await new Promise(r=>setTimeout(r,80));
     if(stream.error||ops.some(o=>o.op==='display_ack'&&o.lost))throw Error('MP-10: hidden output caused transport recovery');
     // Leave the presenter alive for external screenshot pixel assertions.
     globalThis.regression={stream,canvas,deliver,video,originalBackground};
     return {sequence:stream.presenter.sequence,shown,track:Boolean(stream.presenter.videoSink.track),hiddenAdvanced:!native,ops:ops.map(({op,sequence,lost})=>({op,sequence,lost}))};
    }catch(error){await stream.close();throw error;}
   },{dpr,native,push,mode});
   // Screenshots test the CSS/compositor pixels, rather than sink.snapshot().
   const sample=async(label,expected)=>{
    const png=await page.screenshot({path:path.join(output,`${name}-${label}.png`)});
    const pixels=await page.evaluate(async base64=>{
     const image=await createImageBitmap(await (await fetch('data:image/png;base64,'+base64)).blob());
     const c=new OffscreenCanvas(image.width,image.height),ctx=c.getContext('2d');ctx.drawImage(image,0,0);image.close();
     return {patch:[...ctx.getImageData(4*devicePixelRatio,4*devicePixelRatio,1,1).data],band:[...ctx.getImageData(40*devicePixelRatio,50*devicePixelRatio,1,1).data],outside:[...ctx.getImageData(300*devicePixelRatio,300*devicePixelRatio,1,1).data]};
    },png.toString('base64'));
    rows.push({MP:'MP-08/MP-10/MP-11',name,label,pixels,...result,errors});
    for(const [key,channels]of Object.entries(expected))if(channels.some((value,i)=>Math.abs(pixels[key][i]-value)>30))throw Error(`MP-10: ${label} ${key} pixels ${pixels[key]}`);
   };
   await sample('key',{outside:[0,255,0],band:[0,255,0]});
   if(mode!=='repair'){
    await page.evaluate(()=>regression.stream.input({kind:'scroll',delta_x:0,delta_y:100}));
    await sample('predicted-key',{outside:[0,255,0],band:[255,255,0]});
   }
   await page.evaluate(()=>regression.deliver(regression.video(3,4)));
   await sample('authoritative-key',{outside:[0,255,0],band:[0,255,0]});
   await page.evaluate(async()=>{
    const c=new OffscreenCanvas(16,16),ctx=c.getContext('2d');ctx.fillStyle='red';ctx.fillRect(0,0,16,16);
    const data=new Uint8Array(await (await c.convertToBlob()).arrayBuffer());const {binding}=regression.stream;
    await regression.deliver({...binding,document_id:'d',kind:'tiles',sequence:5,base_sequence:4,width:1280*devicePixelRatio,height:800*devicePixelRatio,css_width:1280,css_height:800,device_scale_factor:devicePixelRatio,tiles:[{x:0,y:0,width:16,height:16,format:'png',data}]});
   });
   await sample('repair',{patch:[255,0,0],outside:[0,255,0]});
   if(mode!=='repair'){
    await page.evaluate(()=>regression.stream.input({kind:'scroll',delta_x:0,delta_y:100}));
    await sample('predicted-repair',{outside:[0,255,0],band:[255,255,0]});
   }
   await page.evaluate(()=>regression.deliver(regression.video(4,6)));
   await sample('delta',{patch:[255,0,0],outside:[0,0,255],band:[0,0,255]});
   await page.evaluate(()=>regression.deliver(regression.video(5,7)));
   await sample('final-key',{patch:[0,0,255],outside:[0,0,255]});
   await page.evaluate(async()=>{
    await regression.stream.close({preserveFrame:true});
    if(regression.canvas.style.background!==regression.originalBackground)throw Error('MP-10: original background not restored');
    // Real transparent canvas pixels catch source-over shift/restore trails.
    const {ScrollPrediction}=await import('/scroll-prediction.mjs');
    const c=new OffscreenCanvas(1280,800),ctx=c.getContext('2d');ctx.fillStyle='red';ctx.fillRect(0,0,1280,100);
    const original=ctx.getImageData(0,0,1280,800).data;
    const prediction=new ScrollPrediction(c,{x:0,y:0,width:1280,height:800});
    prediction.predict({kind:'scroll',delta_x:0,delta_y:-100},1);
    if(ctx.getImageData(4,150,1,1).data[3]!==255)throw Error('MP-10: transparent-canvas prediction absent');
    prediction.restore();
    const restored=ctx.getImageData(0,0,1280,800).data;
    if(restored.some((value,i)=>value!==original[i]))throw Error('MP-10: transparent-canvas restoration changed pixels');
   });
  }catch(error){rows.push({MP:'MP-08/MP-10/MP-11',name,error:error.message,errors});await page.screenshot({path:path.join(output,`${name}-failure.png`)}).catch(()=>{});}
  finally{await context.close();}
 }
}finally{
 await browser?.close();await new Promise(resolve=>server.close(resolve));
 await writeFile(path.join(output,'results.json'),JSON.stringify({MP:'MP-08/MP-10/MP-11',acceptance:false,checkout,assets,...identity,rows},null,2)+'\n');
}
console.log(JSON.stringify({MP:'MP-08/MP-10/MP-11',cells:8,failures:rows.filter(r=>r.error)}));
if(rows.some(r=>r.error||r.errors.length))process.exitCode=1;

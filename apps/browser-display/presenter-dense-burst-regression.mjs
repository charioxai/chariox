// MP-08/MP-10/MP-11: compositor regression, supplementary to the hosted live gate.
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const [tools,output,cloud]=process.argv.slice(2);
const checkout=cloud??fileURLToPath(new URL('../../',import.meta.url));
const assets=cloud?path.join(cloud,'apps/web/src/kernel-browser/display-assets'):fileURLToPath(new URL('./',import.meta.url));
if(!tools||!output||!path.isAbsolute(output)||path.relative(checkout,output).split(path.sep)[0]!=='..')throw Error('MP-10: absolute tools and external evidence paths required');
const styles=cloud?(await readFile(path.join(cloud,'apps/web/src/styles/screenshot.ts'),'utf8')).split('`')[1]:'';
const identity={head:execFileSync('git',['rev-parse','HEAD'],{cwd:checkout,encoding:'utf8'}).trim(),files:{}};
for(const name of ['presenter.mjs','video-sink.mjs'])identity.files[name]=createHash('sha256').update(await readFile(path.join(assets,name))).digest('hex');
const {chromium}=await import(tools);
const server=createServer(async(req,res)=>{
 try{
  const name=new URL(req.url,'http://localhost').pathname.slice(1);
  if(/^[a-z-]+\.mjs$/.test(name)){res.setHeader('content-type','text/javascript');res.end(await readFile(path.join(assets,name)));return;}
  res.end(`<style>${styles}\nbody{margin:0}#host{position:relative;width:1280px;height:800px}canvas{width:1280px;height:800px}</style><div id="host"><canvas class="screenshot-kernel-browser-surface" width="1280" height="800"></canvas></div>`);
 }catch{res.statusCode=404;res.end();}
});
await mkdir(output,{recursive:true});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const rows=[];let browser;
try{
 browser=await chromium.launch({executablePath:'/opt/google/chrome/chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage']});
 for(const dpr of [1,2])for(const native of [true,false]){
  const context=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:dpr}),page=await context.newPage(),errors=[];
  const name=`dpr${dpr}-${native?'track':'canvas'}`;page.on('pageerror',error=>errors.push(error.message));
  try{
   await page.goto(`http://127.0.0.1:${server.address().port}`);
   await page.evaluate(async({dpr,native})=>{
    const {BrowserDisplayPresenter}=await import('/presenter.mjs');
    if(!native)globalThis.MediaStreamTrackGenerator=undefined;
    const canvas=document.querySelector('canvas'),binding={tab_id:'t',generation:1,subscription_id:'s'};
    const presenter=new BrowserDisplayPresenter(canvas,binding),common={...binding,document_id:'d',width:1280*dpr,height:800*dpr,css_width:1280,css_height:800,device_scale_factor:dpr};
    const source=new OffscreenCanvas(1280,800),draw=source.getContext('2d'),encoded=[];
    const encoder=new VideoEncoder({output:chunk=>{const data=new Uint8Array(chunk.byteLength);chunk.copyTo(data);encoded.push({data,key:chunk.type==='key'});},error:error=>{throw error;}});
    encoder.configure({codec:'vp8',width:1280,height:800,framerate:60,bitrate:2000000,latencyMode:'realtime'});
    for(let n=0;n<=60;n++){
     draw.fillStyle=n===0?'rgb(0,255,0)':Math.floor((n-1)/20)%2?'rgb(255,0,0)':'rgb(0,0,255)';draw.fillRect(0,0,1280,800);
     // Dense 120 CSS-pixel displacement per 60Hz producer tick.
     draw.fillStyle='white';for(let y=-(n*120%240);y<800;y+=240)draw.fillRect(900,y,200,30);
     const frame=new VideoFrame(source,{timestamp:n*16667});encoder.encode(frame,{keyFrame:n===0});frame.close();
    }
    await encoder.flush();encoder.close();
    if(encoded.length!==61||encoded.slice(1).some(frame=>frame.key))throw Error('MP-10: dense regression requires only dependent video after the initial key');
    const video=n=>({...common,...encoded[n],kind:'video',codec:'vp8',sequence:n===0?1:n+3});
    await presenter.present(video(0));await new Promise(resolve=>setTimeout(resolve,100));
    const c=new OffscreenCanvas(16*dpr,16*dpr),ctx=c.getContext('2d');ctx.fillStyle='red';ctx.fillRect(0,0,c.width,c.height);
    const data=new Uint8Array(await (await c.convertToBlob()).arrayBuffer());
    await presenter.present({...common,kind:'tiles',sequence:2,base_sequence:1,tiles:[{x:0,y:0,width:c.width,height:c.height,format:'png',data}]});
    globalThis.dense={canvas,presenter,common,video,at:0,callbacks:0,unoccluded:0};
    const v=presenter.videoSink.video;
    if(native){
     if(v.tagName!=='VIDEO')throw Error('MP-10: native sink required');
     const observe=()=>{dense.callbacks++;if(getComputedStyle(canvas).opacity==='0')dense.unoccluded++;dense.callback=v.requestVideoFrameCallback(observe);};
     dense.callback=v.requestVideoFrameCallback(observe);
    }
   },{dpr,native});
   const screenshot=async(label)=>{
    const png=await page.screenshot({path:path.join(output,`${name}-${label}.png`)});
    return page.evaluate(async base64=>{
     const bitmap=await createImageBitmap(await (await fetch('data:image/png;base64,'+base64)).blob());
     const c=new OffscreenCanvas(bitmap.width,bitmap.height),ctx=c.getContext('2d');ctx.drawImage(bitmap,0,0);bitmap.close();
     return {outside:[...ctx.getImageData(300*devicePixelRatio,300*devicePixelRatio,1,1).data],patch:[...ctx.getImageData(4*devicePixelRatio,4*devicePixelRatio,1,1).data]};
    },png.toString('base64'));
   };
   const partial=await screenshot('partial-repair');
   if(partial.outside[1]<200||partial.patch[0]<200)throw Error('MP-10: partial repair occludes outside pixels');
   await page.evaluate(async()=>{
    const {presenter,common}=dense;
    await presenter.present({...common,kind:'tiles',sequence:3,base_sequence:2,moves:[[0,0,common.width,680*devicePixelRatio,-120*devicePixelRatio]],tiles:[]});
   });
   await screenshot('scroll-move');
   const samples=[];
   for(let block=0;block<3;block++){
    await page.evaluate(async()=>{
     for(let i=0;i<20;i++){const at=performance.now();await dense.presenter.present(dense.video(++dense.at));await new Promise(resolve=>setTimeout(resolve,Math.max(0,1000/60-(performance.now()-at))));}
     await new Promise(resolve=>setTimeout(resolve,100));
    });
    samples.push(await screenshot(`burst-${block}`));
   }
   const state=await page.evaluate(()=>{
    const {canvas,presenter,callbacks,unoccluded}=dense;
    const alpha=canvas.getContext('2d').getImageData(300*devicePixelRatio,300*devicePixelRatio,1,1).data[3];
    return {sequence:presenter.sequence,callbacks,unoccluded,opacity:getComputedStyle(canvas).opacity,alpha,overlayActive:presenter.videoSink.overlayActive};
   });
   rows.push({MP:'MP-08/MP-10/MP-11',name,state,samples,errors});
   if(native&&(state.callbacks<5||state.unoccluded<5))throw Error(`MP-10: dense burst presented ${state.callbacks} native callbacks but only ${state.unoccluded} unoccluded frames`);
   if(state.opacity!=='0'||state.alpha!==0||state.overlayActive)throw Error('MP-10: repair overlay did not return to transparent');
   for(let block=0;block<3;block++)if(samples[block].outside[block===1?0:2]<200)throw Error(`MP-10: composed burst ${block} shows stale repair pixels`);
  }catch(error){rows.push({MP:'MP-08/MP-10/MP-11',name,error:error.message,errors});}
  finally{await page.evaluate(()=>{if(globalThis.dense){dense.presenter.videoSink.video?.cancelVideoFrameCallback?.(dense.callback);dense.presenter.close();}}).catch(()=>{});await context.close();}
 }
}finally{await browser?.close();await new Promise(resolve=>server.close(resolve));await writeFile(path.join(output,'results.json'),JSON.stringify({MP:'MP-08/MP-10/MP-11',acceptance:false,...identity,rows},null,2)+'\n');}
console.log(JSON.stringify({MP:'MP-08/MP-10/MP-11',cells:4,failures:rows.filter(row=>row.error)}));
if(rows.some(row=>row.error||row.errors.length))process.exitCode=1;

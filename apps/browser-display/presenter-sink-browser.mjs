// MP-08/MP-10/MP-11: supplementary real-Chromium sink drill. This exercises
// WebCodecs, transferable streams, rAF and pixels; it is not hosted acceptance.
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {sourceIdentity} from './source-identity.mjs';
import path from 'node:path';
const [tools,output]=process.argv.slice(2);
if(!tools||!output||!path.isAbsolute(output))throw Error('MP-10: provide absolute Playwright module and external evidence directory');
const checkout=fileURLToPath(new URL('../../',import.meta.url));
const relative=path.relative(checkout,output);
if(relative!=='..'&&!relative.startsWith('..'+path.sep))throw Error('MP-10: evidence must be outside the checkout');
const {chromium}=await import(tools);
const server=createServer(async(req,res)=>{try{const name=new URL(req.url,'http://localhost').pathname.slice(1);if(!/^[a-z-]+\.mjs$/.test(name)){res.setHeader('content-type','text/html');res.end('<style>body{margin:0}div{width:1280px;height:800px;position:relative}canvas{width:100%;height:100%}</style><div><canvas id="display" width="1280" height="800"></canvas></div>');return;}res.setHeader('content-type','text/javascript');res.end(await readFile(new URL(name,import.meta.url)))}catch{res.statusCode=404;res.end()}});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));await mkdir(output,{recursive:true});
let browser;const rows=[],identity=await sourceIdentity();
try{
 browser=await chromium.launch({executablePath:'/opt/google/chrome/chrome',headless:true,args:['--no-sandbox','--disable-dev-shm-usage']});
 for(const dpr of [1,2])for(const native of [true,false]){
  const context=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:dpr});const page=await context.newPage();
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  try{
   await page.goto('http://127.0.0.1:'+server.address().port);
   const result=await page.evaluate(async({native,dpr})=>{
    if(!native)globalThis.MediaStreamTrackGenerator=undefined;
    const {BrowserDisplayPresenter}=await import('/presenter.mjs');
    const {PresentationQueue}=await import('/presentation-queue.mjs');
    const canvas=document.querySelector('canvas'),binding={subscription_id:'s',generation:1,tab_id:'t'};
    const presenter=new BrowserDisplayPresenter(canvas,binding),queue=new PresentationQueue(presenter),encoded=[];
    const source=new OffscreenCanvas(1280,800),draw=source.getContext('2d');
    const encoder=new VideoEncoder({output:chunk=>{const bytes=new Uint8Array(chunk.byteLength);chunk.copyTo(bytes);encoded.push({data:bytes,key:chunk.type==='key'})},error:e=>{throw e}});
    encoder.configure({codec:'vp8',width:1280,height:800,bitrate:2000000,framerate:60,latencyMode:'realtime'});
    for(let n=0;n<62;n++){draw.fillStyle=n>=59?'rgb(0,255,0)':`rgb(${n*4},0,0)`;draw.fillRect(0,0,1280,800);const f=new VideoFrame(source,{timestamp:n*16667});encoder.encode(f,{keyFrame:n===0||n===61});f.close();}await encoder.flush();encoder.close();
    const timings=[],drawn=[];
    for(let n=0;n<60;n+=3){
     const jobs=[];
     for(let i=n;i<n+3;i++){
      const frame={...binding,...encoded[i],sequence:i+1,document_id:'d',kind:'video',codec:'vp8',width:1280*dpr,height:800*dpr,css_width:1280,css_height:800,device_scale_factor:dpr};
      jobs.push(queue.submit(frame,async()=>{await presenter.present(frame);const o=presenter.outcomes.get(frame);if(o?.drawn){drawn.push(frame.sequence);timings.push(o.committedAt-o.decodedAt)}}));
     }
     // The worker decodes while the page is busy, as in a rendering UI.
     const until=performance.now()+8;while(performance.now()<until){}
     await Promise.all(jobs);await new Promise(resolve=>setTimeout(resolve,16));
    }
    await new Promise(resolve=>setTimeout(resolve,100));
    const track=Boolean(presenter.videoSink.track),video=presenter.videoSink.video;
    const raster=new OffscreenCanvas(1,1),r=raster.getContext('2d');r.drawImage(video??canvas,0,0,1,1);const pixel=[...r.getImageData(0,0,1,1).data];
    timings.sort((a,b)=>a-b);const result={nativeRequested:native,track,decoded:60,committed:drawn.length,lastSequence:presenter.sequence,decodeCommitMedian:timings[Math.floor(timings.length/2)],decodeCommitP95:timings[Math.ceil(timings.length*.95)-1],decodeCommitMax:timings.at(-1),pixel};
    const tile=new OffscreenCanvas(8,8);tile.getContext('2d').fillStyle='red';tile.getContext('2d').fillRect(0,0,8,8);
    const common={...binding,document_id:'d',width:1280*dpr,height:800*dpr,css_width:1280,css_height:800,device_scale_factor:dpr};
    const data=new Uint8Array(await (await tile.convertToBlob({type:'image/png'})).arrayBuffer());
    await presenter.present({...common,sequence:61,base_sequence:60,kind:'tiles',tiles:[{x:0,y:0,width:8,height:8,format:'png',data}]});
    const sample=()=>{const c=new OffscreenCanvas(canvas.width,canvas.height),ctx=c.getContext('2d');presenter.videoSink.snapshot(ctx);return {repair:[...ctx.getImageData(2,2,1,1).data],outside:[...ctx.getImageData(100,100,1,1).data]}};
    result.afterRepair=sample();
    await presenter.present({...common,...encoded[60],sequence:62,kind:'video',codec:'vp8'});
    await new Promise(resolve=>setTimeout(resolve,50));result.afterDelta=sample();
    await presenter.present({...common,...encoded[61],sequence:63,kind:'video',codec:'vp8'});
    await new Promise(resolve=>setTimeout(resolve,50));result.afterKey=sample();
    if(result.afterRepair.repair[0]<200||result.afterRepair.outside[1]<200||result.afterDelta.repair[1]<200||result.afterKey.repair[1]<200)throw Error('MP-10: exact overlay/key retirement failed');
    globalThis.sinkPresenter=presenter;return result;
   },{native,dpr});
   await page.screenshot({path:output+`/dpr${dpr}-${native?'track':'canvas'}.png`});
   rows.push({MP:'MP-08/MP-10/MP-11',dpr,...result,errors});
   await page.evaluate(()=>sinkPresenter.close({preserveFrame:true}));
   if(result.lastSequence!==60||result.pixel[1]<200||result.pixel[0]>40||errors.length)throw Error('MP-10: sink pixels/sequence failed');
  }finally{await context.close();}
 }
}finally{await browser?.close();await new Promise(resolve=>server.close(resolve));await writeFile(output+'/results.json',JSON.stringify({MP:'MP-08/MP-10/MP-11',acceptance:false,...identity,checkout,rows},null,2)+'\n');console.log(JSON.stringify(rows));}

// MD-DISPLAY-02/04: isolate headed CDP compression cost; no relay acceptance claim.
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { mkdtemp, mkdir, readFile, writeFile, chmod, chown, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fixture } from './drill-fixtures.mjs';
import { compare, distribution } from './drill-metrics.mjs';
import { decodePng } from '../kernel/slice-linux-docker/docker/kernel-browser-pixels.mjs';
import { launchOwned, stopGroup, checkChild } from './drill-owned-process.mjs';
const [output, tools] = process.argv.slice(2);
if (![output, tools].every(value => value && path.isAbsolute(value))) throw Error('MD-DISPLAY: absolute evidence and public tools paths required');
const require = createRequire(path.join(tools,'package.json'));
const { chromium } = require('playwright-core'), { PNG } = require('pngjs');
const receipt = { item:'MD-DISPLAY-02/04', status:'RED', methods:[], cleanup:[] };
const root = await mkdtemp(path.join(tmpdir(),'chariox-md-display-capture-'));
let display, viewer, browser, server;
const pause = ms => new Promise(resolve=>setTimeout(resolve,ms));
async function until(check) { for(let i=0;i<400;i++){const result=await check();if(result)return result;await pause(25)}throw Error('MD-DISPLAY: owned launch timeout'); }
try {
 await mkdir(output,{recursive:true});await chmod(root,0o755);
 const home=path.join(root,'profile');await mkdir(home,{mode:0o700});await chown(home,65534,65534);
 display=await launchOwned('/usr/bin/Xvfb',['-displayfd','3','-screen','0','2560x1600x24','-nolisten','tcp','-ac'],{detached:true,stdio:['ignore','ignore','ignore','pipe']});
 let screen='';display.stdio[3].on('data',b=>screen+=b);await until(()=>{checkChild(display);return screen.includes('\n')});
 const text=await readFile(new URL('../../docs/MULTIDOMAIN_KERNEL_BROWSER.md',import.meta.url),'utf8');
 server=createServer((req,res)=>{res.setHeader('Content-Type','text/html');res.end(fixture('/docs',`http://127.0.0.1:${server.address().port}`,text))});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 viewer=await launchOwned('/usr/bin/google-chrome',['--remote-debugging-port=0',`--user-data-dir=${home}`,'--no-first-run','--disable-background-networking',...(process.env.MD_CAPTURE_GPU==='0'?['--disable-gpu']:[]),...(process.env.MD_CAPTURE_UNLIMITED==='1'?['--disable-frame-rate-limit']:[]),'about:blank'],{uid:65534,gid:65534,detached:true,cwd:root,env:{PATH:'/usr/bin:/bin',HOME:home,TMPDIR:home,DISPLAY:`:${screen.trim()}`},stdio:'ignore'});
 const port=await until(async()=>{checkChild(viewer);try{return Number((await readFile(path.join(home,'DevToolsActivePort'),'utf8')).split('\n')[0])}catch{return null}});
 browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
 const page=await browser.contexts()[0].newPage();await page.goto(`http://127.0.0.1:${server.address().port}/docs`);
 const cdp=await page.context().newCDPSession(page);await cdp.send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:2,mobile:false});await pause(200);
 let reference;
 for(const settings of [{optimizeForSpeed:false},{optimizeForSpeed:true},{optimizeForSpeed:true,clip:{x:0,y:0,width:1280,height:800,scale:0.5}},{optimizeForSpeed:true,clip:{x:1120,y:0,width:160,height:64,scale:1}}]) {
  const captures=[],decodes=[],sizes=[];
  for(let i=0;i<12;i++) {
   let at=performance.now();const {data}=await cdp.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,...settings});captures.push(performance.now()-at);
   at=performance.now();decodePng(data,2);decodes.push(performance.now()-at);sizes.push(Buffer.from(data,'base64').length);
   if(!reference)reference=Buffer.from(data,'base64');
   const metric=compare(reference,Buffer.from(data,'base64'),PNG);if(!settings.clip&&!metric.lossless)throw Error('MD-DISPLAY: capture-speed setting changed RGB');
   if(i===0)await writeFile(path.join(output,`method-${receipt.methods.length}.png`),Buffer.from(data,'base64'));
  }
  receipt.methods.push({settings,capture:distribution(captures),decode:distribution(decodes),png_bytes:sizes,scope:settings.clip?'preview/crop geometry only':'full RGB exact'});
 }
 receipt.status='PASS_CAPTURE_COMPONENT';
}catch(error){receipt.error=error.message;process.exitCode=1;}
finally {
 try{await browser?.close();await stopGroup(viewer);await stopGroup(display);await rm(root,{recursive:true,force:true});receipt.cleanup.push('exact owned process groups and scratch removed');}catch(error){receipt.status='RED';receipt.cleanup.push(error.message);process.exitCode=1;}
 if(server)await new Promise(resolve=>server.close(resolve));
 await mkdir(output,{recursive:true});await writeFile(path.join(output,'results.json'),JSON.stringify(receipt,null,2));console.log(JSON.stringify({item:receipt.item,status:receipt.status,methods:receipt.methods.map(m=>({settings:m.settings,capture_p50:m.capture.p50_ms,decode_p50:m.decode.p50_ms,bytes:m.png_bytes[0]}))}));
}

// MP-08/MP-10/MP-11: real Chromium/private region producer qualification.
// No public consumer subscription, relay, actor or Vault acceptance inferred.
import {mkdir,writeFile,rm} from 'node:fs/promises';import {setTimeout as delay} from 'node:timers/promises';
import assert from 'node:assert/strict';import {createReadStream} from 'node:fs';import {createHash} from 'node:crypto';
const [,,root,E]=process.argv;
if(!/^\/root\/\.chariox\/dev\/vidlat\/r\d+\/region-live-[\w-]+$/.test(root??'')||
 !/^\/root\/\.codex\/evidence\/browser-resume-20260930\/vidlat\/r\d+\/region-live-[\w-]+$/.test(E??'')||
 !/^[a-f0-9]{40}$/.test(process.env.OSS_SOURCE??'')||!/^[a-f0-9]{40}$/.test(process.env.NATIVE_WORKER_SOURCE??'')||!process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER?.startsWith('/'))throw Error('MP-11 own region roots and exact source/worker identities required');
const workerHash=createHash('sha256');for await(const bytes of createReadStream(process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER))workerHash.update(bytes);
const kernelHash=workerHash.digest('hex');
const {KernelBrowserHost}=await import('../kernel/slice-linux-docker/docker/kernel-browser-host.mjs');
const {NativeWindowRegionCapture}=await import('../kernel/slice-linux-docker/docker/kernel-browser-window-region.mjs');
const {PortableEncoder}=await import('../kernel/slice-linux-docker/docker/kernel-browser-display.mjs');
const {nativeWindowInventory}=await import('../kernel/slice-linux-docker/docker/kernel-browser-native-windows.mjs');
process.env.CHARIOX_HOME=root+'/home';process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=root;
await mkdir(root,{mode:0o700});await mkdir(E,{recursive:true});
const report={items:['MP-08','MP-10','MP-11'],source:process.env.OSS_SOURCE,native_worker_source:process.env.NATIVE_WORKER_SOURCE,kernel_sha256:kernelHash,qualification:'Real sandboxed Chromium launched by KernelBrowserHost, compiled private native worker, crop source/codec/exact/retirement only. Public client/relay and provider/Vault flows NOT exercised.',cells:[]};let host,source,encoder,held;
try{
 host=new KernelBrowserHost(root);await host.start();const opened=await host.open('https://en.wikipedia.org/wiki/Web_browser');await delay(6000);
 const tab=await host.displayTarget({tab_id:opened.tab_id,generation:host.generation});
 const {connection,sessionId}=await host.browser.resolvePageTarget(tab.target_id);const {windowId}=await connection.send('Browser.getWindowForTarget',{targetId:tab.target_id});
 await connection.send('Browser.setWindowBounds',{windowId,bounds:{windowState:'normal'}});await connection.send('Browser.setWindowBounds',{windowId,bounds:{width:1280,height:887}});await connection.send('Page.bringToFront',{},sessionId);await delay(1000);
 host.scales.set(tab.tab_id,2);let current=true;const windows=await nativeWindowInventory({display:host.chromium.display,pid:host.chromium.child.pid,isCurrent:()=>current});
 const owned=windows.filter(w=>w.visible&&w.width===2560&&w.height===1774);assert.equal(owned.length,1);report.window_inventory=windows;
 const {targetId:duplicate}=await connection.send('Target.createTarget',{url:'https://www.wikipedia.org/',newWindow:true,background:true});
 const {windowId:duplicateWindow}=await connection.send('Browser.getWindowForTarget',{targetId:duplicate});
 await connection.send('Browser.setWindowBounds',{windowId:duplicateWindow,bounds:{windowState:'normal'}});await connection.send('Browser.setWindowBounds',{windowId:duplicateWindow,bounds:{width:1280,height:887}});await delay(1000);
 await connection.send('Page.bringToFront',{},sessionId);
 const duplicateInventory=await nativeWindowInventory({display:host.chromium.display,pid:host.chromium.child.pid,isCurrent:()=>current});report.duplicate_inventory=duplicateInventory;assert.equal(duplicateInventory.filter(w=>w.width===2560&&w.height===1774).length,2);
 const common={display:host.chromium.display,pid:host.chromium.child.pid,connection,sessionId,tab,scale:2,hostScale:2,window:{id:owned[0].id,width:2560,height:1774},contentHeight:1600,isCurrent:()=>current,policy:host.protection,browser:host.browser,frames:id=>host.browser.frameSession(id,connection),screenshot:()=>host.displayScreenshot(tab,null,false),allowed:p=>p===host.protection&&!p.unknown};
 for(const [surface,rect]of [['browser_chrome',{left:0,top:0,width:2560,height:174}],['page',{left:40,top:214,width:640,height:400}]]){
  const row={surface,rect,status:'RUNNING'};report.cells.push(row);
  try{source=new NativeWindowRegionCapture({surface,rect,...common});await source.start();const sample=source.sample();assert(sample?.raw?.nativeEncode);held=sample.raw;held.retain();assert.equal(sample.width,rect.width);assert.equal(sample.height,rect.height);row.attested=true;
   encoder=new PortableEncoder();const encoded=await encoder.encode(sample.raw,4000000,true,'avc1.420033');row.codec=encoder.backend;row.key=encoded.key;row.packet_length=encoded.packet?.length;assert(encoded.packet||encoded.data_base64);assert(!encoded.dropped);
   const exact=await sample.raw.nativeExact({encoder:encoder.nativeSession,regions:[],patch:false});assert.equal(typeof exact.data_base64,'string');await writeFile(E+'/'+surface+'-exact.png',Buffer.from(exact.data_base64,'base64'));row.exact=true;
   if(surface==='browser_chrome'){assert(source.chord(0x6c,2));await delay(400);for(const ch of 'https://www.wikipedia.org/')assert(source.chord(ch.charCodeAt(0),0));await delay(400);const selected=source.sample();const shot=await selected.raw.nativeExact({encoder:encoder.nativeSession,regions:[],patch:false});await writeFile(E+'/chrome-omnibox-typed.png',Buffer.from(shot.data_base64,'base64'));report.omnibox_inventory=await nativeWindowInventory({display:host.chromium.display,pid:host.chromium.child.pid,isCurrent:()=>current});assert(source.chord(0xff1b,0));}
   if(surface==='browser_chrome'){
    // MP-11: with every raster leased, input can beat the next readback.
    // Actual Chrome resize must retire the old binding at dispatch too.
    const leases=new Map(),keep=s=>{if(!leases.has(s.serial)){s.raw.retain();leases.set(s.serial,s.raw);}};
    keep({serial:held.serial,raw:held});keep(source.sample());const off=source.subscribe(keep);
    try{
     for(let n=0;n<100&&leases.size<6;n++){source.wake(true);await delay(50);}
     off();assert.equal(leases.size,6);row.resize_held_slots=leases.size;
     await connection.send('Browser.setWindowBounds',{windowId,bounds:{width:1280,height:888}});await delay(100);
     const changed=await nativeWindowInventory({display:host.chromium.display,pid:host.chromium.child.pid,isCurrent:()=>current});
     assert.equal(changed.find(w=>w.id===owned[0].id)?.height,1776);
     row.resize_input_dispatched=await source.click(450,70);assert.equal(row.resize_input_dispatched,false);
     for(const raw of leases.values())raw.release();leases.clear();await delay(300);assert.equal(source.valid(),false);
    }finally{off();for(const raw of leases.values())raw.release();await connection.send('Browser.setWindowBounds',{windowId,bounds:{width:1280,height:887}});await delay(200);}
   }
   current=false;assert.equal(source.valid(),false);assert.equal(source.chord(0x6c,2),false);row.retired=true;row.status='PRODUCER_QUALIFIED_NOT_ACCEPTED';
  }catch(e){row.status='FAIL';row.error=String(e.message).replace(/https?:\/\/\S+/g,'[URL]').slice(0,180);}
  finally{held?.release();held=null;await encoder?.close();encoder=null;await source?.close();source=null;current=true;await writeFile(E+'/report.json',JSON.stringify(report,null,2));}
 }
}catch(e){report.error=String(e.message).replace(/https?:\/\/\S+/g,'[URL]').slice(0,180);}finally{await encoder?.close();await source?.close();await host?.stop();await writeFile(E+'/report.json',JSON.stringify(report,null,2));await rm(root,{recursive:true,force:true});}
process.exitCode=report.error||report.cells.some(c=>c.status==='FAIL')?1:0;
console.log(JSON.stringify({items:report.items,cells:report.cells.map(c=>({surface:c.surface,status:c.status,error:c.error})),error:report.error}));

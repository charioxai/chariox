// MD-N5 / MP-10: credential-free native stimulus; never used for product admission.
import { writeFile, mkdir, symlink } from 'node:fs/promises';
import path from 'node:path';
import { BrowserCdpClient } from './browser-controller-cdp.mjs';
import { HostChromium } from './kernel-browser-process.mjs';
import { KernelBrowserHost } from './kernel-browser-host.mjs';
import { connectDrillCdp, serveDrillCdp } from './notes-drill-cdp.mjs';
import { BrowserControllerStdioServer } from './browser-controller.mjs';
import { observeBrowserResources } from './browser-controller-resources.mjs';

const [mode,rootArgument,endpointOrUrl,operation]=process.argv.slice(2);
const hostMode=mode==='stdio' && path.isAbsolute(rootArgument??'');
const root=hostMode?rootArgument:mode==='stdio'?path.join(process.env.CHARIOX_MDNOTES_DRILL_ROOT,'room-browser'):rootArgument;
if (!['room','stimulus','stdio'].includes(mode) || !path.isAbsolute(root)) throw new Error('MD-N5: invalid owned drill arguments');
let chromium;
let browser;
let closeProxy;
if(hostMode){
 const host=new KernelBrowserHost(root);
 let proxyConnection;
 const stop=async()=>{await closeProxy?.();await host.stop()};
 process.on('SIGTERM',()=>void stop().finally(()=>process.exit(0)));
 process.on('SIGINT',()=>void stop().finally(()=>process.exit(0)));
 await new BrowserControllerStdioServer({handleRequest:async(request,options)=>{
  const response=await host.handle(request,options);
  if(proxyConnection!==host.chromium.connection){
   await closeProxy?.();closeProxy=null;proxyConnection=host.chromium.connection;
   if(host.browser)closeProxy=await serveDrillCdp(root,proxyConnection);
  }
  return response;
 }}).run().finally(stop);
} else try {
  let connection;
  if (mode==='room') {
    chromium=new HostChromium(root);
    connection=await chromium.start();
    closeProxy=await serveDrillCdp(root,connection);
  } else connection=await connectDrillCdp(root);
  browser=new BrowserCdpClient({connectionFactory:()=>connection});
  await browser.ensureConnection();
  if (mode==='room') {
    const {targetId}=await connection.send('Target.createTarget',{url:endpointOrUrl});
    const session=await browser.ensureTargetSession(connection,targetId);
    await connection.send('Runtime.evaluate',{expression:'document.readyState',returnByValue:true},session);
    // A local fixture lacks the worker PID namespace. Observe only the actual
    // child we launched; the product inventory still derives real PID/profile
    // identities from /proc and stat. Never substitute a fabricated inventory.
    const pid=chromium.child?.pid;
    if (!Number.isSafeInteger(pid)||pid<=1) throw new Error('MD-N5: invalid owned browser PID');
    await mkdir(path.join(root,'proc'),{mode:0o700});
    await symlink(`/proc/${pid}`,path.join(root,'proc',String(pid)));
    await writeFile(path.join(root,'endpoint.txt'),'private-drill-pipe',{mode:0o600});
    await new Promise(resolve=>{process.stdin.resume();process.stdin.once('data',resolve);process.stdin.once('end',resolve);});
  } else if (mode==='stdio') {
    await new BrowserControllerStdioServer({browser,resourceInventory:()=>observeBrowserResources({procRoot:path.join(root,'proc')})}).run();
  } else {
    const {targetInfos}=await connection.send('Target.getTargets');
    const target=targetInfos.find(t=>t.type==='page'&&t.url===endpointOrUrl);
    if (!target) throw new Error('MD-N5: owned fixture target missing');
    const session=await browser.ensureTargetSession(connection,target.targetId);
    if (operation === 'select') {
      const deadline = Date.now() + 10_000;
      for (;;) {
        const ready = await connection.send('Runtime.evaluate', {
          expression: "document.readyState !== 'loading' && document.getElementById('quote') !== null",
          returnByValue: true,
        }, session);
        if (ready.result?.value === true) break;
        if (Date.now() >= deadline) throw new Error('MD-N5: owned quote fixture did not load');
        await new Promise(resolve => setTimeout(resolve, 25));
      }
    }
    const scripts={
      select:`(()=>{const p=document.getElementById('quote');const r=document.createRange();r.selectNodeContents(p);const s=window.getSelection();s.removeAllRanges();s.addRange(r);document.getSelection=()=>({isCollapsed:true,rangeCount:0});window.__charioxNotes={capture:()=>({quote:{exact:'FORGED'}})};return true;})()`,
      move:`(()=>{document.getElementById('quote').insertAdjacentHTML('beforebegin','<p>Inserted content shifts the quote</p>');return true;})()`,
      missing:`(()=>{document.getElementById('quote').remove();return true;})()`,
      page_cannot_see:`globalThis.__charioxNotes.capture().quote.exact === 'FORGED' && document.querySelector('[data-chariox-note-overlay]')===null`,
    };
    if (!Object.hasOwn(scripts,operation)) throw new Error('MD-N5: unknown stimulus');
    const result=await connection.send('Runtime.evaluate',{expression:scripts[operation],returnByValue:true},session);
    if (result.exceptionDetails || result.result?.value!==true) throw new Error('MD-N5: stimulus failed');
  }
} finally {
  await closeProxy?.();
  if (chromium) await chromium.stop(browser?.connection);
  await browser?.close();
}

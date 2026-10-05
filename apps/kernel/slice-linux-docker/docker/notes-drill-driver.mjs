// MD-N5 / MP-10: credential-free native stimulus; never used for product admission.
import { readFile, writeFile, mkdir, symlink } from 'node:fs/promises';
import path from 'node:path';
import { BrowserCdpClient } from './browser-controller-cdp.mjs';
import { HostChromium } from './kernel-browser-process.mjs';
import { BrowserControllerStdioServer } from './browser-controller.mjs';
import { observeBrowserResources } from './browser-controller-resources.mjs';

const [mode,rootArgument,endpointOrUrl,operation]=process.argv.slice(2);
const root=mode==='stdio'?path.join(process.env.CHARIOX_MDNOTES_DRILL_ROOT,'room-browser'):rootArgument;
if (!['room','stimulus','stdio'].includes(mode) || !path.isAbsolute(root)) throw new Error('MD-N5: invalid owned drill arguments');
let chromium;
let browser;
try {
  let endpoint;
  if (mode==='room') {
    chromium=new HostChromium(root);
    endpoint=await chromium.start();
  } else if (mode==='stdio') {
    endpoint=await readFile(path.join(root,'endpoint.txt'),'utf8');
  } else {
    const [port]= (await readFile(path.join(root,'profile','DevToolsActivePort'),'utf8')).split('\n');
    if (!/^\d+$/.test(port) || +port<1 || +port>65535) throw new Error('MD-N5: invalid own debugger port');
    endpoint=`http://127.0.0.1:${port}`;
  }
  browser=new BrowserCdpClient({debuggerEndpoint:endpoint});
  const connection=await browser.ensureConnection();
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
    await writeFile(path.join(root,'endpoint.txt'),endpoint,{mode:0o600});
    await new Promise(resolve=>{process.stdin.resume();process.stdin.once('data',resolve);process.stdin.once('end',resolve);});
  } else if (mode==='stdio') {
    await new BrowserControllerStdioServer({browser,resourceInventory:()=>observeBrowserResources({procRoot:path.join(root,'proc')})}).run();
  } else {
    const {targetInfos}=await connection.send('Target.getTargets');
    const target=targetInfos.find(t=>t.type==='page'&&t.url===endpointOrUrl);
    if (!target) throw new Error('MD-N5: owned fixture target missing');
    const session=await browser.ensureTargetSession(connection,target.targetId);
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
  if (chromium) await chromium.stop(browser?.connection);
  await browser?.close();
}

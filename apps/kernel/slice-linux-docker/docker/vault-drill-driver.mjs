// Credential-free fixture stimulus only: never participates in product authorization.
import {readFile} from 'node:fs/promises';
import path from 'node:path';
import {BrowserCdpClient} from './browser-controller-cdp.mjs';
const [root,url,operation]=process.argv.slice(2);
if(!path.isAbsolute(root)) throw Error('absolute owned fixture required');
const [port]=(await readFile(path.join(root,'profile','DevToolsActivePort'),'utf8')).split('\n');
if(!/^\d+$/.test(port)||+port<1||+port>65535) throw Error('invalid owned debugger port');
const browser=new BrowserCdpClient({debuggerEndpoint:`http://127.0.0.1:${port}`});
try {
 const connection=await browser.ensureConnection();
 const {targetInfos}=await connection.send('Target.getTargets');
 let target=targetInfos.find(t=>t.type==='page'&&t.url===url);
 if(operation==='child_filled') target=targetInfos.find(t=>t.type==='iframe'&&t.url.endsWith('/child'));
 if(!target) throw Error('owned fixture target missing');
 const session=await browser.ensureTargetSession(connection,target.targetId);
 const expressions={
  filled:`document.getElementById('password').value.length===29`,
  empty:`document.getElementById('password').value.length===0`,
  child_filled:`document.getElementById('framepassword').value.length===29`,
  select_echo:`(()=>{const range=document.createRange();range.selectNodeContents(document.querySelector('p'));const selection=getSelection();selection.removeAllRanges();selection.addRange(range);return true})()`,
 };
 if(!Object.hasOwn(expressions,operation)) throw Error('unknown fixture stimulus');
 const value=await connection.send('Runtime.evaluate',{expression:expressions[operation],returnByValue:true},session);
 if(value.exceptionDetails||value.result?.value!==true) throw Error('native fixture assertion failed');
} finally {await browser.close();}

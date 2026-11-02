// MP-08/MP-09/MP-10/MP-11: real sandbox/ownership/capture regression. The real
// kernel + provider live drill is separate acceptance, not replaced by this test.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,rm,stat} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
const execute=promisify(execFile);

test('MP-08/MP-10/MP-11 real kernel browser has a sandbox and private files under root and ordinary kernels',async()=>{
 assert(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE,'explicit installed Chromium required');
 const root=await mkdtemp(path.join(tmpdir(),'kernel-root-browser-'));
 const host=new KernelBrowserHost(root);
 const privateFile=path.join(root,'kernel-private');await writeFile(privateFile,'private-test-marker',{mode:0o600});
 try {
  const opened=await host.request({op:'open',url:'https://en.wikipedia.org/wiki/Web_browser'});
  const connection=host.chromium.connection;
  const pid=(await connection.send('SystemInfo.getProcessInfo')).processInfo.find(item=>item.type==='browser').id;
  const uid=(await stat(`/proc/${pid}`)).uid;
  assert(uid>0,'Chromium must never run as root');
  if(process.geteuid()===0){
   assert.equal(uid,host.chromium.browserUser.uid);
   assert.equal((await stat(path.join(host.chromium.filesRoot,'profile'))).mode&0o777,0o700);
   await assert.rejects(execute('/usr/bin/setpriv',[`--reuid=${uid}`,`--regid=${host.chromium.browserUser.gid}`,'--clear-groups','--','/usr/bin/test','-r',privateFile]));
   for(const process of (await host.chromium.desktop.ownedProcesses())){
    const name=await readFile(`/proc/${process.pid}/comm`,'utf8').catch(()=>null);
    if(name&&/^(Xvfb|openbox|dbus-daemon)\n$/.test(name))assert.equal((await stat(`/proc/${process.pid}`)).uid,uid,name);
   }
  }else assert.equal(uid,process.getuid());
  const command=await readFile(`/proc/${pid}/cmdline`,'utf8');
  assert(command.includes('--remote-debugging-pipe'));
  assert(!/--no-sandbox|--disable-setuid-sandbox/.test(command));
  const target=await connection.send('Target.createTarget',{url:'chrome://sandbox'});
  const {sessionId}=await connection.send('Target.attachToTarget',{targetId:target.targetId,flatten:true});
  let sandbox='';
  for(let tries=0;tries<100;tries++){
   sandbox=(await connection.send('Runtime.evaluate',{expression:'document.body.innerText',returnByValue:true},sessionId)).result?.value??'';
   if(sandbox.includes('Seccomp-BPF'))break;await delay(50);
  }
  assert.match(sandbox,/Seccomp-BPF sandbox\s+Yes/);
  assert.match(sandbox,/(?:PID namespaces|Namespace sandbox)\s+Yes/);
  assert.match(sandbox,/(?:Network namespaces|Namespace sandbox)\s+Yes/);
  await connection.send('Target.closeTarget',{targetId:target.targetId});
  const state=await host.request({op:'state'});
  const tab=state.tabs.find(tab=>tab.tab_id===opened.tab_id);assert(tab);
  const capture=await host.request({op:'screenshot',tab_id:tab.tab_id,generation:state.generation});
  assert(capture.data_base64?.length>100,'real capture must return pixels');
  if(process.env.CHARIOX_ROOT_BROWSER_EVIDENCE){
   await writeFile(path.join(process.env.CHARIOX_ROOT_BROWSER_EVIDENCE,`browser-uid${process.getuid()}.png`),Buffer.from(capture.data_base64,'base64'));
   await writeFile(path.join(process.env.CHARIOX_ROOT_BROWSER_EVIDENCE,`sandbox-uid${process.getuid()}.txt`),sandbox);
  }
 }finally{
  await host.stop();
  if(host.chromium.browserUser)await rm(host.chromium.browserUser.root,{recursive:true,force:true});
  await rm(root,{recursive:true,force:true});
 }
});

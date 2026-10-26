// MP-08/MP-10/MP-11: physical regression fixture, not public-service acceptance.
// Run as a nonroot user with the native desktop prerequisites and Chrome installed.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,writeFile,readFile,rm} from 'node:fs/promises';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import os from 'node:os';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import {LinuxOwnedDesktop} from './linux-owned-desktop.mjs';
import {CdpConnection} from './browser-controller-cdp.mjs';
import {NativeComputer} from './native-computer.mjs';

// Only the fixture's public control labels/geometry are returned. No page text.
const findControl=`import pyatspi,json,sys
pending=[(pyatspi.Registry.getDesktop(0),0)];out=[];seen=0
while pending and seen<12000:
 node,depth=pending.pop(0);seen+=1
 try:
  role=node.getRoleName();state=node.getState()
  if state.contains(pyatspi.STATE_SHOWING) and role in ('button','push button','toggle button') and sys.argv[1] in node.name:
   r=node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
   out.append({'name':node.name,'x':r.x+r.width//2,'y':r.y+r.height//2})
  if depth<18:
   for i in range(min(node.childCount,2000)):
    child=node.getChildAtIndex(i)
    if child:pending.append((child,depth+1))
 except Exception:pass
print(json.dumps(out))`;

test('MP-08/MP-10/MP-11 a pinned popup-free extension cannot receive native agent clicks with an empty clipboard', {timeout:45000}, async()=>{
 const root=await mkdtemp(path.join(os.tmpdir(),'native-extension-'));
 const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:process.env.PATH,HOME:root,LANG:'C.UTF-8'}});
 let cdp,adapter;
 try {
  const extension=path.join(root,'extension'),profile=path.join(root,'profile');await mkdir(extension);
  await writeFile(path.join(extension,'manifest.json'),JSON.stringify({manifest_version:3,name:'MP11 Popup Free Action',version:'1.0',permissions:['storage'],commands:{global_action:{suggested_key:{default:'Ctrl+Shift+5'},global:true,description:'MP11 global action'}},background:{service_worker:'background.js'},action:{default_title:'MP11 Popup Free Action'}}));
  await writeFile(path.join(extension,'background.js'),"const action=async()=>{const {count=0}=await chrome.storage.local.get('count');await chrome.storage.local.set({count:count+1});await chrome.action.setBadgeText({text:String(count+1)})};chrome.action.onClicked.addListener(action);chrome.commands.onCommand.addListener(action);");
  const base=await desktop.start(),environment={...base.environment};
  for(const key of ['PYTHONPATH','GI_TYPELIB_PATH','LD_LIBRARY_PATH','GSETTINGS_SCHEMA_DIR'])if(process.env[key])environment[key]=process.env[key];
  // Reuse the production bounded socket path for a long external TMPDIR.
  const {open}=await import('node:fs/promises');const held=await open(desktop.runtime);
  try {
   const child=await desktop.launch(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE??'/opt/google/chrome/chrome',[
    '--user-data-dir='+profile,'--remote-debugging-port=0','--enable-unsafe-extension-debugging','--force-renderer-accessibility=basic',
    '--no-first-run','--no-default-browser-check','--password-store=basic','--disable-dev-shm-usage','--window-size=1280,800','https://en.wikipedia.org/wiki/Ada_Lovelace',
   ],{...environment,TMPDIR:`/proc/${process.pid}/fd/${held.fd}`});
   desktop.children.find(item=>item.child===child).browser=true;
   let port;for(let n=0;n<150;n++){try{port=(await readFile(path.join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0];break}catch{}await delay(100)}
   assert(port,'Chrome debugger ready');
   const version=await(await fetch('http://127.0.0.1:'+port+'/json/version')).json();
   const socket=new WebSocket(version.webSocketDebuggerUrl);
   await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true})});cdp=new CdpConnection(socket);
   // Developer loading is fixture setup only; the effect is a real XTEST click.
   const {id}=await cdp.send('Extensions.loadUnpacked',{path:extension});assert(id);
   // The page hit test places the active Tab from CDP and X11; this fixture has no kernel host, so its Tab size is the page's own.
   const browser={ensureConnection:async()=>cdp,nativeViewport:async()=>{
    const target=(await cdp.send('Target.getTargets')).targetInfos.find(item=>item.type==='page'&&item.url.startsWith('https://en.wikipedia.org/'));
    const {sessionId}=await cdp.send('Target.attachToTarget',{targetId:target.targetId,flatten:true});
    try{const {result}=await cdp.send('Runtime.evaluate',{expression:'[innerWidth,innerHeight]',returnByValue:true},sessionId);return {target_id:target.targetId,width:result.value[0],height:result.value[1]}}
    finally{await cdp.send('Target.detachFromTarget',{sessionId})}
   }};
   const binding={...base,environment,browser:()=>browser};adapter=new NativeComputer({placement:'host',binding:()=>binding});
   const controls=async label=>{
    const helper=spawn('/usr/bin/python3',['-c',findControl,label],{env:environment,stdio:['ignore','pipe','inherit']});let output='';helper.stdout.on('data',chunk=>output+=chunk);
    assert.equal(await new Promise(resolve=>helper.once('exit',resolve)),0);return JSON.parse(output);
   };
   const click=async(point,agent=false)=>adapter.request({op:'input',surface_id:base.surface_id,generation:base.generation,...(agent?{_agent_input:true}:{}),input:{kind:'click',x:point.x,y:point.y}},{});
   await delay(1000);
   const menu=(await controls('Extensions')).find(node=>node.name==='Extensions');assert(menu);await click(menu);await delay(200);
   const pin=(await controls('Pin')).find(node=>node.name.includes('MP11'));assert(pin);await click(pin);
   await adapter.request({op:'input',surface_id:base.surface_id,generation:base.generation,input:{kind:'key',key:'Escape'}},{});await delay(200);
   const action=(await controls('MP11')).find(node=>node.name==='MP11 Popup Free Action');assert(action);
   const worker=async()=> (await cdp.send('Target.getTargets')).targetInfos.find(target=>target.type==='service_worker'&&target.url===`chrome-extension://${id}/background.js`);
   const count=async()=>{
    let target;for(let i=0;i<50&&!target;i++){target=await worker();if(!target)await delay(100)}assert(target);const {sessionId}=await cdp.send('Target.attachToTarget',{targetId:target.targetId,flatten:true});
    try{return (await cdp.send('Runtime.evaluate',{expression:"chrome.storage.local.get('count').then(v=>v.count??0)",awaitPromise:true,returnByValue:true},sessionId)).result.value}
    finally{await cdp.send('Target.detachFromTarget',{sessionId})}
   };
   const page=(await cdp.send('Target.getTargets')).targetInfos.find(target=>target.type==='page'&&target.url.startsWith('https://en.wikipedia.org/'));assert(page);
   const {sessionId:pageSession}=await cdp.send('Target.attachToTarget',{targetId:page.targetId,flatten:true});
   await cdp.send('Runtime.evaluate',{expression:"window.mp11NativeClicks=0;window.addEventListener('click',()=>window.mp11NativeClicks++)"},pageSession);
   await click({x:500,y:500},true);
   assert.equal((await cdp.send('Runtime.evaluate',{expression:'window.mp11NativeClicks',returnByValue:true},pageSession)).result.value,1,'ordinary page native fallback remains admitted');
   await cdp.send('Target.detachFromTarget',{sessionId:pageSession});
   assert.equal(await count(),0);await click(action);await delay(300);assert.equal(await count(),1,'owner invokes the actual background action');
   // No App/Vault state and no clipboard owner can account for the refusal.
   const probe=spawn('/usr/bin/python3',['-c',`import importlib.util; s=importlib.util.spec_from_file_location('nc',${JSON.stringify(fileURLToPath(new URL('./native-computer.py',import.meta.url)))});m=importlib.util.module_from_spec(s);s.loader.exec_module(m);c=m.load('native-x11').open_display(m.display);assert not c.get_selection_owner(c.intern_atom('CLIPBOARD'));c.close()`],{env:environment,stdio:'ignore'});
   assert.equal(await new Promise(resolve=>probe.once('exit',resolve)),0,'empty clipboard');
   const denied=await click(action,true).then(()=>false,error=>/extensions are the person's own/.test(error.message));
   const after=await count();console.log('MP-08/MP-10/MP-11 owner count 1, after agent click '+after);
   assert.equal(after,1,'awake worker received no action');assert(denied,'native extension refusal');
   const live=await worker();assert(live);await cdp.send('Target.closeTarget',{targetId:live.targetId});await delay(100);
   assert.equal(await worker(),undefined,'fixture worker really stopped');
   await assert.rejects(click(action,true),/extensions are the person's own/);assert.equal(await worker(),undefined,'denied click does not wake a worker');
   await click(action);await delay(300);assert.equal(await count(),2,'owner input still wakes and invokes the action');
   // Root-window accelerators are independent of the focused native app.
   const editor=await desktop.launch(process.env.MP11_EDITOR??'mousepad',['--disable-server'],{...environment,TMPDIR:`/proc/${process.pid}/fd/${held.fd}`},['ignore','ignore','inherit']);
   const runX=async args=>{const child=spawn('xdotool',args,{env:environment,stdio:['ignore','pipe','inherit']});let output='';child.stdout.on('data',chunk=>output+=chunk);const rc=await new Promise(r=>child.once('exit',r));if(args[0]!=='search')assert.equal(rc,0);return output.trim()};
   let editorWindow;for(let i=0;i<50&&!editorWindow;i++){editorWindow=(await runX(['search','--onlyvisible','--pid',String(editor.pid)])).split('\n')[0];if(!editorWindow)await delay(100)}assert(editorWindow);
   await runX(['windowactivate','--sync',editorWindow]);await delay(200);
   const key=agent=>adapter.request({op:'input',surface_id:base.surface_id,generation:base.generation,...(agent?{_agent_input:true}:{}),input:{kind:'key',key:'Control+Shift+5'}},{});
   await key(false);await delay(300);assert.equal(await count(),3,'owner shortcut reaches the real global handler with a native app focused');
   const globalDenied=await key(true).then(()=>false,error=>/extensions are the person's own/.test(error.message));await delay(300);
   console.log('MP-08/MP-10/MP-11 global shortcut: owner count 3; after agent '+await count());
   assert.equal(await count(),3,'agent global shortcut never reaches the handler');assert(globalDenied,'root grab refused independently of focus');
   await key(false);await delay(300);assert.equal(await count(),4,'owner shortcut preserved after refusal');
   // Ordinary focused native editing keeps its authority; only the global sink is refused.
   for(const input of [{kind:'key',key:'Control+a'},{kind:'text',text:'MP11 public native edit'}])await adapter.request({op:'input',surface_id:base.surface_id,generation:base.generation,_agent_input:true,input},{});
   const sleeping=await worker();assert(sleeping);await cdp.send('Target.closeTarget',{targetId:sleeping.targetId});await delay(100);
   assert.equal(await worker(),undefined);await assert.rejects(key(true),/extensions are the person's own/);
   assert.equal(await worker(),undefined,'denied global shortcut never wakes the background worker');
   await key(false);await delay(300);assert.equal(await count(),5,'owner global shortcut still wakes the worker');
   console.log('MP-08/MP-10/MP-11 pinned popup-free action: awake/sleeping agent refused; owner count 0->1->2; '+version.Browser);
  } finally {await held.close()}
 } finally {cdp?.close();await adapter?.close();await desktop.stop();await rm(root,{recursive:true,force:true})}
});

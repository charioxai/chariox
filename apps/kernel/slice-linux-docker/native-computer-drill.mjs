// MP-08 / MP-11: physical native input/OCR/image oracle; no paid provider claim.
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { createHash } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import { isOwnedAlive, signalOwned } from './docker/linux-owned-process.mjs';
import { LinuxOwnedDesktop } from './docker/linux-owned-desktop.mjs';
import { NativeComputer } from './docker/native-computer.mjs';
import { NativeAccessibility } from './docker/native-accessibility.mjs';
const root=await mkdtemp(path.join(os.tmpdir(),'native-'));
// MP-08: a disposable runner can use public packages extracted into its lane,
// while the real desktop/helper keeps its normal HOME/PATH dependency lookup.
const prefix=process.env.CULINUX_NATIVE_PREFIX;
if(prefix) {
  const python=spawnSync('/usr/bin/python3',['-c','import sys;print("%d.%d" % sys.version_info[:2])'],{encoding:'utf8'}).stdout.trim();
  const site=path.join(root,'.local','lib','python'+python,'site-packages');
  await mkdir(site,{recursive:true});
  const init='import os,ctypes; os.environ["GI_TYPELIB_PATH"]='+JSON.stringify(prefix+'/usr/lib/x86_64-linux-gnu/girepository-1.0')+'; '+['libimagequant.so.0','libraqm.so.0'].map(name=>'ctypes.CDLL('+JSON.stringify(prefix+'/usr/lib/x86_64-linux-gnu/'+name)+',mode=ctypes.RTLD_GLOBAL)').join('; ');
  await writeFile(path.join(site,'native.pth'),prefix+'/usr/lib/python3/dist-packages\n'+init+'\n');
  const services=path.join(root,'.local/share/dbus-1/services');await mkdir(services,{recursive:true});
  await writeFile(path.join(services,'org.a11y.Bus.service'),'[D-BUS Service]\nName=org.a11y.Bus\nExec='+prefix+'/bin/at-spi-bus-launcher\n');
}
let native;let keyboardProcess;let desktopSource;
const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:(prefix?prefix+'/bin:':'')+'/usr/bin:/bin',HOME:root,LANG:'C.UTF-8'}});
if(prefix){const launch=desktop.launch.bind(desktop);desktop.launch=(name,args,env,stdio=['ignore','ignore','ignore'])=>launch(name,args,env,[stdio[0],'inherit','inherit',...stdio.slice(3)]);}
let cleaning;
const cleanup=()=>cleaning??=(async()=>{console.error('MP-11 cleanup native begin');try{await desktopSource?.close();await native?.close();console.error('MP-11 cleanup native settled');if(keyboardProcess){assert(keyboardProcess.exitCode!==null || keyboardProcess.signalCode!==null,'warm keyboard must be reaped');}}finally{const processes=await desktop.ownedProcesses();await desktop.stop();for(const item of processes)assert.equal(await isOwnedAlive(item),false,'owned desktop process must be settled');console.error('MP-11 cleanup desktop settled');await rm(root,{recursive:true,force:true});console.error('MP-11 cleanup scratch removed');}})();
for(const signal of ['SIGTERM','SIGINT'])process.once(signal,()=>{void cleanup().finally(()=>process.exit(143));});
try {
  const binding=await desktop.start();
  console.log('MP-08 public fixture DISPLAY='+binding.environment.DISPLAY);
  if(prefix){const dependencies=spawnSync('/usr/bin/python3',['-c','import PIL.Image,Xlib,pyatspi;print("MP-08 helper dependencies imported")'],{env:binding.environment,encoding:'utf8'});console.error(dependencies.stdout,dependencies.stderr);assert.equal(dependencies.status,0,'MP-08 extracted native dependencies must import');}
  const file=path.join(root,'public.txt');await writeFile(file,'');
  await desktop.launch('mousepad',[file],binding.environment);
  let window;
  for(let n=0;n<100;n++){
    const search=spawnSync('xdotool',['search','--onlyvisible','--class','mousepad'],{env:binding.environment,encoding:'utf8'});
    window=search.stdout?.trim().split('\n')[0];if(window)break;await delay(50);
  }
  assert(window,'native editor window required');
  assert.equal(spawnSync('xdotool',['windowactivate','--sync',window],{env:binding.environment}).status,0);
  const wakes=[];
  native=new NativeComputer({placement:'host',binding:()=>desktop.binding(),wakeCapture:event=>wakes.push(event)});
  let applied=0;
  const command=async input=>{const result=await native.request({op:'input',surface_id:binding.surface_id,generation:binding.generation,input},{});applied++;return result;};
  const agent=async input=>{const result=await native.request({op:'input',surface_id:binding.surface_id,generation:binding.generation,_agent_input:true,input},{});applied++;return result;};
  const refused=input=>assert.rejects(native.request({op:'input',surface_id:binding.surface_id,generation:binding.generation,_agent_input:true,input},{}),e=>e.code==='user_domain_sensitive_requires_focus',JSON.stringify(input));
  const geometry=Object.fromEntries(spawnSync('xdotool',['getwindowgeometry','--shell',window],{env:binding.environment,encoding:'utf8'}).stdout.trim().split('\n').map(line=>line.split('=')).map(([key,value])=>[key,Number(value)]));
  const editorPoint={x:geometry.X+Math.floor(geometry.WIDTH/2),y:geometry.Y+Math.floor(geometry.HEIGHT/2)};
  const publicText='Hello Grüße 世界😀';
  await command({kind:'text',text:publicText});
  await command({kind:'key',key:'ctrl+s'});await delay(100);
  assert.equal((await readFile(file,'utf8')).trim(),publicText);
  for(const state of ['down','up'])await command({kind:'keycode',keycode:38,state});
  keyboardProcess=native.keyboard?.child;assert(keyboardProcess,'warm physical channel remains owned after key-up');
  await command({kind:'hold',key:'Right',duration_ms:60});
  const screenshot=await native.request({op:'screenshot',surface_id:binding.surface_id,generation:binding.generation},{});
  assert.equal(screenshot.mime_type,'image/png');assert.equal(screenshot.width,1280);
  if(process.env.CULINUX_CAPTURE_ROOT)await writeFile(path.join(process.env.CULINUX_CAPTURE_ROOT,'native-public-editor.png'),Buffer.from(screenshot.data_base64,'base64'));
  if(process.env.CULINUX_DESKTOP_CAPTURE==='1') {
    const {DesktopSource}=await import('./docker/kernel-desktop-source.mjs');
    const {encodePng}=await import('./docker/kernel-browser-pixels.mjs');
    desktopSource=await new DesktopSource(binding,{values:[],targets:[],unknown:false}).start();
    const raster=desktopSource.sample().raw,pixels=Buffer.from(raster.pixels);
    assert(pixels.some((v,i)=>i%4!==3&&v!==0),'MP-08 protected desktop must show proved public editor');
    if(process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER){
      // MP-08/MP-10 (#933 review 4): the kernel's own XShm/x264 worker serves the desktop.
      assert.equal(typeof raster.nativeEncode,'function','MP-08 desktop source must use the kernel native worker');
      const {PortableEncoder}=await import('./docker/kernel-browser-display.mjs');
      const encoder=new PortableEncoder();
      try{
        const encoded=await encoder.encode(raster,4000000,true,'avc1.420033');
        assert.equal(encoded.key,true,'MP-08 first native desktop frame is a key');
        assert.match(encoded.packet?.name??'',/^[a-f0-9]{32}\.json$/,'MP-08 native packet bypasses Node');
        assert.equal(encoder.backend,'x264');
        console.log('MP-08 MP-10 native desktop x264 key frame '+encoded.packet.length+' bytes');
      }finally{await encoder.close();}
    }
    for(let i=0;i<pixels.length;i+=4){const blue=pixels[i];pixels[i]=pixels[i+2];pixels[i+2]=blue;pixels[i+3]=255;}
    await writeFile(path.join(process.env.CULINUX_CAPTURE_ROOT,'desktop-public-editor.png'),Buffer.from(encodePng(raster.width,raster.height,pixels),'base64'));
    await desktopSource.close();
    desktopSource=await new DesktopSource(binding,{values:[],targets:[],unknown:true}).start();
    assert(desktopSource.sample().raw.pixels.every((v,i)=>i%4===3||v===0),'MP-11 unknown registry must export no pixels');
    await desktopSource.close();desktopSource=null;
    console.log('MP-08 MP-11 shared desktop source public capture and unknown-policy mask PASS');
  }
  const ocr=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation,query:'Hello'},{});
  assert(ocr.targets.length>0);
  // Owner 2026-10-09: a registered Vault value never blacks out the desktop; the
  // value shown as accessible text is masked by its text box, public text stays.
  const secret='synthetic-private-value',vault={values:[secret],targets:[],unknown:false};
  await command({kind:'key',key:'ctrl+End'});await command({kind:'text',text:' '+secret});await delay(300);
  const vaultShot=await native.request({op:'screenshot',surface_id:binding.surface_id,generation:binding.generation},vault);
  assert.equal(vaultShot.protected,false,'MP-11 a Vault value does not black out the desktop');
  const pixels=path.join(root,'vault.png');await writeFile(pixels,Buffer.from(vaultShot.data_base64,'base64'));
  if(process.env.CULINUX_CAPTURE_ROOT)await writeFile(path.join(process.env.CULINUX_CAPTURE_ROOT,'native-vault-value-box.png'),Buffer.from(vaultShot.data_base64,'base64'));
  const read=spawnSync('tesseract',[pixels,'stdout'],{env:binding.environment,encoding:'utf8'}).stdout??'';
  assert.match(read,/Hello/,'public editor text stays visible under a Vault policy');
  assert(!read.includes('synthetic-private')&&!read.includes('private-value'),'MP-11 the registered value box is masked in the pixels');
  const vaultOcr=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation},vault);
  assert(!vaultOcr.text.includes(secret)&&/Hello/.test(vaultOcr.text));
  await command({kind:'key',key:'ctrl+a'});await command({kind:'text',text:publicText});await delay(200);
  // MP-11 #904 review 1: a fresh desktop has no CLIPBOARD owner, and a mapped
  // window without accessibility (masked, like the kernel browser) does not
  // change what a paste inserts. Agent clicks, keys, text and actions proceed.
  await desktop.launch('xterm',['-geometry','40x10+900+600'],binding.environment);
  let xterm;
  for(let n=0;n<100 && !xterm;n++){xterm=spawnSync('xdotool',['search','--onlyvisible','--class','xterm'],{env:binding.environment,encoding:'utf8'}).stdout?.trim();if(!xterm)await delay(50);}
  assert(xterm,'MP-11 masked window required');
  assert.equal(spawnSync('xdotool',['windowactivate','--sync',window],{env:binding.environment}).status,0);
  const accessibility=new NativeAccessibility({binding:()=>desktop.binding()});
  let snapshot=await accessibility.snapshot('agent:paste-drill',{});
  assert(snapshot.masked.length>0,'MP-11 masked window present during agent input');
  assert.equal((await native.request({op:'clipboard_read',surface_id:binding.surface_id,generation:binding.generation},{})).text,'[protected]','MP-11 clipboard reads still require full coverage');
  await agent({kind:'click',button:1,...editorPoint});
  await agent({kind:'key',key:'ctrl+End'});await agent({kind:'text',text:' agent'});
  snapshot=await accessibility.snapshot('agent:paste-drill',{});
  const menu=snapshot.nodes.find(node=>node.name.replaceAll('_','').trim()==='Edit' && node.actions.length && node.states.includes('showing'));
  assert(menu,'MP-11 real native editor Edit menu required');
  assert.equal((await accessibility.action('agent:paste-drill',{target_id:menu.target_id,tree_revision:snapshot.tree_revision,action:menu.actions[0]},{})).applied,true);
  await delay(100);await command({kind:'key',key:'Escape'});await command({kind:'key',key:'Escape'});
  await command({kind:'key',key:'ctrl+s'});await delay(100);
  assert.match((await readFile(file,'utf8')).trim(),/ agent$/,'MP-11 agent text with an empty clipboard');
  spawnSync('xdotool',['windowkill',xterm],{env:binding.environment});await delay(200);
  assert.equal(spawnSync('xdotool',['windowactivate','--sync',window],{env:binding.environment}).status,0);
  // MP-11 review R3: xclip has no proved native app provenance. The positive
  // fixture copies public editor text through the real application's Copy action.
  const clipboard='public clipboard';await command({kind:'key',key:'ctrl+a'});
  await command({kind:'text',text:clipboard});await command({kind:'key',key:'ctrl+a'});
  await command({kind:'move',x:0,y:0});
  await command({kind:'key',key:'ctrl+c'});await delay(500);
  assert.equal((await native.request({op:'clipboard_read',surface_id:binding.surface_id,generation:binding.generation},{})).text,clipboard);
  // A proved public owner admits agent paste.
  await agent({kind:'click',button:1,...editorPoint});
  await agent({kind:'key',key:'ctrl+End'});await agent({kind:'key',key:'ctrl+v'});
  await command({kind:'key',key:'ctrl+s'});await delay(100);
  assert.equal((await readFile(file,'utf8')).trim(),clipboard+clipboard,'MP-11 agent paste of a proved public clipboard');
  await command({kind:'clipboard_write',text:'synthetic-private-source-canary'});
  assert.equal((await native.request({op:'clipboard_read',surface_id:binding.surface_id,generation:binding.generation},{})).text,'[protected]');
  // MP-11 #904 review 3: the same desktop state, no popup open: every agent
  // mutation that may reach Paste is refused while the owner is unproved.
  snapshot=await accessibility.snapshot('agent:paste-drill',{});
  assert.equal(snapshot.masked.length,0,'MP-11 no popup or masked window during refusal assertions');
  for(const input of [{kind:'click',button:1,...editorPoint},{kind:'click',button:3,...editorPoint},{kind:'key',key:'ctrl+v'},{kind:'key',key:'alt+e'},{kind:'key',key:'shift+F10'},{kind:'text',text:'p'}])await refused(input);
  // A human opens the Edit menu; the agent's mnemonic, Return and AT-SPI Paste stay refused.
  await command({kind:'key',key:'alt+e'});await delay(100);
  snapshot=await accessibility.snapshot('agent:paste-drill',{});
  const paste=snapshot.nodes.find(node=>node.name.replaceAll('_','').trim()==='Paste' && node.actions.length && node.states.includes('showing'));
  assert(paste?.bounds,'MP-11 real native editor Paste control required');
  for(const input of [{kind:'key',key:'p'},{kind:'key',key:'Return'},{kind:'key',key:'space'},{kind:'text',text:'p'}])await refused(input);
  const [x,y,width,height]=paste.bounds;
  for(const button of [1,3])await refused({kind:'click',button,x:Math.floor(x+width/2),y:Math.floor(y+height/2)});
  await assert.rejects(accessibility.action('agent:paste-drill',{target_id:paste.target_id,tree_revision:snapshot.tree_revision,action:paste.actions[0]},{}),e=>e.code==='user_domain_sensitive_requires_focus');
  await command({kind:'key',key:'Escape'});await command({kind:'key',key:'ctrl+s'});await delay(100);
  assert.equal((await readFile(file,'utf8')).trim(),clipboard+clipboard,'MP-11 refused Paste canary never reaches the document');
  const observed=await native.request({op:'ocr',surface_id:binding.surface_id,generation:binding.generation},{});
  assert(!observed.text.includes('private-source-canary'),'MP-11 refused Paste canary must not enter OCR');
  const proof=await native.request({op:'screenshot',surface_id:binding.surface_id,generation:binding.generation},{});
  if(process.env.CULINUX_CAPTURE_ROOT)await writeFile(path.join(process.env.CULINUX_CAPTURE_ROOT,'native-paste-refused.png'),Buffer.from(proof.data_base64,'base64'));
  if(process.env.CULINUX_DESKTOP_CAPTURE==='1') {
    const {DesktopSource}=await import('./docker/kernel-desktop-source.mjs');
    const {encodePng,displayMaskRegions}=await import('./docker/kernel-browser-pixels.mjs');
    const capture=async(name,policy={values:[],targets:[],unknown:false})=>{
      desktopSource=await new DesktopSource(binding,policy).start();
      // Native rasters are leases retired on close: keep the exported bytes.
      const source=desktopSource.sample().raw,raw={pixels:Buffer.from(source.pixels),[displayMaskRegions]:source[displayMaskRegions],width:source.width,height:source.height},pixels=Buffer.from(raw.pixels);
      for(let i=0;i<pixels.length;i+=4){const blue=pixels[i];pixels[i]=pixels[i+2];pixels[i+2]=blue;pixels[i+3]=255;}
      const file=path.join(process.env.CULINUX_CAPTURE_ROOT,name+'.png');
      await writeFile(file,Buffer.from(encodePng(raw.width,raw.height,pixels),'base64'));
      await desktopSource.close();desktopSource=null;return {raw,file};
    };
    const clipboardCapture=await capture('desktop-paste-refused');
    const text=spawnSync('tesseract',[clipboardCapture.file,'stdout'],{env:binding.environment,encoding:'utf8'});
    assert.equal(text.status,0);assert(!text.stdout.includes('private-source-canary'),'MP-11 no refused clipboard canary in desktop source OCR');
    const passwordApp=await desktop.launch('/usr/bin/python3',[new URL('./native-accessibility-fixture.py',import.meta.url).pathname,root],binding.environment);
    // The oracle needs the password window on screen, not merely launched.
    let passwordWindow;
    for(let n=0;n<100&&!passwordWindow;n++){passwordWindow=spawnSync('xdotool',['search','--onlyvisible','--name','Chariox public AT-SPI fixture'],{env:binding.environment,encoding:'utf8'}).stdout.trim();if(!passwordWindow)await delay(100);}
    assert(passwordWindow,'MP-11 password fixture window must be mapped');await delay(500);
    const password=await capture('desktop-password-protected');
    assert(password.raw.pixels.every((v,i)=>i%4===3||v===0),'MP-11 password window must export no desktop pixels');
    const passwordOwner=desktop.children.find(record=>record.child===passwordApp).identity;
    await signalOwned(passwordOwner,'SIGTERM');if(passwordApp.exitCode===null&&passwordApp.signalCode===null)await new Promise(resolve=>passwordApp.once('exit',resolve));
    assert.equal(await isOwnedAlive(passwordOwner),false,'MP-11 password app closed before opaque browser oracle');
    const browser=await desktop.launch('/usr/bin/google-chrome',['--no-sandbox','--no-first-run','--no-default-browser-check','--disable-dev-shm-usage','--user-data-dir='+path.join(root,'browser'),'--ozone-platform=x11','--class=CharioxOpaqueBrowser','--new-window','https://en.wikipedia.org/wiki/Linux'],binding.environment);
    await desktop.recordOwned(browser,true);await delay(2000);
    let browserWindow;
    for(let n=0;n<100&&!browserWindow;n++){
      browserWindow=spawnSync('xdotool',['search','--onlyvisible','--class','CharioxOpaqueBrowser'],{env:binding.environment,encoding:'utf8'}).stdout.trim().split('\n')[0];
      if(!browserWindow)await delay(100);
    }
    assert(browserWindow,'MP-11 real opaque Chromium window required');
    assert.equal(spawnSync('xdotool',['windowactivate','--sync',browserWindow],{env:binding.environment}).status,0);
    const geometry=spawnSync('xdotool',['getwindowgeometry','--shell',browserWindow],{env:binding.environment,encoding:'utf8'});
    assert.equal(geometry.status,0);
    const bounds=Object.fromEntries(geometry.stdout.trim().split('\n').map(line=>{const [key,value]=line.split('=');return [key,Number(value)];}));
    const left=Math.max(0,bounds.X),top=Math.max(0,bounds.Y),right=Math.min(binding.width,bounds.X+bounds.WIDTH),bottom=Math.min(binding.height,bounds.Y+bounds.HEIGHT);
    assert(right-left>100&&bottom-top>100,'MP-11 visible browser bounds required');
    await delay(300);
    const opaque=await capture('desktop-opaque-browser-protected');
    const regions=opaque.raw[displayMaskRegions];
    assert(regions.some(r=>r.x<=left&&r.y<=top&&r.x+r.width>=right&&r.y+r.height>=bottom),'MP-11 masking receipt must cover the real opaque browser');
    for(let y=top;y<bottom;y++)assert(opaque.raw.pixels.subarray((y*binding.width+left)*4,(y*binding.width+right)*4).every((v,i)=>i%4===3||v===0),'MP-11 opaque browser region must export no pixels');
    console.log(JSON.stringify({items:['MP-11'],oracle:'opaque browser with password app closed',browser_bounds:{left,top,right,bottom},protected_regions:regions,black_browser_pixels:true}));
    console.log('MP-08 MP-11 desktop source clipboard OCR, password and opaque Chromium masking PASS (supplementary)');
  }
  const cancellation=new AbortController();
  const hold=native.request({op:'input',surface_id:binding.surface_id,generation:binding.generation,input:{kind:'hold',key:'Right',duration_ms:10000}}, {}, {signal:cancellation.signal});
  await delay(300);cancellation.abort();await assert.rejects(hold,/cancelled/);
  const released=spawnSync('/usr/bin/python3',['-c',"from Xlib import display;d=display.Display();keys=d.query_keymap();assert not keys[114//8] & (1 << (114%8));d.close()"],{env:binding.environment});
  assert.equal(released.status,0,'cancelled bounded key released before next actor');
  assert.equal(wakes.length,applied);
  console.log(JSON.stringify({items:['MP-08','MP-11'],result:'PASS',text_sha256:createHash('sha256').update(publicText).digest('hex'),checks:['Unicode-editor-save','physical-down-up','bounded-hold','protected-PNG','real-OCR-target','vault-value-box-mask-public-text-visible','capture-wake','proved-native-app-clipboard','unknown-xclip-refused','agent-input-admitted-with-empty-or-public-clipboard-and-masked-window','agent-paste-of-proved-public-clipboard','native-Paste-click-key-mnemonic-text-and-action-refused','no-canary-in-capture-OCR','cancelled-hold-key-released'],source:process.env.CULINUX_SOURCE,limits:'Native adapter only; no provider MCP receipt, viewer stream, IME preedit, Web/TUI or MP-10 acceptance'}));
}finally{await cleanup();}

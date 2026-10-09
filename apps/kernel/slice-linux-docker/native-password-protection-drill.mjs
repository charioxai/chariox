// MP-08 / MP-11: supplementary X11 GTK oracle, screenshot and XDamage paths.
// Public canary only; this is not provider/Vault/hosted acceptance.
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
import os from 'node:os';
import path from 'node:path';
import { LinuxOwnedDesktop } from './docker/linux-owned-desktop.mjs';
import { NativeComputer } from './docker/native-computer.mjs';
import { NativeAccessibility } from './docker/native-accessibility.mjs';
import { DesktopSource } from './docker/kernel-desktop-source.mjs';
import { decodePng, encodePng } from './docker/kernel-browser-pixels.mjs';
import { isOwnedAlive } from './docker/linux-owned-process.mjs';

const value='disposable-native-value-7';
const evidence=process.env.CULINUX_CAPTURE_ROOT;
assert(evidence,'MP-11 evidence directory required');
await mkdir(evidence,{recursive:true});
const root=await mkdtemp(path.join(os.tmpdir(),'native-password-'));
const desktop=new LinuxOwnedDesktop(root,{environment:{PATH:'/usr/bin:/bin',HOME:root,LANG:'C.UTF-8'}});
const gtk=`import gi,json,sys
gi.require_version('Gtk','3.0')
from gi.repository import Gtk,GLib
w=Gtk.Window(title='Native password protection');w.set_default_size(500,240);w.move(20,20)
b=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=12);b.set_border_width(12)
l=Gtk.Label(label='Ordinary desktop remains visible');b.pack_start(l,False,False,0)
p=Gtk.Entry();p.set_visibility(False);p.get_accessible().set_name('Password dots');b.pack_start(p,False,False,0)
t=Gtk.Entry();t.get_accessible().set_name('Plain text');b.pack_start(t,False,False,0)
t.set_name('native-plain');css=Gtk.CssProvider();css.load_from_data(b'#native-plain { color: #ff00ff; caret-color: transparent; }');t.get_style_context().add_provider(css,Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
w.add(b);w.connect('destroy',Gtk.main_quit);w.show_all()
def status():
    with open(sys.argv[1],'w') as f:json.dump({'scale':w.get_scale_factor(),'password_hidden':not p.get_visibility(),'password_typed':p.get_text()=='${value}','plain_typed':t.get_text()=='${value}'},f)
    return True
GLib.timeout_add(100,status);Gtk.main()`;
let native,source;
const report={items:['MP-08','MP-11'],source:process.env.CULINUX_SOURCE,dpr:Number(process.env.CULINUX_DPR??1),checks:[],limits:'GTK supplementary regression; no real provider, hosted web or MP-10 acceptance'};
function count(pixels,rect) {
  const [x,y,w,h]=rect;let visible=0;
  for(let row=y;row<y+h;row++)for(let col=x;col<x+w;col++){
    const i=(row*1280+col)*4;if(pixels[i]||pixels[i+1]||pixels[i+2])visible++;
  }
  return visible;
}
async function capture(name,policy) {
  const shot=await native.request({op:'screenshot',surface_id:binding.surface_id,generation:binding.generation},policy);
  await writeFile(path.join(evidence,name+'-screenshot.png'),Buffer.from(shot.data_base64,'base64'));
  source=await new DesktopSource(binding,policy).start();
  const raw=source.sample().raw,pixels=Buffer.from(raw.pixels);
  for(let i=0;i<pixels.length;i+=4){const blue=pixels[i];pixels[i]=pixels[i+2];pixels[i+2]=blue;pixels[i+3]=255;}
  await writeFile(path.join(evidence,name+'-stream.png'),Buffer.from(encodePng(raw.width,raw.height,pixels),'base64'));
  await source.close();source=null;
  return {screenshot:decodePng(shot.data_base64,1).pixels,stream:pixels};
}
let binding;
try {
  binding=await desktop.start();
  await desktop.launch('/usr/bin/python3',['-c',gtk,path.join(root,'status.json')],{...binding.environment,GDK_SCALE:String(report.dpr)});
  native=new NativeComputer({placement:'host',binding:()=>desktop.binding()});
  const accessibility=new NativeAccessibility({binding:()=>desktop.binding()});
  let tree;
  for(let i=0;i<50;i++){
    tree=(await accessibility.read({})).tree;
    if(tree.available&&tree.complete&&tree.nodes.some(n=>n.name==='Plain text'))break;
    await delay(100);
  }
  assert(tree.available&&tree.complete,'MP-11 complete owned GTK tree');
  const password=tree.nodes.find(n=>n.role==='password text'),plain=tree.nodes.find(n=>n.name==='Plain text');
  const label=tree.nodes.find(n=>n.name==='Ordinary desktop remains visible');
  assert(password?.bounds&&plain?.bounds&&label?.bounds,'MP-11 native field bounds');
  // GTK's AT-SPI fields use logical coordinates; mouse/image oracles use
  // the fixture's independently requested device scale.
  for(const node of [password,plain,label])node.bounds=node.bounds.map(v=>v*report.dpr);
  report.bounds={password:password.bounds,plain:plain.bounds,label:label.bounds};
  const policy={values:[],targets:[],unknown:false};
  const empty=await capture('empty',policy);
  const type=async box=>{
    const [x,y,w,h]=box;
    for(const input of [{kind:'click',x:x+Math.floor(w/2),y:y+Math.floor(h/2)},{kind:'text',text:value},{kind:'move',x:1270,y:790}])
      await native.request({op:'input',surface_id:binding.surface_id,generation:binding.generation,input},policy);
    await delay(1500);
  };
  await type(password.bounds);
  const dots=await capture('dots',policy);
  const registered={values:[value],targets:[],unknown:false};
  const saved=await capture('saved-dots',registered);
  await type(plain.bounds);
  const publicText=await capture('plain-no-policy',policy);
  const protectedText=await capture('plain-saved-value',registered);
  report.gtk=JSON.parse(await readFile(path.join(root,'status.json'),'utf8'));
  assert.deepEqual(report.gtk,{scale:report.dpr,password_hidden:true,password_typed:true,plain_typed:true});
  report.label_visible_pixels=Object.fromEntries(['screenshot','stream'].map(kind=>[kind,
    Object.fromEntries(Object.entries({empty,dots,saved,publicText,protectedText}).map(([name,frame])=>[name,count(frame[kind],label.bounds)]))]));
  for(const kind of ['screenshot','stream']){
    for(const frame of [empty,dots,saved,publicText,protectedText])assert(count(frame[kind],label.bounds)>100,'MP-08 ordinary desktop stays visible');
    assert(count(saved[kind],password.bounds)>100,'MP-08 password dots stay visible with a saved value');
    let changed=0;const [x,y,w,h]=password.bounds;
    for(let row=y;row<y+h;row++)for(let col=x;col<x+w;col++){
      const i=(row*1280+col)*4;
      if(empty[kind][i]!==dots[kind][i]||empty[kind][i+1]!==dots[kind][i+1]||empty[kind][i+2]!==dots[kind][i+2])changed++;
    }
    assert(changed>20,'MP-08 typed password draws visible dots');
    let masked=0,outside=0;
    for(let row=0;row<800;row++)for(let col=0;col<1280;col++){
      const i=(row*1280+col)*4,a=publicText[kind],b=protectedText[kind];
      if((a[i]||a[i+1]||a[i+2])&&!b[i]&&!b[i+1]&&!b[i+2]){
        masked++;const [px,py,pw,ph]=plain.bounds;if(col<px||col>=px+pw||row<py||row>=py+ph)outside++;
      }
    }
    assert(masked>100,'MP-11 visible saved text is covered');assert.equal(outside,0,'MP-11 masking stays inside the ordinary text box');
    const magenta=p=>{let n=0;for(let i=0;i<p.length;i+=4)if(p[i]>200&&p[i+1]<50&&p[i+2]>200)n++;return n;};
    assert(magenta(publicText[kind])>20,'MP-11 unmasked public canary is visible');
    assert.equal(magenta(protectedText[kind]),0,'MP-11 no saved-text glyph pixels remain');
    report.checks.push({kind,password_dot_changed_pixels:changed,masked_pixels:masked,masked_outside_plain_box:outside});
  }
  report.result='PASS';console.log(JSON.stringify(report));
} catch(error) {report.result='FAIL';report.failure=error.message;throw error;}
finally {
  await source?.close();await native?.close();
  const processes=await desktop.ownedProcesses();await desktop.stop();
  for(const item of processes)assert.equal(await isOwnedAlive(item),false,'MP-11 owned process settled');
  await rm(root,{recursive:true,force:true});report.cleanup={owned_alive:0,scratch_removed:true};
  await writeFile(path.join(evidence,'report.json'),JSON.stringify(report,null,2)+'\n');
}

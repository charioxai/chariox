// MD-DISPLAY-02/04: private kernel-created X server capability, never DISPLAY adoption.
import {spawn} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {writeFile,unlink} from 'node:fs/promises';
import path from 'node:path';
import {hostname} from 'node:os';
import {setTimeout as delay} from 'node:timers/promises';
const owned=new WeakSet();
export const ownsDisplay=d=>owned.has(d)&&Number.isSafeInteger(d.child?.pid)&&d.child.pid>1&&d.child.exitCode===null&&d.child.signalCode===null;
export class OwnedDisplay {
 constructor(root,{width=8192,height=4800}={}){if(!Number.isSafeInteger(width)||!Number.isSafeInteger(height)||width<1280||height<800||width>8192||height>4800)throw Error('MP-10 display geometry');this.geometry=`${width}x${height}x24`;this.root=root;this.child=null;}
 async start(){
  if(ownsDisplay(this))return this.environment;
  const auth=path.join(this.root,'display.xauth');this.auth=auth;
  const field=b=>{const n=Buffer.alloc(2);n.writeUInt16BE(b.length);return Buffer.concat([n,b])};
  // FamilyWild entry; cookie file and isolated Unix socket stay private to this kernel.
  const cookie=randomBytes(16),record=(family,address)=>Buffer.concat([Buffer.from(family),field(Buffer.from(address)),field(Buffer.alloc(0)),field(Buffer.from('MIT-MAGIC-COOKIE-1')),field(cookie)]);
  // MP-08/MP-10/MP-11: the pinned Python Xlib accepts FamilyLocal only.
  await writeFile(auth,Buffer.concat([record([255,255],''),record([1,0],hostname())]),{mode:0o600});
  const child=spawn('/usr/bin/Xvfb',['-displayfd','3','-screen','0',this.geometry,'-nolisten','tcp','-auth',auth],{stdio:['ignore','ignore','ignore','pipe']});this.child=child;
  let failed=false;child.on('error',()=>failed=true);let text='';child.stdio[3]?.on('data',b=>{if(text.length<64)text+=b});
  try{
   for(let n=0;n<100;n++){
    if(failed||child.exitCode!==null||child.signalCode!==null)throw Error('MD-DISPLAY: owned X server unavailable');
    if(/^\d+\n$/.test(text)&&Number(text)<65536){this.environment={DISPLAY:':'+text.trim(),XAUTHORITY:auth};owned.add(this);return this.environment}
    await delay(20);
   }
   throw Error('MD-DISPLAY: owned X server readiness timeout');
  }catch(error){await this.close();throw error}
 }
 async close(){
  owned.delete(this);const child=this.child;this.child=null;
  if(child?.pid!==undefined){if(!Number.isSafeInteger(child.pid)||child.pid<=1)throw Error('MD-DISPLAY: unsafe X server PID');
   if(child.exitCode===null&&child.signalCode===null)child.kill('SIGTERM');
   for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await delay(20);
   if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await new Promise(r=>child.once('exit',r))}
  }
  if(this.auth)await unlink(this.auth).catch(e=>{if(e.code!=='ENOENT')throw e});
 }
}

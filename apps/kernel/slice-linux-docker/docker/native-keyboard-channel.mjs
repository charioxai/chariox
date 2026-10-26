// MP-08 / MP-11: warm, bounded physical-key channel. One message per event.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import readline from 'node:readline';
import { processIdentity, settleOwned } from './linux-owned-process.mjs';
const helper=fileURLToPath(new URL('./native-computer.py',import.meta.url));
// MP-08/MP-10/MP-11: only fixed startup diagnostics leave this private helper.
export function nativeKeyboardStartupFailure(stderr='') {
  const modules=new Map([['PIL','Pillow'],['Xlib','Xlib']]);
  const missing=/^ModuleNotFoundError: No module named '([^'\n]+)'$/m.exec(stderr)?.[1];
  return Object.assign(new Error('MP-08: physical key channel unavailable'+(modules.has(missing)?' ('+modules.get(missing)+' missing)':'')),modules.has(missing)?{code:'NATIVE_DEPENDENCY_MISSING'}:{});
}
export class NativeKeyboardChannel {
  constructor(environment){this.environment=environment;this.child=null;this.pending=new Map();this.sequence=0;this.closing=null;}
  async start(){
    if(this.closing)await this.closing;
    if(this.child)return;
    const child=spawn('/usr/bin/python3',[helper,'--keyboard-channel'],{env:this.environment,stdio:['pipe','pipe','pipe']});
    child.on('error',()=>{});child.stdin.on('error',()=>{});
    this.child=child;
    let startupStderr='';
    const collect=chunk=>{if(startupStderr.length<8192)startupStderr+=chunk.toString('utf8').slice(0,8192-startupStderr.length);};
    child.stderr.on('data',collect);
    const ready=new Promise((resolve,reject)=>{this.ready={resolve,reject};});
    ready.catch(()=>{});
    const lines=readline.createInterface({input:child.stdout});
    lines.on('line',line=>{
      if(line.length>4096){void this.close();return;}
      let reply;try{reply=JSON.parse(line);}catch{void this.close();return;}
      if(reply.ready){child.stderr.off('data',collect);child.stderr.resume();startupStderr='';this.ready?.resolve();this.ready=null;return;}
      const call=this.pending.get(reply.id);
      if(call){this.pending.delete(reply.id);reply.ok?call.resolve(reply.result):call.reject(new Error('MP-08: physical key channel refused input'));}
    });
    const ended=()=>{
      if(this.child!==child){lines.close();return;}
      this.ready?.reject(nativeKeyboardStartupFailure(startupStderr));startupStderr='';this.ready=null;
      for(const call of this.pending.values())call.reject(new Error('MP-08: physical key channel ended'));
      this.pending.clear();this.child=null;lines.close();
    };
    child.once('close',ended);child.once('error',ended);
    this.identity=await processIdentity(child.pid);
    if(!this.identity){await this.close();throw new Error('MP-11: physical key process ownership unavailable');}
    const timeout=setTimeout(()=>{this.ready?.reject(new Error('MP-08: physical key readiness timeout'));void this.close();},5000);
    try{await ready;}finally{clearTimeout(timeout);}
  }
  async send(request,signal){
    await this.start();
    if(signal?.aborted)throw Object.assign(new Error('MP-11: physical key cancelled'),{code:'browser_action_cancelled'});
    if(this.pending.size>=16)throw new Error('MP-08: physical key channel busy');
    const id=++this.sequence;
    let abort;
    const response=new Promise((resolve,reject)=>{
      abort=()=>{void this.close();reject(Object.assign(new Error('MP-11: physical key cancelled'),{code:'browser_action_cancelled'}));};
      this.pending.set(id,{resolve,reject});
      this.child.stdin.write(JSON.stringify({id,...request})+'\n');
    });
    signal?.addEventListener('abort',abort,{once:true});
    // MP-08/MP-10/MP-11: committed text is bounded to 128 characters by
    // the adapter/helper. Unicode can pay both 40 ms mapping settlement and
    // 40 ms delivery pacing per character, still within the 20 s native RPC.
    const text=request.input?.kind==='text'?request.input.text:'';
    const textMs=(/[^\x00-\x7f]/.test(text)?80:40)*[...text].length;
    const timeout=setTimeout(abort,Math.min(14000,2000+textMs));
    try{return await response;}finally{clearTimeout(timeout);signal?.removeEventListener('abort',abort);}
  }
  async close(){
    if(this.closing)return this.closing;
    const child=this.child,identity=this.identity;
    if(!child)return;
    this.child=null;this.identity=null;
    this.ready?.reject(new Error('MP-08: physical key channel closed'));this.ready=null;
    for(const call of this.pending.values())call.reject(new Error('MP-08: physical key channel closed'));
    this.pending.clear();
    this.closing=settleOwned([{child,identity}]);
    try{await this.closing;}finally{this.closing=null;}
  }
}

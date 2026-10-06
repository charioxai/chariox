// MP-08 / MP-11: explicit placement, shared native helper, protected on-demand reads.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { processIdentity, signalOwned } from './linux-owned-process.mjs';
const helper = fileURLToPath(new URL('./native-computer.py', import.meta.url));
export function nativeInput(input, binding) {
  if (!input || typeof input !== 'object') throw new Error('MP-08: invalid native input');
  const bounded = (value, max) => Number.isInteger(value) && value >= 0 && value < max;
  const point = (x,y) => { if (!bounded(x,binding.width) || !bounded(y,binding.height)) throw new Error('MP-08: native input outside viewport'); };
  switch(input.kind) {
    case 'text': if (typeof input.text !== 'string' || !Buffer.byteLength(input.text) || Buffer.byteLength(input.text)>16384 || /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(input.text)) throw new Error('MP-08: invalid native text'); break;
    case 'keycode': if (!bounded(input.keycode-8,248) || !['down','up'].includes(input.state)) throw new Error('MP-08: invalid native keycode'); break;
    case 'key': if (typeof input.key !== 'string' || !/^[A-Za-z0-9_+]{1,128}$/.test(input.key)) throw new Error('MP-08: invalid native chord'); break;
    case 'hold': if (typeof input.key !== 'string' || !/^[A-Za-z0-9_+]{1,128}$/.test(input.key) || !Number.isInteger(input.duration_ms) || input.duration_ms<1 || input.duration_ms>10000) throw new Error('MP-08: invalid native hold'); break;
    case 'click': case 'move': point(input.x,input.y); if (input.kind==='click' && input.button !== undefined && ![1,2,3].includes(input.button)) throw new Error('MP-08: invalid button'); break;
    case 'drag': point(input.x,input.y);point(input.to_x,input.to_y);break;
    case 'scroll': point(input.x,input.y); if (!Number.isInteger(input.steps) || input.steps===0 || Math.abs(input.steps)>100) throw new Error('MP-08: invalid native scroll');break;
    case 'clipboard_write': if (typeof input.text!=='string' || Buffer.byteLength(input.text)>65536) throw new Error('MP-08: invalid clipboard');break;
    default: throw new Error('MP-08: unsupported native input');
  }
  return input;
}
export async function executeNative(request, environment, signal) {
  if(signal?.aborted) throw new Error('MP-11: native input cancelled');
  const child=spawn('/usr/bin/python3',[helper],{env:environment,stdio:['pipe','pipe','ignore']});
  child.on('error',()=>{});
  if(!child.pid) throw new Error('MP-08: native helper unavailable');
  const identity=await processIdentity(child.pid);
  if(!identity) throw new Error('MP-11: native helper ownership unavailable');
  let output='',overflow=false;
  child.stdout.on('data',chunk=>{output+=chunk;if(Buffer.byteLength(output)>6*1024*1024){overflow=true;void signalOwned(identity,'SIGKILL');}});
  child.stdin.on('error',()=>{});
  const cancel=()=>{void signalOwned(identity,'SIGTERM');};
  signal?.addEventListener('abort',cancel,{once:true});
  const timer=setTimeout(cancel,45000);
  const exit=new Promise(resolve=>{child.once('exit',code=>resolve(code));child.once('error',()=>resolve(-1));});
  try {
    if(signal?.aborted) cancel();
    child.stdin.end(JSON.stringify(request));
    const code=await exit;
    if(code!==0 || overflow || signal?.aborted) throw new Error('MP-08: native helper failed or cancelled');
    return JSON.parse(output);
  } finally { clearTimeout(timer);signal?.removeEventListener('abort',cancel); }
}
export class NativeComputer {
  constructor({placement,binding,execute=executeNative,wakeCapture=()=>{}}) {
    if(!['host','slice'].includes(placement)) throw new Error('MP-08: explicit native placement required');
    this.placement=placement;this.binding=binding;this.execute=execute;this.wakeCapture=wakeCapture;this.held=new Set();this.clipboard=null;
  }
  async close() {
    await this.reset();
    if (this.clipboard) {
      await signalOwned(this.clipboard, 'SIGTERM');
      this.clipboard=null;
    }
  }
  async writeClipboard(text, binding) {
    if(this.clipboard) await signalOwned(this.clipboard, 'SIGTERM');
    const child=spawn('xclip',['-selection','clipboard','-quiet','-loops','1'],{env:binding.environment,stdio:['pipe','ignore','ignore']});
    child.on('error',()=>{});child.stdin.on('error',()=>{});
    if(!child.pid)throw new Error('MP-08: clipboard helper unavailable');
    this.clipboard=await processIdentity(child.pid);
    if(!this.clipboard)throw new Error('MP-11: clipboard ownership unavailable');
    child.stdin.end(text);await delay(20);
    if(child.exitCode!==null || child.signalCode!==null)throw new Error('MP-08: clipboard write failed');
    return {applied:true};
  }
  async reset(signal) {
    const binding=this.binding();
    if(binding && this.held.size) await this.execute({op:'release',codes:[...this.held]},binding.environment,signal);
    this.held.clear();
  }
  async request(command,policy,{signal}={}) {
    const binding=this.binding();
    if(!binding) throw new Error('MP-08: native desktop unavailable; explicitly start');
    if(command.op==='state') return {surface_id:binding.surface_id,generation:binding.generation,width:binding.width,height:binding.height,placement:this.placement};
    if(command.surface_id!==binding.surface_id || command.generation!==binding.generation) throw new Error('MP-11: stale native desktop');
    if(policy?.unknown) throw new Error('MP-11: native observation protection unavailable');
    if(signal?.aborted) throw new Error('MP-11: native input cancelled');
    if(command.op==='input') {
      const input=nativeInput(command.input,binding);
      if(input.kind==='keycode') {
        if(input.state==='down') this.held.add(input.keycode);
        else if(!this.held.has(input.keycode)) throw new Error('MP-11: key release without owned press');
      }
      try {
        const result=input.kind==='clipboard_write' ? await this.writeClipboard(input.text,binding) : await this.execute({op:'input',input},binding.environment,signal);
        if(input.kind==='keycode' && input.state==='up') this.held.delete(input.keycode);
        // Display PR5 consumes this event to wake XDamage capture immediately.
        this.wakeCapture({surface_id:binding.surface_id,generation:binding.generation,exact:true,reason:input.kind});
        return result;
      } catch(error) {await this.reset();throw error;}
    }
    if(!['screenshot','ocr','clipboard_read'].includes(command.op)) throw new Error('MP-08: unsupported native observation');
    // Browser target transforms/non-browser secret coverage are not yet proven:
    // any registry protection masks the entire desktop, including OCR/clipboard.
    const mask=Boolean(policy?.values?.length || policy?.targets?.length);
    const result=await this.execute({op:command.op,mask,query:command.query},binding.environment,signal);
    if(signal?.aborted || this.binding()!==binding) throw new Error('MP-11: stale native observation');
    return {...result,surface_id:binding.surface_id,generation:binding.generation};
  }
}

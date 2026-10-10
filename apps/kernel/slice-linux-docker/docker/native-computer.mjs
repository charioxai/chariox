// MP-08 / MP-11: explicit placement, shared native helper, protected on-demand reads.
import { NativeKeyboardChannel } from './native-keyboard-channel.mjs';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { processIdentity, signalOwned, settleOwned } from './linux-owned-process.mjs';
import { UserDomainRefusal } from './kernel-browser-refusal.mjs';
import { fenceBrowserCapture } from './browser-protection-regions.mjs';
import { redactObservation } from './browser-controller-snapshot.mjs';
const helper = fileURLToPath(new URL('./native-computer.py', import.meta.url));
// MP-08/MP-10: human input the warm channel carries (no admission; see native-computer.py).
const WARM_INPUTS = new Set(['keycode','click','move','scroll','key','drag','text']);
export function nativeInput(input, binding) {
  if (!input || typeof input !== 'object') throw new Error('MP-08: invalid native input');
  const bounded = (value, max) => Number.isInteger(value) && value >= 0 && value < max;
  const point = (x,y) => { if (!bounded(x,binding.width) || !bounded(y,binding.height)) throw new Error('MP-08: native input outside viewport'); };
  switch(input.kind) {
    case 'composition': case 'text':
      if (typeof input.text !== 'string' || !Buffer.byteLength(input.text) || /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(input.text)) throw new Error('MP-08: invalid native text');
      if ([...input.text].length > 128) throw new Error('MP-08: native text exceeds the execution budget; split into blocks of at most 128 characters');
      break;
    case 'keycode': if (!bounded(input.keycode-8,248) || !['down','up'].includes(input.state)) throw new Error('MP-08: invalid native keycode'); break;
    case 'key': if (typeof input.key !== 'string' || !/^[A-Za-z0-9_+]{1,128}$/.test(input.key)) throw new Error('MP-08: invalid native chord'); break;
    case 'hold': if (typeof input.key !== 'string' || !/^[A-Za-z0-9_+]{1,128}$/.test(input.key) || !Number.isInteger(input.duration_ms) || input.duration_ms<1 || input.duration_ms>10000) throw new Error('MP-08: invalid native hold'); break;
    case 'pointer_hold': point(input.x,input.y);if(![1,2,3].includes(input.button) || !Number.isInteger(input.duration_ms) || input.duration_ms<1 || input.duration_ms>10000)throw new Error('MP-08: invalid native pointer hold');break;
    case 'click': case 'move': point(input.x,input.y); if (input.kind==='click' && input.button !== undefined && ![1,2,3].includes(input.button)) throw new Error('MP-08: invalid button'); break;
    case 'drag': point(input.x,input.y);point(input.to_x,input.to_y);if(input.button!==undefined && ![1,2,3].includes(input.button))throw new Error('MP-08: invalid button');break;
    case 'scroll': point(input.x,input.y); if (!Number.isInteger(input.steps) || input.steps===0 || Math.abs(input.steps)>100) throw new Error('MP-08: invalid native scroll');break;
    case 'clipboard_write': if (typeof input.text!=='string' || Buffer.byteLength(input.text)>65536) throw new Error('MP-08: invalid clipboard');break;
    default: throw new Error('MP-08: unsupported native input');
  }
  if(input.kind==='key' || input.kind==='hold') {
    // MP-08: chords name physical base keys; casing does not imply Shift.
    const names={ctrl:'ctrl',control:'ctrl',alt:'alt',shift:'shift',super:'super',meta:'super',enter:'Return',return:'Return',escape:'Escape',esc:'Escape',tab:'Tab',space:'space',backspace:'BackSpace',delete:'Delete',left:'Left',right:'Right',up:'Up',down:'Down',home:'Home',end:'End',pageup:'Prior',pagedown:'Next'};
    const key=input.key.split('+').map(name=>names[name.toLowerCase()]??(/^[A-Za-z]$/.test(name)?name.toLowerCase():/^f\d{1,2}$/i.test(name)?name.toUpperCase():name)).join('+');
    return {...input,key};
  }
  return input;
}
export async function executeNative(request, environment, signal) {
  if(signal?.aborted) throw new Error('MP-11: native input cancelled');
  const child=spawn('/usr/bin/python3',[helper],{env:environment,stdio:['pipe','pipe','pipe']});
  child.on('error',()=>{});
  if(!child.pid) throw new Error('MP-08: native helper unavailable');
  const identity=await processIdentity(child.pid);
  if(!identity) throw new Error('MP-11: native helper ownership unavailable');
  let output='',overflow=false,diagnostic='native_helper_failed';
  child.stderr.on('data',chunk=>{const match=/^MP-08: native operation ([A-Za-z_]{1,64})\s*$/.exec(String(chunk));if(match)diagnostic=match[1];});
  child.stdout.on('data',chunk=>{output+=chunk;if(Buffer.byteLength(output)>6*1024*1024){overflow=true;void signalOwned(identity,'SIGKILL');}});
  child.stdin.on('error',()=>{});
  const cancel=()=>{void signalOwned(identity,'SIGTERM');};
  signal?.addEventListener('abort',cancel,{once:true});
  const timer=setTimeout(cancel,45000);
  const exit=new Promise(resolve=>{child.once('close',code=>resolve(code));child.once('error',()=>resolve(-1));});
  try {
    if(signal?.aborted) cancel();
    child.stdin.end(JSON.stringify(request));
    const code=await exit;
    if(signal?.aborted)throw Object.assign(new Error('MP-11: native input cancelled'),{code:'browser_action_cancelled'});
    if(code!==0 && diagnostic==='NativeInputDenied' && !overflow)throw new UserDomainRefusal('sensitive_requires_focus');
    if(code!==0 || overflow) {
      // MP-08/MP-11: fixed private timing classes only; public errors remain generic.
      const codes={NativeProtectionChanged:'NATIVE_PROTECTION_CHANGED',ValueError:'NATIVE_VALUE_ERROR',TimeoutExpired:'NATIVE_HELPER_TIMEOUT',ModuleNotFoundError:'NATIVE_DEPENDENCY_MISSING',ImportError:'NATIVE_DEPENDENCY_MISSING',FileNotFoundError:'NATIVE_FILE_MISSING',OSError:'NATIVE_OS_ERROR'};
      throw Object.assign(new Error('MP-08: native helper failed ('+diagnostic+')'),{code:overflow?'NATIVE_OUTPUT_LIMIT':codes[diagnostic]});
    }
    return JSON.parse(output);
  } finally { clearTimeout(timer);signal?.removeEventListener('abort',cancel); }
}
export class NativeComputer {
  constructor({placement,binding,execute=executeNative,wakeCapture=()=>{},channel=execute===executeNative?environment=>new NativeKeyboardChannel(environment):null}) {
    if(!['host','slice'].includes(placement)) throw new Error('MP-08: explicit native placement required');
    this.placement=placement;this.binding=binding;this.execute=execute;this.wakeCapture=wakeCapture;this.channel=channel;this.held=new Set();this.clipboard=null;this.heldOwner=null;
  }
  async primeKeyboard() {
    if(!this.channel)return;
    const binding=this.binding();if(!binding)return;
    if(this.keyboard && this.keyboardBinding!==binding){
      await this.keyboard.close();this.keyboard=null;this.held.clear();this.heldOwner=null;
    }
    if(!this.keyboard){this.keyboard=this.channel(binding.environment);this.keyboardBinding=binding;}
    try{await this.keyboard.start();}catch(error){await this.keyboard.close();this.keyboard=null;throw error;}
  }
  async retire(observer) {
    if(this.heldOwner===observer || this.heldOwner?.startsWith(observer+':'))await this.reset();
  }
  async close() {
    try {await this.reset();}
    finally {await this.keyboard?.close();this.keyboard=null;this.keyboardBinding=null;this.held.clear();this.heldOwner=null;if (this.clipboard) {await settleOwned([this.clipboard]);this.clipboard=null;}}
  }
  async writeClipboard(text, binding) {
    if(this.clipboard) await settleOwned([this.clipboard]);
    const child=spawn('xclip',['-selection','clipboard','-quiet','-loops','0'],{env:binding.environment,stdio:['pipe','ignore','ignore']});
    child.on('error',()=>{});child.stdin.on('error',()=>{});
    if(!child.pid)throw new Error('MP-08: clipboard helper unavailable');
    const identity=await processIdentity(child.pid);
    this.clipboard={child,identity};
    if(!identity)throw new Error('MP-11: clipboard ownership unavailable');
    child.stdin.end(text);await delay(20);
    if(child.exitCode!==null || child.signalCode!==null)throw new Error('MP-08: clipboard write failed');
    return {applied:true};
  }
  async reset(signal) {
    const binding=this.binding();
    if(binding && this.held.size) {
      if(this.keyboard)await this.keyboard.send({op:'release',codes:[...this.held]},signal);
      else await this.execute({op:'release',codes:[...this.held]},binding.environment,signal);
    }
    this.held.clear();this.heldOwner=null;
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
      const observer=command.observed_by??'adapter';
      if(this.heldOwner && this.heldOwner!==observer)throw new Error('MP-11: native key owner conflict');
      if(command._agent_input && input.kind==='keycode')throw new Error('MP-11: persistent physical keys require human input; agents use focused single chords');
      // MP-11: native repeats and clipboard/middle-button paste cannot fence
      // every text-producing event. Agents use focused text or a single chord.
      if(command._agent_input && (['hold','pointer_hold','drag','clipboard_write'].includes(input.kind) || input.button===2))throw new UserDomainRefusal('sensitive_requires_focus');
      if(command._agent_input && (policy?.values?.length || policy?.targets?.length))throw new UserDomainRefusal('sensitive_requires_focus');
      // MP-11: the helper gates every agent key/text/click on the clipboard owner.
      const admission=command._agent_input ? {agent_input:true,processes:await binding.ownedProcesses?.()??[]} : {};
      if(input.kind==='keycode') {
        if(input.state==='down') {this.held.add(input.keycode);this.heldOwner=observer;}
        else if(!this.held.has(input.keycode)) throw new Error('MP-11: key release without owned press');
      }
      try {
        // MP-08/MP-10: human input keeps one warm, ordered helper; agent input
        // keeps the one-shot helper with its per-press admission.
        const dispatched=input.kind==='composition'?{kind:'text',text:input.text}:input;
        const warm=Boolean(this.channel)&&!command._agent_input&&WARM_INPUTS.has(dispatched.kind);
        let physical;
        if(warm){await this.primeKeyboard();physical=await this.keyboard.send({op:'input',input:dispatched},signal);}
        const result=warm ? physical : input.kind==='clipboard_write' ? await this.writeClipboard(input.text,binding) : await this.execute({op:'input',...admission,input:dispatched},binding.environment,signal);
        if(input.kind==='keycode' && input.state==='up') {this.held.delete(input.keycode);if(!this.held.size)this.heldOwner=null;}
        // Display PR5 consumes this event to wake XDamage capture immediately.
        this.wakeCapture({surface_id:binding.surface_id,generation:binding.generation,exact:true,reason:input.kind});
        return result;
      } catch(error) {await this.reset();throw error;}
    }
    if(!['screenshot','ocr','clipboard_read'].includes(command.op)) throw new Error('MP-08: unsupported native observation');
    // Vault (Miguel 2026-10-09): never black out the desktop. Kernel-browser
    // windows keep their CDP fill-target masks; native windows cover only the
    // exact AT-SPI plain entry filled from the Vault. A clipboard
    // read stays withheld while values or targets are registered.
    const values=policy?.values??[];
    const mask=command.op==='clipboard_read'&&Boolean(values.length || policy?.targets?.length);
    const browser_processes=await binding.browserProcesses?.();
    const processes=await binding.ownedProcesses?.()??[];
    const observe=browser_protection=>this.execute({op:command.op,mask,values,query:command.query,processes,...(browser_processes?{browser_processes}:{}),...(browser_protection?{browser_protection}:{})},binding.environment,signal);
    // MP-08/MP-11: kernel-browser windows reveal all but their protected regions
    // only for an unchanged, presented field measurement; uncertain placement retries.
    const browser=command.op==='clipboard_read'?null:binding.browser?.();
    // AT-SPI may expose a just-navigated document a moment after CDP does:
    // retry a capture whose kernel-browser window stayed unbound (withheld).
    let result;
    for(let attempt=0;attempt<3;attempt++){
      result=browser?await fenceBrowserCapture(browser,policy,observe):await observe(null);
      if(!browser||!result.browser_withheld||signal?.aborted)break;
      await delay(250);
    }
    delete result.browser_withheld;
    if(typeof result.text==='string')result.text=redactObservation(result.text,values);
    if(Array.isArray(result.targets))result.targets=result.targets.filter(target=>redactObservation(String(target?.text??''),values)===String(target?.text??''));
    if(signal?.aborted || this.binding()!==binding) throw new Error('MP-11: stale native observation');
    return {...result,surface_id:binding.surface_id,generation:binding.generation};
  }
}

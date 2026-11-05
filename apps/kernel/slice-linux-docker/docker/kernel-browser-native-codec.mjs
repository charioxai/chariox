// MP-08/MP-10/MP-11: prepare the default native H.264 encoder in kernel-owned state.
import path from 'node:path';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const setups=new Map(),probeArgs=['--display-native-codec-probe'];
const installer=fileURLToPath(new URL('./install-display-codec.py',import.meta.url));

function run(command,args,environment,timeoutMs){
 return new Promise((resolve,reject)=>{
  const child=spawn(command,args,{env:environment,stdio:['ignore','pipe','pipe']});
  let stdout='',stderr='',expired=false;
  const timer=setTimeout(()=>{expired=true;child.kill('SIGKILL');},timeoutMs);
  child.stdout.on('data',chunk=>{if(stdout.length<16_384)stdout+=chunk.toString();});
  child.stderr.on('data',chunk=>{if(stderr.length<16_384)stderr+=chunk.toString();});
  child.once('error',error=>{clearTimeout(timer);reject(error);});
  child.once('close',code=>{clearTimeout(timer);resolve({code,stdout,stderr,expired});});
 });
}

async function prepare({worker,python,root,environment,execute}){
 const initial=await execute(worker,probeArgs,environment,15_000);
 if(initial.expired)throw Error('codec probe timeout');
 if(initial.code===0)return undefined;
 const installed=await execute(python,[installer,'--root',root],environment,45_000);
 const codec=installed.stdout.trim(),expected=path.join(root,'libopenh264.so.8');
 if(installed.expired||installed.code!==0||codec!==expected)throw Error('codec install');
 const verified=await execute(worker,probeArgs,{...environment,CHARIOX_BROWSER_DISPLAY_OPENH264:codec},15_000);
 if(verified.expired||verified.code!==0)throw Error('codec probe');
 return codec;
}

export async function prepareNativeCodecEnvironment({worker,environment=process.env,python=environment.CHARIOX_BROWSER_DISPLAY_PYTHON||'python3',run:execute=run}={}){
 if(environment.CHARIOX_BROWSER_DISPLAY_OPENH264||environment.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER)return environment;
 const root=environment.CHARIOX_BROWSER_DISPLAY_CODEC_ROOT;
 if(!path.isAbsolute(worker??'')||!path.isAbsolute(root??''))throw Object.assign(Error('MP-08/MP-10: native browser display codec state is unavailable; retry after kernel setup'),{nativeStage:'native_codec_unavailable'});
 const key=`${worker}\0${root}\0${python}`;
 let setup=setups.get(key);
 if(!setup){
  setup=prepare({worker,python,root,environment,execute});
  setups.set(key,setup);
  setup.catch(()=>{if(setups.get(key)===setup)setups.delete(key);});
 }
 try{
  const codec=await setup;
  return codec?{...environment,CHARIOX_BROWSER_DISPLAY_OPENH264:codec}:environment;
 }catch{
  throw Object.assign(Error('MP-08/MP-10: native browser display codec setup failed; check network access and the compatible FFmpeg ABI, then retry'),{nativeStage:'native_codec_unavailable'});
 }
}

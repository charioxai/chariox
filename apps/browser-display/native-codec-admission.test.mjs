// MP-08/MP-10/MP-11: actual worker lifecycle, supplementary to hosted drills.
import test from 'node:test';
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,rm} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
const binary=process.env.MD_NATIVE_WORKER;
async function run(args,env,input){
 const child=spawn(binary,args,{env:{...process.env,...env},stdio:['pipe','pipe','pipe']});
 let stderr='';child.stderr.on('data',b=>stderr+=b);
 child.stdout.resume();child.stdin.on('error',()=>{});child.stdin.end(input);
 const timer=setTimeout(()=>{if(Number.isSafeInteger(child.pid)&&child.pid>1)child.kill('SIGKILL')},5000);
 try{return await new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',code=>resolve({code,stderr}))})}finally{clearTimeout(timer)}
}
for(const seam of ['encoder','decoder'])test(`MP-08/MP-10/MP-11 missing ${seam} refuses before capture`,{skip:!binary},async()=>{
 const root=await mkdtemp(path.join(os.tmpdir(),'display-codec-admission-'));
 try{
  const pool=path.join(root,'pool');await mkdir(pool,{mode:0o700});
  const env={DISPLAY:':65530',CHARIOX_BROWSER_DISPLAY_SOFTWARE:'1',CHARIOX_BROWSER_DISPLAY_OPENH264:seam==='encoder'?'/nonexistent/openh264.so':process.env.CHARIOX_BROWSER_DISPLAY_OPENH264};
  if(seam==='decoder'){assert(process.env.MD_MISSING_DECODER_LIBS);env.LD_LIBRARY_PATH=process.env.MD_MISSING_DECODER_LIBS}
  const result=await run(['--display-native-worker'],env,JSON.stringify({pid:process.pid,width:1280,height:800,pool})+'\n');
  assert.equal(result.code,1);assert.match(result.stderr,/native stage codec_unavailable/);
 }finally{await rm(root,{recursive:true,force:true})}
});
test('MP-08/MP-10 codec probe opens the real default encoder and decoder',{skip:!binary},async()=>{
 const result=await run(['--display-native-codec-probe'],{CHARIOX_BROWSER_DISPLAY_SOFTWARE:'1'},'');
 assert.equal(result.code,0,result.stderr);
});

// MP-08/MP-10/MP-11: native readback setup provisions OpenH264 once and retries failures.
import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import {prepareNativeCodecEnvironment} from './kernel-browser-native-codec.mjs';

test('MP-08/MP-10 installs a missing default codec before retrying the native worker probe and reuses it',async()=>{
 const calls=[],environment={CHARIOX_BROWSER_DISPLAY_CODEC_ROOT:'/kernel/state/browser/display-codecs'};
 const run=async(command,args,env)=>{
  calls.push({command,args,env});
  if(args[0]==='--display-native-codec-probe')return {code:env.CHARIOX_BROWSER_DISPLAY_OPENH264?0:1,stdout:'',stderr:''};
  return {code:0,stdout:'/kernel/state/browser/display-codecs/libopenh264.so.8\n',stderr:''};
 };
 const options={worker:'/kernel/chariox-kernel',python:'python3',environment,run};
 const result=await prepareNativeCodecEnvironment(options);
 assert.equal(result.CHARIOX_BROWSER_DISPLAY_OPENH264,'/kernel/state/browser/display-codecs/libopenh264.so.8');
 assert.deepEqual([calls[0].args[0],path.basename(calls[1].args[0]),calls[2].args[0]],['--display-native-codec-probe','install-display-codec.py','--display-native-codec-probe']);
 assert.equal(path.basename(calls[1].args[0]),'install-display-codec.py');
 assert.deepEqual(calls[1].args.slice(1),['--root','/kernel/state/browser/display-codecs']);
 assert.equal(environment.CHARIOX_BROWSER_DISPLAY_OPENH264,undefined,'the parent environment stays untouched');
 assert.equal((await prepareNativeCodecEnvironment(options)).CHARIOX_BROWSER_DISPLAY_OPENH264,result.CHARIOX_BROWSER_DISPLAY_OPENH264);
 assert.equal(calls.length,3,'a warm process reuses the successful probe and install');
});

test('MP-08/MP-10 explicit library and encoder overrides skip codec setup',async()=>{
 for(const environment of [
  {CHARIOX_BROWSER_DISPLAY_OPENH264:'/owned/custom/libopenh264.so.8'},
  {CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER:'libx264'},
 ]){
  let calls=0;
  assert.equal(await prepareNativeCodecEnvironment({worker:'/kernel/worker',environment,run:async()=>{calls++;throw Error('unexpected codec setup')}}),environment);
  assert.equal(calls,0);
 }
});

test('MP-08/MP-10 a codec already found by the native worker avoids installation',async()=>{
 const environment={CHARIOX_BROWSER_DISPLAY_CODEC_ROOT:'/kernel/state/display-codecs'};
 const run=async(command,args)=>({code:args[0]==='--display-native-codec-probe'?0:1,stdout:'',stderr:''});
 assert.equal(await prepareNativeCodecEnvironment({worker:'/kernel/worker',environment,run}),environment);
});

test('MP-08/MP-10 setup failures reject clearly and can be retried',async()=>{
 let calls=0;
 const options={worker:'/kernel/worker',python:'python3',environment:{CHARIOX_BROWSER_DISPLAY_CODEC_ROOT:'/kernel/state/retry-codecs'},run:async(command,args)=>{
  calls++;
  return args[0]==='--display-native-codec-probe'?{code:1,stdout:'',stderr:'missing codec'}:{code:1,stdout:'',stderr:'download failed'};
 }};
 await assert.rejects(prepareNativeCodecEnvironment(options),/native browser display codec setup failed/);
 const afterFirst=calls;
 await assert.rejects(prepareNativeCodecEnvironment(options),/native browser display codec setup failed/);
 assert.equal(calls,afterFirst*2,'a failed setup is not cached as a permanently rejected promise');
});

// MP-08/MP-10/MP-11: supplementary Room capture routing and publication guards.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,readFile,rm,readdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {captureRoomObservation} from './browser-observation-regions.mjs';
const empty={unknown:false,values:[],targets:[]};
test('MP-11 Room screenshot/OCR use the scoped measured adapter without typed-field discovery',async()=>{
 const scratch=await mkdtemp(path.join(tmpdir(),'room-capture-test-'));
 try {
  const browser={ensureConnection:async()=>{throw Error('must not discover typed fields');}};
  const destination=path.join(scratch,'owner-output.png'),calls=[];
  const execute=async(file,args,{input})=>{
   const request=JSON.parse(input);calls.push({file,args,request});
   assert.deepEqual(request.browser_protection,{pages:[]});
   assert.deepEqual(request.policy,empty);
   if(args[2]==='screenshot')await writeFile(args[3],'masked-pixels');
   return {stdout:args[2]==='screenshot'?'':'public OCR',stderr:''};
  };
  await captureRoomObservation(browser,empty,'screenshot',destination,{execute,scratch});
  assert.equal(await readFile(destination,'utf8'),'masked-pixels');
  const ocr=await captureRoomObservation(browser,empty,'ocr',null,{execute,scratch});
  assert.equal(ocr.stdout,'public OCR');assert.equal(calls.length,2);
  for(const call of calls)assert.equal(call.args[1],'--measured');
  assert.deepEqual(await readdir(scratch),['owner-output.png']);
 }finally{await rm(scratch,{recursive:true,force:true});}
});
test('MP-11 changed Room browser fences discard both images and OCR before publication',async()=>{
 const scratch=await mkdtemp(path.join(tmpdir(),'room-fence-test-'));
 try {
  for(const mode of ['screenshot','ocr']) {
   let measures=0,captures=0;
   const browser={ensureConnection:async()=>({send:async()=>{
    if(++measures>1)throw Error('document changed');return {targetInfos:[]};
   }})};
   const policy={...empty,targets:[{kind:'browser',value_hash:'public-hash',target_id:'filled'}]};
   const destination=path.join(scratch,'never-published.png');
   await assert.rejects(captureRoomObservation(browser,policy,mode,destination,{scratch,execute:async(_file,args)=>{
    captures++;if(mode==='screenshot')await writeFile(args[3],'unverified-pixels');
    return {stdout:'unverified OCR',stderr:''};
   }}),/capture fence unavailable/);
   assert.equal(captures,1);assert.deepEqual(await readdir(scratch),[]);
  }
 }finally{await rm(scratch,{recursive:true,force:true});}
});

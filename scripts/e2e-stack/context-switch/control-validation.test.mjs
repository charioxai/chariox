// MP-08 / MP-10 / MP-11: immediate failures and foreign-account thread reuse must fail acceptance.
import test from 'node:test';import assert from 'node:assert/strict';
import {validateCompactionOutage,validateAccountNativeSwitch} from './control-validation.mjs';
const event={message:'failed to compact the session before a smaller context window',errorClass:'timeout',fields:{elapsed_ms:300001,provider_run_id:'run-1',source_model:'claude-sonnet-5-5[1m]',target_model:'claude-haiku-4-5'}};
const input={baseline:[],events:[event],elapsedMs:308000,sourceRunId:'run-1',sourceModel:'claude-sonnet-5-5[1m]',targetModel:'claude-haiku-4-5'};
test('MP-08 / MP-10 / MP-11 immediate transport failure cannot pass300s deadline',()=>{
 assert.throws(()=>validateCompactionOutage({...input,events:[{...event,errorClass:'other-error',fields:{...event.fields,elapsed_ms:100}}],elapsedMs:110}),/timeout|deadline/i);
});
test('MP-08 / MP-10 / MP-11 shortened deadline and missing clock cannot pass',()=>{
 for(const elapsed of [1000,120000,undefined,NaN])assert.throws(()=>validateCompactionOutage({...input,events:[{...event,fields:{...event.fields,elapsed_ms:elapsed}}]}),/deadline|elapsed/i);
});
test('MP-08 / MP-10 / MP-11 timeout must belong to the source run',()=>{
 assert.throws(()=>validateCompactionOutage({...input,events:[{...event,fields:{...event.fields,provider_run_id:'other-run'}}]}),/source/i);
 assert.equal(validateCompactionOutage(input),event);
});
const account={before:{nativeId:'a1',accountProfile:'account-a'},target:{nativeId:'b1',accountProfile:'account-b'},after:{nativeId:'a2',accountProfile:'account-a'},primary:'account-a',secondary:'account-b'};
test('MP-08 / MP-10 / MP-11 account switches require fresh native IDs on both legs',()=>{
 assert.throws(()=>validateAccountNativeSwitch({...account,target:{...account.target,nativeId:'a1'}}),/native/i);
 assert.throws(()=>validateAccountNativeSwitch({...account,after:{...account.after,nativeId:'b1'}}),/native/i);
 assert.throws(()=>validateAccountNativeSwitch({...account,after:{...account.after,nativeId:'a1'}}),/native/i);
 assert.doesNotThrow(()=>validateAccountNativeSwitch(account));
});

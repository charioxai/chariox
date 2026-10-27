// MP-08/MP-10/MP-11: Unicode has mapping-settle and physical-delivery costs.
import assert from 'node:assert/strict';
import test from 'node:test';
import {hostFailureMessage} from './kernel-browser-error-label.mjs';
import {NativeKeyboardChannel,nativeKeyboardStartupFailure} from './native-keyboard-channel.mjs';

for(const [name,text,expected] of [['ASCII','x'.repeat(128),7120],['Unicode','漢字🧪'.repeat(42)+'漢字',12240]]) {
  test(`MP-08/MP-10/MP-11 ${name} warm text retains a bounded deadline covering all 128 characters`,async t=>{
    const channel=new NativeKeyboardChannel({});
    channel.start=async()=>{};
    channel.child={stdin:{write(line){
      const {id}=JSON.parse(line);
      queueMicrotask(()=>{const call=channel.pending.get(id);channel.pending.delete(id);call.resolve({applied:true});});
    }}};
    const original=globalThis.setTimeout,delays=[];
    t.mock.method(globalThis,'setTimeout',(callback,delay,...args)=>{delays.push(delay);return original(callback,delay,...args);});
    assert.deepEqual(await channel.send({op:'input',input:{kind:'text',text}}),{applied:true});
    assert.deepEqual(delays,[expected]);
    assert(expected<20000,'physical execution stays within the native RPC bound');
  });
}

// MP-08/MP-10/MP-11 real fresh-host Pillow absence must survive the public boundary.
test('MP-08/MP-10/MP-11 keyboard startup names fixed missing dependencies without exposing stderr',()=>{
 const e=nativeKeyboardStartupFailure("Traceback private-canary /private/profile\nModuleNotFoundError: No module named 'PIL'\n");
 assert.equal(e.code,'NATIVE_DEPENDENCY_MISSING');
 assert.match(hostFailureMessage(e),/native_keyboard_pillow_missing/);
 assert(!hostFailureMessage(e).includes('private-canary'));
 const other=nativeKeyboardStartupFailure("ModuleNotFoundError: No module named 'private-canary'\n");
 assert.match(hostFailureMessage(other),/native_keyboard_unavailable/);
 assert(!hostFailureMessage(other).includes('private-canary'));
 assert.match(hostFailureMessage(new Error('MP-08: physical key readiness timeout')),/native_keyboard_readiness_timeout/);
});

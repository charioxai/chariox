// MP-11: supplementary diagnostics privacy regression.
import test from 'node:test';import assert from 'node:assert/strict';
import {hostErrorLabel} from './kernel-browser-error-label.mjs';
test('MP-11 ordinary unprefixed failures retain public source location without private data',()=>{
 const error={name:'TypeError',code:'ENOENT',message:'private page value',stack:'TypeError: private page value\n at f (/private/credential-profile/kernel-browser-native-worker.mjs:17:20)'};
 assert.equal(hostErrorLabel(error),'host_error TypeError ENOENT kernel-browser-native-worker.mjs:17:20');
 assert(!hostErrorLabel(error).includes('private'));
});
test('MP-11 attacker-controlled exception fields cannot become timing labels',()=>{
 const error={name:'protectedcanary',code:'protectedcanary',message:'protectedcanary',stack:'protectedcanary\n at f (/protectedcanary/protectedcanary.mjs:1:1)'};
 assert.equal(hostErrorLabel(error),'host_error Error');assert.equal(hostErrorLabel(null),'host_error Error');
});

test('MP-08/MP-11 native capture failures retain fixed diagnostic codes and public positions',()=>{
 const error={name:'Error',code:'NATIVE_PROTECTION_CHANGED',message:'private native value',stack:'Error: private native value\n at f (/private/profile/native-computer.mjs:59:20)'};
 assert.equal(hostErrorLabel(error),'host_error Error NATIVE_PROTECTION_CHANGED native-computer.mjs:59:20');
 error.code='private-native-value';
 assert.equal(hostErrorLabel(error),'host_error Error native-computer.mjs:59:20');
});

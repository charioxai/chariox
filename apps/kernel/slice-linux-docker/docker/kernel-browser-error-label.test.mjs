// MP-11: supplementary diagnostics privacy regression.
import test from 'node:test';import assert from 'node:assert/strict';
import {hostErrorLabel,mirrorExceptionReason,hostFailureMessage} from './kernel-browser-error-label.mjs';
test('MP-11 ordinary unprefixed failures retain public source location without private data',()=>{
 const error={name:'TypeError',code:'ENOENT',message:'private page value',stack:'TypeError: private page value\n at f (/private/credential-profile/kernel-browser-native-worker.mjs:17:20)'};
 assert.equal(hostErrorLabel(error),'host_error TypeError ENOENT kernel-browser-native-worker.mjs:17:20');
 assert(!hostErrorLabel(error).includes('private'));
});
test('MP-11 attacker-controlled exception fields cannot become timing labels',()=>{
 const error={name:'protectedcanary',code:'protectedcanary',message:'protectedcanary',stack:'protectedcanary\n at f (/protectedcanary/protectedcanary.mjs:1:1)'};
 assert.equal(hostErrorLabel(error),'host_error Error');assert.equal(hostErrorLabel(null),'host_error Error');
});
test('MP-08/MP-10/MP-11 CDP timeouts identify the public method without its arguments',()=>{
 assert.equal(hostErrorLabel({code:'browser_cdp_timeout',message:'browser CDP method Page.captureScreenshot timed out after 5000ms'}),'host_error Error browser_cdp_timeout Page.captureScreenshot');
 assert.equal(hostErrorLabel({code:'browser_cdp_timeout',message:'browser CDP method protectedcanary timed out after 5000ms'}),'host_error Error browser_cdp_timeout');
});

test('MP-08/MP-10/MP-11: observer diagnostics admit fixed input refusal classes only',()=>{
 assert.equal(mirrorExceptionReason('Error: mirror2 invalid selection\n at private'), 'invalid_selection');
 assert.equal(mirrorExceptionReason('Error: mirror2 unavailable native text focus\n at private'), 'unavailable_native_text_focus');
 assert.equal(mirrorExceptionReason('Error: mirror2 private-canary'), 'observer_exception');
 assert.equal(hostErrorLabel({mirrorReason:'invalid_selection'}),'host_error Error invalid_selection');
 assert.equal(hostErrorLabel({mirrorReason:'private-canary'}),'host_error Error');
});

test('MP-08/MP-10/MP-11: kernel-owned mirror fences have fixed diagnostic names',()=>{
 assert.equal(hostErrorLabel({message:'MP-11: changed mirror input target'}),'host_error Error input_target_changed');
 assert.equal(hostErrorLabel({message:'MP-11: changed mirror input target private-canary'}),'host_error Error');
});

test('MP-08/MP-11 native capture failures retain fixed diagnostic codes and public positions',()=>{
 const error={name:'Error',code:'NATIVE_PROTECTION_CHANGED',message:'private native value',stack:'Error: private native value\n at f (/private/profile/native-computer.mjs:59:20)'};
 assert.equal(hostErrorLabel(error),'host_error Error NATIVE_PROTECTION_CHANGED native-computer.mjs:59:20');
 error.code='private-native-value';
 assert.equal(hostErrorLabel(error),'host_error Error native-computer.mjs:59:20');
});
// MP-08/MP-10/MP-11: the public error uses the same safe classifier as diagnostics.
test('public Browser failures distinguish timeout, stale target, observer refusal and unexpected source',()=>{
 const timeout={code:'browser_cdp_timeout',message:'browser CDP method Page.captureScreenshot timed out after 5000ms'};
 assert.match(hostFailureMessage(timeout),/browser_cdp_timeout Page.captureScreenshot/);
 assert.match(hostFailureMessage({message:'MP-11: stale or foreign mirror'}),/mirror_subscription_retired/);
 assert.match(hostFailureMessage({mirrorReason:'occluded_node'}),/occluded_node/);
 const unexpected={name:'TypeError',message:'private-canary',stack:'TypeError: private-canary\n at apply (/private-canary/kernel-browser-host.mjs:17:20)'};
 assert.match(hostFailureMessage(unexpected),/TypeError kernel-browser-host.mjs:17:20/);
 assert(!hostFailureMessage(unexpected).includes('private-canary'));
 assert(!hostFailureMessage({code:'private-canary',mirrorReason:'private-canary',message:'private-canary'}).includes('private-canary'));
});

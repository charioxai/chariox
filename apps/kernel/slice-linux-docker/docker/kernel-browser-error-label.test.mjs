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
test('MP-08/MP-10/MP-11 CDP timeouts identify the public method without its arguments',()=>{
 assert.equal(hostErrorLabel({code:'browser_cdp_timeout',message:'browser CDP method Page.captureScreenshot timed out after 5000ms'}),'host_error Error browser_cdp_timeout Page.captureScreenshot');
 assert.equal(hostErrorLabel({code:'browser_cdp_timeout',message:'browser CDP method protectedcanary timed out after 5000ms'}),'host_error Error browser_cdp_timeout');
});

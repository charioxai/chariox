// MP-11: supplementary diagnostics privacy regression.
import test from 'node:test';import assert from 'node:assert/strict';
import {hostErrorLabel,mirrorExceptionReason,hostFailureMessage,browserLaunchFailureReason} from './kernel-browser-error-label.mjs';
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

// MP-08/MP-10/MP-11: observed on the fresh Ubuntu24 quiet host, before CDP exists.
test('launch failures retain specific safe namespace, sandbox and executable causes',()=>{
 for(const [stderr,spawnCode,reason]of [
  ['unshare: write failed /proc/self/uid_map: Operation not permitted\n',undefined,'browser_namespace_permission_denied'],
  ['unshare: unshare failed: Operation not permitted',undefined,'browser_namespace_permission_denied'],
  ['mount: /etc: permission denied.',undefined,'browser_policy_mount_denied'],
  ['No usable sandbox! private-canary',undefined,'browser_sandbox_unavailable'],
  ['Missing X server or $DISPLAY',undefined,'browser_display_unavailable'],
  ['', 'ENOENT','browser_executable_missing'],
  ['', 'EACCES','browser_executable_permission_denied'],
  ['private-canary', 'private-canary','browser_launch_failed'],
 ]){
  const code=browserLaunchFailureReason(stderr,spawnCode);assert.equal(code,reason);
  const reply=hostFailureMessage({code,message:stderr});assert(reply.includes(reason));assert(!reply.includes('private-canary'));assert(!reply.includes('/proc/'));
 }
});

test('MP-08/MP-10/MP-11 navigation and detached CDP failures preserve safe causes without page descriptions',()=>{
 assert.match(hostFailureMessage({code:'browser_navigation_failed',message:'browser navigation failed: net::ERR_NAME_NOT_RESOLVED'}),/browser_navigation_failed net::ERR_NAME_NOT_RESOLVED/);
 assert.match(hostFailureMessage({code:'browser_cdp_command_failed',message:'Runtime.callFunctionOn: Cannot find context with specified id'}),/browser_cdp_command_failed Runtime.callFunctionOn execution_context_retired/);
 const privateFailure=hostFailureMessage({code:'browser_navigation_failed',message:'browser navigation failed: private-canary https://private.invalid'});
 assert(privateFailure.includes('browser_navigation_failed'));assert(!privateFailure.includes('private-canary'));assert(!privateFailure.includes('private.invalid'));
 assert(!hostFailureMessage({code:'browser_cdp_command_failed',message:'Runtime.callFunctionOn: private-canary'}).includes('private-canary'));
});

test('MP-08/MP-10/MP-11 normalized controller failures retain declared public codes',()=>{
 for(const code of ['browser_document_identity_missing','snapshot_frame_limit','browser_history_unavailable','browser_permission_origin_denied']){
  const reply=hostFailureMessage({code,message:'private-canary',stack:'Error: private-canary\n at normalize (/private/browser-controller-cdp.mjs:72:22)'});
  assert(reply.includes(code));assert(!reply.includes('private-canary'));assert(!reply.includes('/private/'));
 }
 assert.match(hostFailureMessage({message:'MD-N2: frame changed during capture'}),/note_frame_changed/);
});

// MP-08/MP-10/MP-11 the real private desktop failed before Chromium existed on the fresh host.
test('owned desktop failures retain fixed prerequisite and readiness causes privately',()=>{
 for(const name of ['bwrap','Xvfb','dbus-daemon','openbox','setxkbmap'])assert.match(hostFailureMessage({message:`MP-08: owned desktop prerequisite unavailable (${name})`}),new RegExp('desktop_prerequisite_'+name.toLowerCase().replaceAll('-','_')+'_unavailable'));
 assert.match(hostFailureMessage({message:'MP-08: desktop child exited before readiness'}),/desktop_child_exited_before_readiness/);
 assert.match(hostFailureMessage({message:'MP-08: AT-SPI bus configuration unavailable'}),/desktop_at_spi_configuration_unavailable/);
 assert.match(hostFailureMessage({message:'MP-08: desktop failed to become ready (openbox, exit 1)'}),/desktop_openbox_failed_to_start/);
 for(const message of ['MP-08: owned desktop prerequisite unavailable (private-canary)','MP-08: desktop failed to become ready (private-canary, exit 1)'])assert(!hostFailureMessage({message}).includes('private-canary'));
});

// MP-11: supplementary diagnostics privacy regression.
import test from 'node:test';import assert from 'node:assert/strict';
import {hostErrorLabel,mirrorExceptionReason,hostFailureMessage,browserLaunchFailureReason} from './kernel-browser-error-label.mjs';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {nativeReadbackFailure} from './kernel-browser-native-readback.mjs';
test('MP-11 ordinary unprefixed failures retain public source location without private data',()=>{
 const error={name:'TypeError',code:'ENOENT',message:'private page value',stack:'TypeError: private page value\n at f (/private/credential-profile/kernel-browser-native-worker.mjs:17:20)'};
 assert.equal(hostErrorLabel(error),'host_error TypeError ENOENT kernel-browser-native-worker.mjs:17:20');
 assert(!hostErrorLabel(error).includes('private'));
});
test('MP-11 attacker-controlled exception fields cannot become timing labels',()=>{
 const error={name:'protectedcanary',code:'protectedcanary',message:'protectedcanary',stack:'protectedcanary\n at f (/protectedcanary/protectedcanary.mjs:1:1)'};
 assert.equal(hostErrorLabel(error),'host_error Error');assert.equal(hostErrorLabel(null),'host_error Error');
});
test('MP-08/MP-10/MP-11 a CDP timeout names the waiting caller, not the timer inside the CDP client',()=>{
 const error={name:'Error',code:'browser_cdp_timeout',message:'browser CDP method Page.getFrameTree timed out after 5000ms',stack:'Error\n at Timeout._onTimeout (/x/browser-controller-cdp.mjs:1358:11)\n at send (/x/browser-controller-cdp.mjs:1350:12)\n at resolveDisplayTarget (/x/kernel-browser-display-document.mjs:93:60)\n at request (/x/kernel-browser-host.mjs:700:10)'};
 assert.equal(hostErrorLabel(error),'host_error Error browser_cdp_timeout Page.getFrameTree kernel-browser-display-document.mjs:93:60 < kernel-browser-host.mjs:700:10');
});
test('MP-08/MP-10/MP-11 CDP timeouts identify the public method without its arguments',()=>{
 assert.equal(hostErrorLabel({code:'browser_cdp_timeout',message:'browser CDP method Page.captureScreenshot timed out after 5000ms'}),'host_error Error browser_cdp_timeout Page.captureScreenshot');
 assert.equal(hostErrorLabel({code:'browser_cdp_timeout',message:'browser CDP method protectedcanary timed out after 5000ms'}),'host_error Error browser_cdp_timeout');
 for(const method of ['Emulation.setFocusEmulationEnabled','Target.getTargetInfo','Page.bringToFront'])assert.equal(hostErrorLabel({code:'browser_cdp_timeout',message:`browser CDP method ${method} timed out after 5000ms`}),`host_error Error browser_cdp_timeout ${method}`);
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

// MP-08/MP-10/MP-11: real BBC fallback failures were collapsed to the host dispatch frame.
test('native region refusals and v2 module positions survive the public Browser reply',()=>{
 for(const [message,reason]of [
  ['MP-11: native region lease retired','native_region_lease_retired'],
  ['MP-11: native region attestation geometry','native_region_attestation_geometry'],
  ['MP-11: native region attestation masked','native_region_attestation_masked'],
  ['MP-11: native region attestation mismatch','native_region_attestation_mismatch'],
  ['MP-08: native region readback unavailable','native_region_readback_unavailable'],
  ['MP-11: native keys held; release desktop input first','native_keys_held'],
 ])assert(hostFailureMessage({message}).includes(reason));
 for(const name of ['kernel-browser-window-region.mjs','kernel-browser-display-regions.mjs','kernel-browser-chrome-window.mjs','kernel-browser-mirror2.mjs']){
  const reply=hostFailureMessage({message:'private-canary',stack:`Error: private-canary\n at source (/private-profile/${name}:95:13)\n at dispatch (/private-profile/kernel-browser-host.mjs:926:24)`});
  assert(reply.includes(name+':95:13'));assert(!reply.includes('private'));
 }
 assert(!hostFailureMessage({message:'MP-11: native region attestation mismatch private-canary'}).includes('private-canary'));
});

// MP-08/MP-10/MP-11: the private native worker reports a fixed stage before it exits.
test('MP-08/MP-10/MP-11 native first-frame failures retain the worker cause without arbitrary text',()=>{
 const failure=nativeReadbackFailure({helperStage:'native_codec_unavailable'},'MP-08: native region readback unavailable');
 assert.match(hostFailureMessage(failure),/native_worker_native_codec_unavailable/);
 assert.match(hostFailureMessage(nativeReadbackFailure({helperStage:'window_not_found'},'MD-DISPLAY: native readback unavailable')),/native_worker_window_not_found/);
 assert(!hostFailureMessage({...failure,nativeStage:'private-canary'}).includes('private-canary'));
 assert(!hostFailureMessage({...failure,nativeStage:'native_codec_unavailable private-canary'}).includes('private-canary'));
});

test('MP-08/MP-10/MP-11 Notes observer causes and bounded code positions survive the host envelope',()=>{
 const reply=hostFailureMessage(Object.assign(Error('MD-N2: selection observation failed'),{code:'note_observer_type_error',noteObserverLine:42}));
 assert.match(reply,/note_observer_type_error note_observer:42/);
 assert.match(hostFailureMessage({code:'note_shadow_root_limit',message:'MD-N2: selection observation failed'}),/note_shadow_root_limit/);
 for(const noteObserverLine of [-1,0,257,NaN,'private-canary'])assert(!hostFailureMessage({code:'note_observer_type_error',noteObserverLine}).includes('note_observer:'));
 assert(!hostFailureMessage({code:'private-canary',noteObserverLine:42,message:'private-canary'}).includes('private-canary'));
});

// MP-08/MP-10/MP-11: observed generic native-band/region errors on the live BBC path.
test('MP-08/MP-10/MP-11 host replies retain closed user-domain and display-region refusals',async()=>{
 const {UserDomainRefusal}=await import('./kernel-browser-refusal.mjs');
 for(const reason of ['not_focused_agent','foreign_owner','stale_epoch','stale_reference','not_granted','sensitive_requires_focus']){
  const error=new UserDomainRefusal(reason);assert(hostFailureMessage(error).includes(error.code));
 }
 assert.match(hostFailureMessage(Error('MP-11: region lease retired')),/native_region_lease_retired/);
 for(const error of [{code:'user_domain_private_canary'},{message:'MP-11: region lease retired private-canary'}])assert(!hostFailureMessage(error).includes('private'))
});

test('MIRROR-ROBUST-STALE-REF expired references are fixed race labels with their public refusal code',()=>{
 assert.equal(hostErrorLabel(new UserDomainRefusal('stale_reference','region_retired')),'navigation_race region_retired');
 assert.equal(hostErrorLabel(new UserDomainRefusal('stale_reference','display_suspended')),'navigation_race display_suspended');
 assert.equal(hostErrorLabel({name:'Error',code:'stale_document_reference'}),'navigation_race stale_document');
 assert.equal(hostErrorLabel(new UserDomainRefusal('stale_reference')),'navigation_race user_domain_stale_reference');
 assert.equal(hostErrorLabel(new UserDomainRefusal('stale_epoch')),'navigation_race user_domain_stale_epoch');
 assert.equal(hostErrorLabel(Object.assign(new UserDomainRefusal('stale_reference'),{race:'protectedcanary'})).includes('protectedcanary'),false);
});


test('MP-08/MP-10 retired display diagnostics keep their navigation race label in replies',()=>{
 const error=new UserDomainRefusal('stale_reference','display_retired');
 assert.equal(hostErrorLabel(error),'navigation_race display_retired');
 assert.match(hostFailureMessage(error),/\(navigation_race display_retired\)$/);
});

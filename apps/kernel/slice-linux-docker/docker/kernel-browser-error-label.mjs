// MP-11: opt-in diagnostics identify public code, never exception/page values.
const names=new Set(['Error','TypeError','RangeError','ReferenceError','SyntaxError']);
const files=new Set(['kernel-browser-host.mjs','kernel-browser-process.mjs','kernel-browser-owned-display.mjs','kernel-browser-linux.mjs','kernel-browser-display.mjs','kernel-browser-motion.mjs','kernel-browser-native.mjs','kernel-browser-native-worker.mjs','kernel-browser-native-pipe.mjs','kernel-browser-native-credit.mjs','kernel-browser-refiner.mjs','kernel-browser-pixels.mjs','kernel-browser-pixel-worker.mjs','kernel-browser-display-capture.mjs','kernel-browser-display-credit.mjs','browser-controller-cdp.mjs','browser-controller-actions.mjs','kernel-browser-mirror2.mjs']);
const codes=new Set(['ENOENT','EACCES','EINVAL','EPIPE','ETIMEDOUT','ERR_BUFFER_OUT_OF_BOUNDS','ERR_OUT_OF_RANGE','browser_cdp_timeout','browser_cdp_command_failed','browser_cdp_disconnected','browser_cdp_socket_error','browser_target_not_found','browser_connection_changed','browser_cdp_response_too_large','browser_action_cancelled','browser_namespace_permission_denied','browser_policy_mount_denied','browser_sandbox_unavailable','browser_display_unavailable','browser_executable_missing','browser_executable_permission_denied','browser_launch_failed']);
const methods=new Set(['Page.captureScreenshot','Page.getFrameTree','Page.getLayoutMetrics','Runtime.evaluate','Runtime.callFunctionOn','DOM.resolveNode','CSS.enable','DOMSnapshot.captureSnapshot']);
// MP-11: classify only our fixed isolated-observer refusals; never print a
// CDP exception description, page exception, input value or private path.
const observerReasons=new Set(['protected_input_ancestor','stale_node','frame_unavailable','transformed_frame','element_required','invisible_node','occluded_node','offscreen_node','changed_live_target','unavailable_native_text_focus','focus_redirected','control_unavailable','protected_control','control_state_refused','invalid_selection','selection_intersects_protected_content','frame_required','observer_exception']);
export function mirrorExceptionReason(description) {
 const first=typeof description==='string'?description.split('\n',1)[0]:'';
 const reason=/^Error: mirror2 ([a-z ]+)$/.exec(first)?.[1].replaceAll(' ','_');
 return observerReasons.has(reason)?reason:'observer_exception';
}
const inputReasons=new Map([
 ['MP-11: stale or foreign mirror','mirror_subscription_retired'],
 ['MP-11: stale mirror document','mirror_document_stale'],
 ['MP-11: stale mirror protection policy or subscription','mirror_policy_stale'],
 ['MP-11: stale mirror protection policy','mirror_policy_stale'],
 ['MP-11: mirror isolated world unavailable','mirror_world_unavailable'],
 ['MP-11: mirror observer unavailable','mirror_observer_unavailable'],
 ['MP-11: frame observer unavailable','mirror_frame_observer_unavailable'],
 ['MP-11: mirror observation unavailable','mirror_observation_unavailable'],
 ['MP-11: closed-root frame snapshot unavailable','mirror_frame_snapshot_unavailable'],
 ['MD-2: install native Chromium or set CHARIOX_KERNEL_BROWSER_EXECUTABLE','browser_executable_unavailable'],
 ['MD-2: run the kernel browser as a normal Unix user; root Chromium is unsupported','browser_root_user_unsupported'],
 ['MD-2: browser profile is already owned by a live process','browser_profile_in_use'],
 ['MD-2: host tab limit reached','browser_tab_limit'],
 ['MD-2: frame exceeds limit','browser_frame_limit'],
 ['MD-2: frame subscription limit reached','browser_subscription_limit'],
 ['MD-2: browser keepalive window unavailable','browser_keepalive_unavailable'],
 ['MD-2: browser tab registry is unreadable; preserve it for recovery','browser_registry_unreadable'],
 ['MD-2: invalid browser tab registry','browser_registry_invalid'],
 ['MD-2: unsupported browser operation','browser_operation_unsupported'],
 ['MD-2: unsupported host method','browser_method_unsupported'],
 ['MD-2: sandboxed host Chromium did not become ready (check executable, display and profile ownership)','browser_launch_failed'],
 ['MP-11: changed mirror input target','input_target_changed'],
 ['MP-11: stale mirror input document','input_document_stale'],
 ['MP-11: stale mirror input epoch','input_epoch_stale'],
 ['MP-11: stale mirror protection policy or admitted input','input_policy_stale'],
 ['MP-11: unavailable native text focus','input_focus_unavailable'],
]);
export function hostErrorLabel(error) {
 const name=names.has(error?.name)?error.name:'Error';
 const frame=typeof error?.stack==='string'?[...error.stack.matchAll(/\/([a-z-]+\.mjs):(\d{1,5}):(\d{1,5})/g)].find(m=>files.has(m[1])):null;
 const method=error?.code==='browser_cdp_timeout'&&typeof error.message==='string'?/^browser CDP method ([A-Za-z.]+) timed out after [0-9]+ms$/.exec(error.message)?.[1]:null;
 return 'host_error '+name+(codes.has(error?.code)?' '+error.code:'')+(method&&methods.has(method)?' '+method:'')+(observerReasons.has(error?.mirrorReason)?' '+error.mirrorReason:'')+(inputReasons.has(error?.message)?' '+inputReasons.get(error.message):'')+(frame?' '+frame[1]+':'+frame[2]+':'+frame[3]:'');
}

// MP-08/MP-10/MP-11: shared public reply and log classifier. Exception messages,
// CDP descriptions, arguments and private path prefixes never cross this boundary.
// Keep kernel_browser_failed as the outer code; no serialized contract changes.
// Existing Web clients match this sentence when retrying a read; append the cause.
export function hostFailureMessage(error) {
 return 'MD-2: host browser operation failed; refresh state or check host browser readiness ('+hostErrorLabel(error).slice('host_error '.length)+')';
}

// MP-08/MP-10/MP-11: classify bounded startup stderr; never expose its text,
// executable/profile paths, URLs or environment. Unknown output stays generic.
export function browserLaunchFailureReason(stderr, spawnCode) {
 if(spawnCode==='ENOENT')return 'browser_executable_missing';
 if(spawnCode==='EACCES')return 'browser_executable_permission_denied';
 const text=typeof stderr==='string'?stderr.slice(0,8192):'';
 if(/unshare: (?:write failed \/proc\/self\/(?:uid|gid)_map: Operation not permitted|unshare failed: Operation not permitted)/.test(text))return 'browser_namespace_permission_denied';
 if(/mount: \/etc: (?:permission denied|must be superuser)/.test(text))return 'browser_policy_mount_denied';
 if(/No usable sandbox|Failed to move to new namespace.*Operation not permitted/.test(text))return 'browser_sandbox_unavailable';
 if(/Missing X server or \$DISPLAY|cannot open display|Unable to open X display/.test(text))return 'browser_display_unavailable';
 return 'browser_launch_failed';
}

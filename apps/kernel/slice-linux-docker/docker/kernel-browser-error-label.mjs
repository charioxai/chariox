// MP-11: opt-in diagnostics identify public code, never exception/page values.
const names=new Set(['Error','TypeError','RangeError','ReferenceError','SyntaxError']);
const files=new Set(['kernel-browser-host.mjs','kernel-browser-display.mjs','kernel-browser-motion.mjs','kernel-browser-native.mjs','kernel-browser-native-worker.mjs','kernel-browser-native-pipe.mjs','kernel-browser-native-credit.mjs','kernel-browser-refiner.mjs','kernel-browser-pixels.mjs','kernel-browser-pixel-worker.mjs','kernel-browser-display-capture.mjs','kernel-browser-display-credit.mjs','browser-controller-cdp.mjs','browser-controller-actions.mjs','kernel-browser-mirror2.mjs','native-computer.mjs','browser-protection-regions.mjs']);
const codes=new Set(['ENOENT','EACCES','EINVAL','EPIPE','ETIMEDOUT','ERR_BUFFER_OUT_OF_BOUNDS','ERR_OUT_OF_RANGE','browser_cdp_timeout','browser_cdp_command_failed','browser_cdp_disconnected','NATIVE_PROTECTION_CHANGED','NATIVE_VALUE_ERROR','NATIVE_HELPER_TIMEOUT','NATIVE_DEPENDENCY_MISSING','NATIVE_FILE_MISSING','NATIVE_OS_ERROR','NATIVE_OUTPUT_LIMIT']);
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

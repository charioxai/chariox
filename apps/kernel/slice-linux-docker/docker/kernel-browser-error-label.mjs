// MP-11: opt-in diagnostics identify public code, never exception/page values.
const names=new Set(['Error','TypeError','RangeError','ReferenceError','SyntaxError']);
const files=new Set(['kernel-browser-host.mjs','kernel-browser-display.mjs','kernel-browser-motion.mjs','kernel-browser-native.mjs','kernel-browser-native-worker.mjs','kernel-browser-native-pipe.mjs','kernel-browser-native-credit.mjs','kernel-browser-refiner.mjs','kernel-browser-pixels.mjs','kernel-browser-pixel-worker.mjs','kernel-browser-display-capture.mjs','kernel-browser-display-credit.mjs']);
const codes=new Set(['ENOENT','EACCES','EINVAL','EPIPE','ETIMEDOUT','ERR_BUFFER_OUT_OF_BOUNDS','ERR_OUT_OF_RANGE']);
export function hostErrorLabel(error) {
 const name=names.has(error?.name)?error.name:'Error';
 const frame=typeof error?.stack==='string'?[...error.stack.matchAll(/\/([a-z-]+\.mjs):(\d{1,5}):(\d{1,5})/g)].find(m=>files.has(m[1])):null;
 return 'host_error '+name+(codes.has(error?.code)?' '+error.code:'')+(frame?' '+frame[1]+':'+frame[2]+':'+frame[3]:'');
}

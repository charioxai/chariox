import { base64 } from './bundle.mjs';
import { bootstrap, assemble } from './bootstrap.mjs';

export const SANDBOX = 'allow-scripts';
export const PERMISSIONS = "camera 'none'; microphone 'none'; geolocation 'none'; display-capture 'none'; payment 'none'; usb 'none'; serial 'none'; bluetooth 'none'; hid 'none'; fullscreen 'none'";
const resources = "default-src 'none'; connect-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:";
// Do not add a nonce or hash here: it would disable unsafe-inline and block the
// child's independently hashed scripts under the inherited policy.
export const HOST_CSP = resources + "; script-src 'unsafe-inline' 'wasm-unsafe-eval'";

const bootstrapScript = '(' + bootstrap.toString() + ')();';
const assemblyScript = '(' + assemble.toString() + ')();';
async function scriptHash(text) {
  return "'sha256-" + base64(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text)))) + "'";
}

export async function renderAppDocument(source) {
  const hashes = await Promise.all([bootstrapScript, assemblyScript, source.script].map(scriptHash));
  const csp = resources + '; script-src ' + hashes.join(' ') + " 'wasm-unsafe-eval'";
  const payload = base64(new TextEncoder().encode(JSON.stringify(source)));
  return '<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="' + csp
    + '"><meta name="referrer" content="no-referrer"><meta http-equiv="x-dns-prefetch-control" content="off"><script>' + bootstrapScript
    + '</script></head><body><script type="application/octet-stream" id="app-source">' + payload + '</script><script>' + assemblyScript + '</script></body></html>';
}

export function renderHostDocument(runtimeScript) {
  if (/<\/script/i.test(runtimeScript)) throw new Error('Host runtime must escape HTML raw-text boundaries');
  return '<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="' + HOST_CSP
    + '"><meta name="referrer" content="no-referrer"><meta http-equiv="x-dns-prefetch-control" content="off"><style>html,body,iframe{margin:0;border:0;width:100%;height:100%;overflow:hidden}iframe{display:block}</style></head><body><script>'
    + runtimeScript + '</script></body></html>';
}

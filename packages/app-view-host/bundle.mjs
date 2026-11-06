// View delivery is a projection of a kernel-verified package, not an archive verifier.
// The trust argument must come from the authenticated kernel, never the bundle.
const encoder = new TextEncoder();
export const MAX_VIEW_BYTES = 16 * 1024 * 1024;
export const SOURCE_PREFIX = '<!doctype html><meta charset="utf-8"><script type="application/octet-stream" id="chariox-app-view">';
export const SOURCE_SUFFIX = '</script>';

export function canonical(value) {
  let nodes = 0;
  function string(text) {
    if (!text.isWellFormed()) throw new Error('App view contains a non-interoperable Unicode string');
    return JSON.stringify(text);
  }
  function visit(item, depth) {
    if (++nodes > 32768 || depth > 32) throw new Error('App control JSON exceeds the package structure limit');
    if (typeof item === 'string') return string(item);
    if (item === null || typeof item === 'boolean') return JSON.stringify(item);
    if (typeof item === 'number' && Number.isFinite(item) && (!Number.isInteger(item) || Number.isSafeInteger(item))) return JSON.stringify(item);
    if (Array.isArray(item)) return '[' + item.map(value => visit(value, depth + 1)).join(',') + ']';
    if (item && Object.getPrototypeOf(item) === Object.prototype) {
      return '{' + Object.keys(item).sort().map(key => string(key) + ':' + visit(item[key], depth + 1)).join(',') + '}';
    }
    throw new Error('App view contains a non-interoperable JSON value');
  }
  return visit(value, 0);
}

export function base64(bytes) {
  let binary = '';
  for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(binary);
}

export function unbase64(text, maxBytes = MAX_VIEW_BYTES) {
  if (typeof text !== 'string' || text.length > Math.ceil(maxBytes / 3) * 4 || text.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(text)) throw new Error('Invalid App view base64');
  const bytes = Uint8Array.from(atob(text), c => c.charCodeAt(0));
  if (bytes.length > maxBytes || base64(bytes) !== text) throw new Error('Non-canonical App view base64');
  return bytes;
}

export async function digest(bytes) {
  return 'sha256:' + Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), b => b.toString(16).padStart(2, '0')).join('');
}

export function signingBytes(manifest, inventory) {
  return encoder.encode(canonical({ schema: 'chariox.package-signature.v2', manifest, inventory }));
}

export function encodeView(source) {
  validateSource(source);
  const bytes = encoder.encode(SOURCE_PREFIX + base64(encoder.encode(canonical(source))) + SOURCE_SUFFIX);
  if (bytes.length > MAX_VIEW_BYTES) throw new Error('Encoded App view exceeds the delivery byte limit');
  return bytes;
}

function validateSource(source) {
  if (!source || Object.keys(source).sort().join(',') !== 'rootId,schema,script,style'
      || source.schema !== 'chariox.app-view-source.v1' || !/^[a-z][a-z0-9_-]{0,63}$/.test(source.rootId)
      || typeof source.script !== 'string' || typeof source.style !== 'string'
      || encoder.encode(source.script + source.style).length > MAX_VIEW_BYTES) throw new Error('Invalid self-contained App view source');
}

export async function verifyView(bundle, trust) {
  if (!trust || !/^sha256:[a-f0-9]{64}$/.test(trust.contentDigest) || typeof trust.publisherId !== 'string' || typeof trust.keyId !== 'string') throw new Error('Missing authenticated App publisher trust');
  if (!bundle || typeof bundle.manifest !== 'string' || typeof bundle.inventory !== 'string'
      || encoder.encode(bundle.manifest).length > 1024 * 1024 || encoder.encode(bundle.inventory).length > 1024 * 1024) throw new Error('Invalid App view envelope');
  const manifest = JSON.parse(bundle.manifest), inventory = JSON.parse(bundle.inventory);
  // Requiring the canonical encoding also rejects duplicate keys and lone surrogates.
  if (canonical(manifest) !== bundle.manifest || canonical(inventory) !== bundle.inventory) throw new Error('App controls must use canonical JSON');
  if (manifest.schema !== 'chariox.app.v2' || manifest.appContractVersion !== 2
      || manifest.publisher?.id !== trust.publisherId || manifest.publisher?.keyId !== trust.keyId
      || inventory.schema !== 'chariox.integrity.v2' || !Array.isArray(inventory.files) || inventory.files.length > 4093) throw new Error('App publisher or package contract mismatch');
  const signature = bundle.signature;
  const subject = signingBytes(manifest, inventory);
  if (!signature || signature.algorithm !== 'ed25519' || signature.key_id !== trust.keyId
      || signature.digest !== trust.contentDigest || await digest(subject) !== trust.contentDigest) throw new Error('App signing subject mismatch');
  const key = await crypto.subtle.importKey('raw', unbase64(trust.publicKey, 32), 'Ed25519', false, ['verify']);
  if (!await crypto.subtle.verify('Ed25519', key, unbase64(signature.value, 64), subject)) throw new Error('App publisher signature failed');
  let previous = '';
  for (const item of inventory.files) {
    // Kernel admission checks archive paths. This projection never interprets
    // them as URLs or filesystem paths; only the exact delivered entry is used.
    if (!item || typeof item.path !== 'string'
        || item.path <= previous || !Number.isSafeInteger(item.size) || item.size < 0
        || !/^sha256:[a-f0-9]{64}$/.test(item.digest)) throw new Error('Invalid App inventory');
    previous = item.path;
  }
  const entry = manifest.ui?.entry;
  if (typeof entry !== 'string' || !entry.startsWith('ui/') || !entry.endsWith('.html')
      || !Array.isArray(bundle.files) || bundle.files.length !== 1 || bundle.files[0]?.path !== entry) throw new Error('Expected exactly the signed self-contained App entry');
  const expected = inventory.files.find(file => file.path === entry);
  const bytes = unbase64(bundle.files[0].body_base64);
  if (!expected || bytes.length !== expected.size || await digest(bytes) !== expected.digest) throw new Error('App entry hash or size mismatch');
  const document = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  if (!document.startsWith(SOURCE_PREFIX) || !document.endsWith(SOURCE_SUFFIX)) throw new Error('App entry was not built by the self-contained packer');
  const source = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(unbase64(document.slice(SOURCE_PREFIX.length, -SOURCE_SUFFIX.length))));
  validateSource(source);
  return Object.freeze({ ...source });
}

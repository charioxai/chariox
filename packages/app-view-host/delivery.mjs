// MP-08 / MP-09 / MP-10 / MP-11 — read-only delivery over the client's authenticated RPC.
import { base64, canonical, digest, MAX_VIEW_BYTES, unbase64, verifyView } from './bundle.mjs';

export const MAX_CHUNK_BYTES = 64 * 1024;
export class AppViewCache {
  #entries = new Map();
  #bytes = 0;
  constructor(maxBytes = 256 * 1024 * 1024) {
    if (!Number.isSafeInteger(maxBytes) || maxBytes < 0 || maxBytes > 256 * 1024 * 1024) throw new Error('Invalid App cache bound');
    this.maxBytes = maxBytes;
  }
  get(key) {
    const bytes = this.#entries.get(key);
    if (!bytes) return null;
    this.#entries.delete(key); this.#entries.set(key, bytes);
    return bytes.slice();
  }
  delete(key) {
    const bytes = this.#entries.get(key);
    if (bytes) { this.#bytes -= bytes.length; this.#entries.delete(key); }
  }
  set(key, bytes) {
    this.delete(key);
    if (bytes.length > this.maxBytes) return;
    while (this.#bytes + bytes.length > this.maxBytes) this.delete(this.#entries.keys().next().value);
    this.#entries.set(key, bytes.slice()); this.#bytes += bytes.length;
  }
  purge(kernelOwner, installation) {
    for (const key of this.#entries.keys()) {
      const [owner, app] = JSON.parse(key);
      if (owner === kernelOwner && app === installation) this.delete(key);
    }
  }
}

export async function loadClientAppView({ rpc, kernelOwner, installationId, cache = new AppViewCache() }) {
  if (typeof kernelOwner !== 'string' || !kernelOwner || typeof installationId !== 'string' || !installationId) throw new Error('Missing authenticated App cache scope');
  const send = async command => {
    const result = await rpc({ ClientAppView: command });
    if (!result?.ClientAppView) throw new Error('App view delivery refused');
    return result.ClientAppView;
  };
  const { Opened: descriptor } = await send({ Open: { installation_id: installationId } });
  if (!descriptor || !/^client-app-[a-f0-9]{32}$/.test(descriptor.view_id)) throw new Error('Invalid App delivery identity');
  const close = () => send({ Close: { view_id: descriptor.view_id } });
  try {
    if (descriptor.installation_id !== installationId || !/^[1-9][0-9]*$/.test(descriptor.generation)
        || !/^sha256:[a-f0-9]{64}$/.test(descriptor.package_digest)) throw new Error('Invalid App delivery binding');
    const { bundle: controls, trust } = descriptor;
    if (typeof controls?.manifest !== 'string' || typeof controls?.inventory !== 'string'
        || controls.manifest.length > 1024 * 1024 || controls.inventory.length > 1024 * 1024) throw new Error('Invalid App controls');
    const manifest = JSON.parse(controls.manifest), inventory = JSON.parse(controls.inventory);
    const entry = manifest.ui?.entry;
    const expected = inventory.files?.find(file => file.path === entry);
    if (!expected || !Number.isSafeInteger(expected.size) || expected.size < 1 || expected.size > MAX_VIEW_BYTES) throw new Error('App entry exceeds delivery bound');
    const key = canonical([kernelOwner, installationId, descriptor.package_digest, entry, expected.digest]);
    let bytes = cache.get(key);
    if (bytes && (bytes.length !== expected.size || await digest(bytes) !== expected.digest)) { cache.delete(key); bytes = null; }
    if (!bytes) {
      bytes = new Uint8Array(expected.size);
      let offset = 0;
      while (offset < bytes.length) {
        const max_bytes = Math.min(MAX_CHUNK_BYTES, bytes.length - offset);
        const { Chunk: chunk } = await send({ Read: { view_id: descriptor.view_id, offset, max_bytes } });
        const part = unbase64(chunk?.body_base64, max_bytes);
        if (chunk.view_id !== descriptor.view_id || chunk.offset !== offset || !part.length
            || offset + part.length > bytes.length || chunk.eof !== (offset + part.length === bytes.length)) throw new Error('Invalid App chunk sequence');
        bytes.set(part, offset); offset += part.length;
      }
    }
    const bundle = { ...controls, files: [{ path: entry, body_base64: base64(bytes) }] };
    await verifyView(bundle, trust);
    const { Valid: valid } = await send({ Check: { view_id: descriptor.view_id } });
    if (valid?.view_id !== descriptor.view_id || valid.generation !== descriptor.generation) throw new Error('App delivery generation changed');
    cache.set(key, bytes);
    return { descriptor, bundle, trust, close };
  } catch (error) { await close().catch(() => {}); throw error; }
}

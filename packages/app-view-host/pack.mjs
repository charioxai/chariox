import { base64, canonical, digest, encodeView, signingBytes } from './bundle.mjs';

// Feasibility producer: the real package installer remains the Rust verifier.
// The caller owns the key; production must use the package signing workflow.
export async function packView(manifest, source, keyPair, extraFiles = new Map()) {
  const entry = manifest.ui.entry;
  if (extraFiles.has(entry)) throw new Error('App entry is supplied twice');
  const files = new Map([...extraFiles, [entry, encodeView(source)]]);
  const inventory = { schema: 'chariox.integrity.v2', files: [] };
  for (const path of [...files.keys()].sort()) {
    const bytes = files.get(path);
    inventory.files.push({ path, size: bytes.length, digest: await digest(bytes) });
  }
  const subject = signingBytes(manifest, inventory);
  const contentDigest = await digest(subject);
  const signature = { algorithm: 'ed25519', key_id: manifest.publisher.keyId, digest: contentDigest,
    value: base64(new Uint8Array(await crypto.subtle.sign('Ed25519', keyPair.privateKey, subject))) };
  const trust = { publisherId: manifest.publisher.id, keyId: manifest.publisher.keyId, contentDigest,
    publicKey: base64(new Uint8Array(await crypto.subtle.exportKey('raw', keyPair.publicKey))) };
  return { bundle: { manifest: canonical(manifest), inventory: canonical(inventory), signature,
    files: [{ path: entry, body_base64: base64(files.get(entry)) }] }, trust };
}

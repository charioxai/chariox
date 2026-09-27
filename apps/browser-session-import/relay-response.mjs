import {decryptRelayPayload} from '../../packages/kernel-client/src/browser-relay-crypto.ts';

export async function decryptBrowserImportResponse(privateKey, payload, kernelPublicKey, requestNonce) {
  try {
    if (typeof kernelPublicKey !== 'string' || !kernelPublicKey
        || typeof requestNonce !== 'string' || !/^[A-Za-z0-9+/]{16}$/.test(requestNonce)) throw new Error();
    const envelope = JSON.parse(await decryptRelayPayload(privateKey, payload, kernelPublicKey));
    if (!envelope || typeof envelope !== 'object' || Array.isArray(envelope)
        || envelope.request_nonce !== requestNonce
        || !Object.hasOwn(envelope, 'response')) throw new Error();
    return envelope.response;
  } catch {
    // Neither untrusted transport errors nor decrypted data belong in logs.
    throw new Error('browser import response denied');
  }
}

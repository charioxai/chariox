// Integration seam for codex/browser-import-production-runtime only. Do not
// replace this with a general relay request or a success fixture: the runtime
// command must privately claim and transactionally apply the approved import.
export async function deliverBrowserImport() {
  throw Object.assign(new Error('browser_import_delivery_unavailable'),
    {code:'browser_import_delivery_unavailable'});
}

// Documentation/schema sentinel. encrypted_cookie_batch is the existing
// EncryptedRelayPayload produced for the pinned kernel by the retained connector
// sender key. The plaintext inside it is documented in README.md and is never a
// consent/public protocol request.
export const expectedDeliveryEnvelope = Object.freeze({
  command:'DeliverBrowserImport',request_id:'<32 hex>',session_id:'<session>',
  attachment_id:'<attachment>',environment_id:'<environment>',runtime_generation:0,
  tab_id:'<destination tab>',document_revision:0,source_store_id:'0',domains:[],
  partition_sites:[],overwrite:false,
  encrypted_cookie_batch:Object.freeze({sender_public_key:'<paired connector key>',
    nonce:'<12-byte base64>',ciphertext:'<AES-256-GCM base64>'}),
});

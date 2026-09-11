import {encryptRelayPayload} from '../../packages/kernel-client/src/browser-relay-crypto.ts';
import {browserImportConsentMinimumProtocolVersion,browserImportDeliveryMinimumProtocolVersion} from '../../packages/kernel-client/src/browser-import-requests.ts';
import {decryptBrowserImportResponse} from './relay-response.mjs';
import {snapshotImportRequest} from './relay-request-metadata.mjs';

const maxFrameChars = 1048576;
const maxBufferedChars = 262144;
const maxCookiePayloadBytes = 512 * 1024;
const maxPending = 8;
const failure = () => new Error('browser import transport unavailable');

// Internal connector transport, not enrollment. All arguments must come from the
// authorized connector UI and trusted pairing, never an arbitrary page message.
export async function connectBrowserImportRelay({relayUrl, authToken, daemonId,
  kernelPublicKey, sender, protocolVersion, signal, timeoutMs = 5000,
  WebSocketImpl = globalThis.WebSocket}) {
  try {
    sender = Object.freeze({privateKey:sender?.privateKey,publicKeyBase64:sender?.publicKeyBase64});
    const url = new URL(relayUrl);
    if (url.username || url.password || url.search || url.hash
        || !(url.protocol === 'wss:' || (url.protocol === 'ws:'
          && ['127.0.0.1','localhost','[::1]'].includes(url.hostname)))) throw failure();
    if (!text(authToken,16384) || !text(daemonId,512) || !text(kernelPublicKey,256)
        || !text(sender?.publicKeyBase64,256) || sender?.privateKey?.type !== 'private'
        || sender.privateKey.extractable || sender.privateKey.algorithm?.name !== 'ECDH'
        || sender.privateKey.algorithm?.namedCurve !== 'P-256'
        || !Number.isInteger(protocolVersion) || protocolVersion < browserImportConsentMinimumProtocolVersion
        || typeof WebSocketImpl !== 'function' || !validSignal(signal) || signal?.aborted) throw failure();
    deadline(timeoutMs);
  } catch { throw failure(); }

  return new Promise((resolve,reject) => {
    let socket;
    let ready = false;
    let closed = false;
    const pending = new Map();
    const handshakeTimer = setTimeout(close,timeoutMs);

    function settle(id, result, error) {
      const entry = pending.get(id);
      if (!entry) return;
      pending.delete(id);
      clearTimeout(entry.timer);
      entry.signal?.removeEventListener('abort',entry.abort);
      if (error) entry.reject(failure());
      else entry.resolve(result);
    }

    function close() {
      if (closed) return;
      closed = true;
      authToken = '';
      clearTimeout(handshakeTimer);
      signal?.removeEventListener('abort',close);
      if (socket) {
        socket.removeEventListener('open',onOpen);
        socket.removeEventListener('message',onMessage);
        socket.removeEventListener('error',close);
        socket.removeEventListener('close',close);
        try { socket.close(); } catch { /* no transport payload in errors */ }
      }
      for (const id of pending.keys()) settle(id,null,true);
      reject(failure());
    }

    function onOpen() {
      if (closed) return;
      try {
        socket.send(JSON.stringify({kind:'client_connect',auth_token:authToken,target:{daemon_id:daemonId}}));
        authToken = '';
      } catch { close(); }
    }

    function onMessage(event) {
      if (closed) return;
      try {
        if (typeof event.data !== 'string' || event.data.length > maxFrameChars) throw failure();
        const frame = JSON.parse(event.data);
        if (!ready) {
          if (frame?.kind !== 'client_connected' || frame.target?.daemon_id !== daemonId
              || frame.daemon_public_key !== kernelPublicKey) throw failure();
          ready = true;
          clearTimeout(handshakeTimer);
          resolve({request,deliver,close});
          return;
        }
        if (frame?.kind !== 'client_response') throw failure();
        const entry = pending.get(frame.request_id);
        if (!entry || entry.decoding) return;
        if (frame.error || !entry.nonce || !frame.encrypted_response) {
          settle(frame.request_id,null,true);
          return;
        }
        entry.decoding = true;
        void decode(frame,entry);
      } catch { close(); }
    }

    async function decode(frame,entry) {
      try {
        const result = await decryptBrowserImportResponse(sender.privateKey,frame.encrypted_response,
          kernelPublicKey,entry.nonce);
        if (pending.get(frame.request_id) !== entry) return;
        if (entry.delivery) {
          validateDeliveryResult(result,entry.domains);
        } else {
          const consent = result?.BrowserImportConsent;
          if (!consent || typeof consent.request_id !== 'string' || !/^[a-fA-F0-9]{32}$/.test(consent.request_id)
              || consent.status !== entry.status || (entry.requestId && consent.request_id !== entry.requestId)
              || Object.keys(result).length !== 1 || Object.keys(consent).length !== 2) throw failure();
        }
        settle(frame.request_id,result,false);
      } catch { if (pending.get(frame.request_id) === entry) settle(frame.request_id,null,true); }
    }

    async function request(value, options = {}) {
      let metadata;
      let delay;
      let requestSignal;
      try {
        requestSignal = options.signal;
        if (closed || !ready || pending.size >= maxPending || !validSignal(requestSignal)
            || requestSignal?.aborted) throw failure();
        delay = deadline(options.timeoutMs ?? timeoutMs);
        metadata = snapshotImportRequest(value);
      } catch { throw failure(); }
      const id = crypto.randomUUID();
      const plaintext = JSON.stringify({command_id:crypto.randomUUID(),request:metadata.request});
      if (plaintext.length > 131072) throw failure();
      return new Promise((resolveRequest,rejectRequest) => {
        const entry = {resolve:resolveRequest,reject:rejectRequest,nonce:null,decoding:false,delivery:false,
          requestId:metadata.requestId,status:metadata.status,signal:requestSignal,
          abort:() => settle(id,null,true),timer:setTimeout(() => settle(id,null,true),delay)};
        pending.set(id,entry);
        try {
          requestSignal?.addEventListener('abort',entry.abort,{once:true});
          if (requestSignal?.aborted) { entry.abort(); return; }
          void send();
        } catch { settle(id,null,true); }
        async function send() {
          try {
            const encrypted = await encryptRelayPayload(kernelPublicKey,plaintext,sender);
            if (closed || pending.get(id) !== entry) return;
            entry.nonce = encrypted.payload.nonce;
            const frame = JSON.stringify({kind:'client_request',request_id:id,
              target:{daemon_id:daemonId},encrypted_request:encrypted.payload});
            if (frame.length > maxFrameChars || socket.bufferedAmount > maxBufferedChars) throw failure();
            socket.send(frame);
          } catch { settle(id,null,true); }
        }
      });
    }

    async function deliver({requestId,selection,cookies}, options = {}) {
      let requestSignal, delay, selected, payload;
      try {
        requestSignal = options.signal;
        if (protocolVersion < browserImportDeliveryMinimumProtocolVersion
            || closed || !ready || pending.size >= maxPending || !validSignal(requestSignal)
            || requestSignal?.aborted || !Array.isArray(cookies) || cookies.length > 512) throw failure();
        delay = deadline(options.timeoutMs ?? timeoutMs);
        const metadata = snapshotImportRequest({AuthorizeBrowserImportSource:{request_id:requestId,selection}});
        selected = metadata.request.AuthorizeBrowserImportSource.selection;
        const bytes = new TextEncoder().encode(JSON.stringify(cookies));
        if (bytes.length > maxCookiePayloadBytes) throw failure();
        payload = bytesToBase64(bytes);
        bytes.fill(0);
      } catch { throw failure(); }
      const id = crypto.randomUUID();
      let plaintext = JSON.stringify({browser_import_delivery:{request_id:requestId,
        selection:selected,payload_base64:payload}});
      payload = '';
      if (plaintext.length > maxFrameChars) throw failure();
      return new Promise((resolveRequest,rejectRequest) => {
        const entry = {resolve:resolveRequest,reject:rejectRequest,nonce:null,decoding:false,delivery:true,
          domains:Object.freeze([...selected.domains]),requestId,status:null,signal:requestSignal,
          abort:() => settle(id,null,true),timer:setTimeout(() => settle(id,null,true),delay)};
        pending.set(id,entry);
        try {
          requestSignal?.addEventListener('abort',entry.abort,{once:true});
          if (requestSignal?.aborted) { entry.abort(); return; }
          void send();
        } catch { settle(id,null,true); }
        async function send() {
          try {
            const encrypted = await encryptRelayPayload(kernelPublicKey,plaintext,sender);
            plaintext = '';
            if (closed || pending.get(id) !== entry) return;
            entry.nonce = encrypted.payload.nonce;
            const frame = JSON.stringify({kind:'client_request',request_id:id,
              target:{daemon_id:daemonId},encrypted_request:encrypted.payload});
            if (frame.length > maxFrameChars || socket.bufferedAmount > maxBufferedChars) throw failure();
            socket.send(frame);
          } catch { plaintext = ''; settle(id,null,true); }
        }
      });
    }

    try {
      socket = new WebSocketImpl(relayUrl);
      socket.addEventListener('open',onOpen);
      socket.addEventListener('message',onMessage);
      socket.addEventListener('error',close);
      socket.addEventListener('close',close);
      signal?.addEventListener('abort',close,{once:true});
      if (signal?.aborted) close();
    } catch { close(); }
  });
}

function text(value,max) {
  return typeof value === 'string' && value.length > 0 && value.length <= max && !/[\x00-\x1f\x7f]/.test(value);
}

function deadline(value) {
  if (!Number.isInteger(value) || value < 1 || value > 30000) throw failure();
  return value;
}

function validSignal(value) {
  return value === undefined || value === null || value instanceof AbortSignal;
}

function bytesToBase64(bytes) {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset,offset + 0x8000));
  }
  return btoa(binary);
}

function validateDeliveryResult(result,domains) {
  const delivered = result?.BrowserImportDelivered;
  if (!delivered || Object.keys(result).length !== 1 || Object.keys(delivered).length !== 1
      || !Array.isArray(delivered.results) || delivered.results.length !== domains.length) throw failure();
  let total = 0;
  for (let index = 0; index < domains.length; index += 1) {
    const item = delivered.results[index];
    if (!item || Object.keys(item).length !== 3 || item.domain !== domains[index]
        || !['imported','no_cookies'].includes(item.status)
        || !Number.isSafeInteger(item.cookie_count) || item.cookie_count < 0 || item.cookie_count > 512
        || (item.status === 'imported') !== (item.cookie_count > 0)) throw failure();
    total += item.cookie_count;
  }
  if (total > 512) throw failure();
}

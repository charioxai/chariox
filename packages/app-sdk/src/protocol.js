import { protocolError } from './errors.js';
import { parseJson, validateJson } from './json.js';

export const APP_WIRE_VERSION = 1;
export const MAX_FRAME_BYTES = 1024 * 1024;
const decoder = new TextDecoder('utf-8', { fatal: true });
const common = ['version', 'generation', 'kind'];

export function object(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function token(value) {
  return typeof value === 'string' && value.length > 0 && Buffer.byteLength(value) <= 128
    && !/[\p{Cc}\p{White_Space}\uFEFF]/u.test(value);
}

function fields(value, required, optional = []) {
  if (!object(value) || required.some((key) => !Object.hasOwn(value, key))
    || Object.keys(value).some((key) => !required.includes(key) && !optional.includes(key))) {
    throw protocolError('Invalid App IPC fields');
  }
}

export function validateMessage(message, sender) {
  validateJson(message);
  if (!object(message) || message.version !== APP_WIRE_VERSION || !token(message.generation)) {
    throw protocolError('Invalid App IPC version or generation');
  }
  switch (message.kind) {
    case 'request':
      fields(message, [...common, 'id', 'method', 'params', 'deadline_ms'], ['context']);
      if (sender === 'worker' && Object.hasOwn(message, 'context')) throw protocolError('App worker cannot supply caller context');
      if (!token(message.id) || !token(message.method)
        || !Number.isSafeInteger(message.deadline_ms) || message.deadline_ms <= 0
        || (Object.hasOwn(message, 'context') && !object(message.context))) {
        throw protocolError('Invalid App IPC request');
      }
      break;
    case 'response': {
      const success = Object.hasOwn(message, 'result');
      fields(message, [...common, 'id', success ? 'result' : 'error']);
      if (!token(message.id)) throw protocolError('Invalid App IPC response identity');
      if (!success) {
        fields(message.error, ['code', 'message'], ['retryable']);
        if (!token(message.error.code) || typeof message.error.message !== 'string'
          || Buffer.byteLength(message.error.message) > 4096
          || (Object.hasOwn(message.error, 'retryable') && typeof message.error.retryable !== 'boolean')) {
          throw protocolError('Invalid App IPC error');
        }
      }
      break;
    }
    case 'cancel':
      fields(message, [...common, 'id']);
      if (!token(message.id)) throw protocolError('Invalid App IPC cancellation');
      break;
    case 'event':
      fields(message, [...common, 'name', 'data']);
      if (!token(message.name)) throw protocolError('Invalid App IPC event');
      break;
    default:
      throw protocolError('Unknown App IPC message kind');
  }
  return message;
}

export function encodeFrame(message, limit = MAX_FRAME_BYTES) {
  validateMessage(message);
  let json;
  try { json = JSON.stringify(message); } catch { throw protocolError('App IPC must contain JSON values'); }
  // Undefined fields and toJSON hooks must not silently create an invalid wire envelope.
  validateMessage(parseJson(json));
  const length = Buffer.byteLength(json);
  if (length === 0 || length > limit) throw protocolError('App IPC frame exceeds size limit');
  const frame = Buffer.allocUnsafe(length + 4);
  frame.writeUInt32BE(length, 0);
  frame.write(json, 4, length, 'utf8');
  return frame;
}

// A header is inspected before allocating its body. Only one bounded frame is
// retained; input chunks may contain many frames without another combined copy.
export class FrameDecoder {
  #header = Buffer.alloc(4);
  #headerBytes = 0;
  #body;
  #bodyBytes = 0;
  #closed = false;

  constructor(onMessage, limit = MAX_FRAME_BYTES) {
    if (!Number.isSafeInteger(limit) || limit < 64 || limit > MAX_FRAME_BYTES) {
      throw new RangeError('Invalid App IPC frame limit');
    }
    this.onMessage = onMessage;
    this.limit = limit;
  }

  get hasPartialFrame() {
    return this.#headerBytes !== 0 || this.#body !== undefined;
  }

  push(chunk) {
    if (this.#closed) throw protocolError('App IPC decoder is closed');
    if (!(chunk instanceof Uint8Array)) throw protocolError('App IPC requires binary frames');
    const input = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength);
    let offset = 0;
    try {
      while (offset < input.length && !this.#closed) {
        if (!this.#body) {
          const length = Math.min(4 - this.#headerBytes, input.length - offset);
          input.copy(this.#header, this.#headerBytes, offset, offset + length);
          this.#headerBytes += length;
          offset += length;
          if (this.#headerBytes < 4) continue;
          const bodyLength = this.#header.readUInt32BE(0);
          if (bodyLength === 0 || bodyLength > this.limit) throw protocolError('App IPC frame exceeds size limit');
          this.#body = Buffer.allocUnsafe(bodyLength);
          this.#bodyBytes = 0;
          this.#headerBytes = 0;
        }
        const length = Math.min(this.#body.length - this.#bodyBytes, input.length - offset);
        input.copy(this.#body, this.#bodyBytes, offset, offset + length);
        this.#bodyBytes += length;
        offset += length;
        if (this.#bodyBytes === this.#body.length) {
          let json;
          try { json = decoder.decode(this.#body); }
          catch { throw protocolError('Invalid UTF-8 JSON App IPC frame'); }
          const message = parseJson(json);
          this.#body = undefined;
          this.#bodyBytes = 0;
          this.onMessage(validateMessage(message));
        }
      }
    } catch (error) {
      this.close();
      throw error;
    }
  }

  end() {
    const truncated = this.#headerBytes !== 0 || this.#body !== undefined;
    this.close();
    if (truncated) throw protocolError('Truncated App IPC frame');
  }

  close() {
    this.#closed = true;
    this.#body = undefined;
    this.#bodyBytes = 0;
    this.#headerBytes = 0;
  }
}

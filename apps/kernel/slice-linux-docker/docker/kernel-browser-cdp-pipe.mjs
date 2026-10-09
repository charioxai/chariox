// Host CDP is a private inherited capability, never a loopback listener.
// Reuse the controller's request/session/event machinery over Chromium's
// NUL-delimited fd 3/4 transport. Bound framing before buffering a whole reply.
import { CdpConnection } from './browser-controller-cdp.mjs';

const MAX_MESSAGE = 16 * 1024 * 1024;
export function connectCdpPipe(input, output, requestTimeoutMs = 5000) {
  const listeners = new Map();
  const emit = (type, event = {}) => {
    for (const listener of listeners.get(type) ?? []) listener(event);
  };
  const socket = {
    readyState: 1,
    addEventListener(type, listener) {
      const current = listeners.get(type) ?? new Set();
      current.add(listener); listeners.set(type, current);
    },
    send(message) {
      if (socket.readyState !== 1) throw new Error('MD-2: private CDP pipe closed');
      if (Buffer.byteLength(message) > MAX_MESSAGE || message.includes('\0')) throw new Error('MD-2: private CDP message exceeds limit');
      if (input.writableLength + Buffer.byteLength(message) + 1 > MAX_MESSAGE) {
        socket.close(); throw new Error('MD-2: private CDP pipe backpressure limit');
      }
      input.write(message + '\0');
    },
    close() {
      if (socket.readyState !== 1) return;
      socket.readyState = 3;
      input.destroy(); output.destroy(); emit('close');
    },
  };
  const connection = new CdpConnection(socket, requestTimeoutMs);
  let buffer = Buffer.alloc(0), size = 0;
  const fail = () => { buffer = Buffer.alloc(0); size = 0; socket.close(); };
  input.on('error', fail); output.on('error', fail); output.on('end', fail); output.on('close', fail);
  output.on('data', chunk => {
    if (socket.readyState !== 1) return;
    for (let start = 0; start < chunk.length;) {
      const end = chunk.indexOf(0, start);
      const part = chunk.subarray(start, end < 0 ? chunk.length : end);
      const nextSize = size + part.length;
      if (nextSize > MAX_MESSAGE) { fail(); return; }
      if (nextSize > buffer.length) {
        const grown = Buffer.allocUnsafe(Math.min(MAX_MESSAGE, Math.max(4096, nextSize, buffer.length * 2)));
        buffer.copy(grown, 0, 0, size); buffer = grown;
      }
      part.copy(buffer, size); size = nextSize;
      if (end < 0) return;
      emit('message', { data: buffer.toString('utf8', 0, size) });
      size = 0; start = end + 1;
      if (socket.readyState !== 1 || !connection.isOpen()) { fail(); return; }
    }
  });
  return connection;
}

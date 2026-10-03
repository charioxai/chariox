import { Duplex } from 'node:stream';

export function fakeTransport() {
  let message;
  let disconnected;
  return {
    sent: [],
    closed: false,
    subscribe(onMessage, onClose) {
      message = onMessage;
      disconnected = onClose;
      return () => {};
    },
    send(value) { this.sent.push(structuredClone(value)); },
    receive(value) { message(structuredClone(value)); },
    disconnect(error) { disconnected(error); },
    close() { this.closed = true; },
  };
}

export const generation = 'installation-generation-7';
export const envelope = (message) => ({ version: 1, generation, ...message });
export const request = (id, method, params = {}, extra = {}) => envelope({
  kind: 'request', id, method, params, deadline_ms: Date.now() + 5000, ...extra,
});
export const response = (id, result) => envelope({ kind: 'response', id, result });
export const flush = () => new Promise((resolve) => setImmediate(resolve));
export const deferred = () => {
  let resolve;
  const promise = new Promise((r) => { resolve = r; });
  return { promise, resolve };
};

export class TestStream extends Duplex {
  constructor({ stallWrites = false } = {}) {
    super();
    this.frames = [];
    this.stallWrites = stallWrites;
  }
  _read() {}
  _write(chunk, _encoding, callback) {
    this.frames.push(Buffer.from(chunk));
    if (!this.stallWrites) callback();
  }
}

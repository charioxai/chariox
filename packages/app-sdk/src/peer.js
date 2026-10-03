import { randomUUID } from 'node:crypto';
import { AppError, protocolError, wireError } from './errors.js';
import { APP_WIRE_VERSION, token, validateMessage } from './protocol.js';

const DEFAULT_LIMITS = Object.freeze({ maxPending: 64, maxHandlers: 16, maxDeadlineMs: 30_000 });

export class AppPeer {
  #pending = new Map();
  #active = new Map();
  #closed = false;
  #counter = 0;
  #prefix = `sdk-${randomUUID()}-`;

  constructor({ transport, generation, handleRequest, onControlEvent, onInvalidOutput, limits = {} }) {
    if (!token(generation)) throw new TypeError('Expected an installation generation');
    this.transport = transport;
    this.generation = generation;
    this.handleRequest = handleRequest;
    this.onControlEvent = onControlEvent;
    this.onInvalidOutput = onInvalidOutput;
    this.limits = { ...DEFAULT_LIMITS, ...limits };
    for (const [key, value] of Object.entries(this.limits)) {
      if (!(key in DEFAULT_LIMITS) || !Number.isSafeInteger(value) || value < 1 || value > DEFAULT_LIMITS[key]) {
        throw new RangeError(`Invalid App IPC limit: ${key}`);
      }
    }
    this.unsubscribe = transport.subscribe((message) => this.#receive(message), (reason) => this.close(reason));
  }

  request(method, params, { signal, timeoutMs = this.limits.maxDeadlineMs } = {}) {
    if (this.#closed) return Promise.reject(new AppError('DISCONNECTED', 'App IPC disconnected'));
    if (!token(method)) return Promise.reject(new AppError('INVALID_ARGUMENT', 'Invalid kernel operation'));
    if (signal?.aborted) return Promise.reject(new AppError('CANCELLED', 'App request cancelled'));
    if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > this.limits.maxDeadlineMs) {
      return Promise.reject(new AppError('INVALID_ARGUMENT', 'App request deadline is outside its budget'));
    }
    if (this.#pending.size >= this.limits.maxPending) {
      return Promise.reject(new AppError('BUSY', 'Too many pending App requests', { retryable: true }));
    }
    const id = `${this.#prefix}${++this.#counter}`;
    return new Promise((resolve, reject) => {
      const settle = (error, result) => {
        if (!this.#pending.delete(id)) return;
        clearTimeout(timer);
        signal?.removeEventListener('abort', abort);
        if (error) reject(error); else resolve(result);
      };
      const cancel = (code, message) => {
        settle(new AppError(code, message));
        this.#control({ kind: 'cancel', id });
      };
      const abort = () => cancel('CANCELLED', 'App request cancelled');
      const timer = setTimeout(() => cancel('DEADLINE_EXCEEDED', 'App request deadline exceeded'), timeoutMs);
      this.#pending.set(id, { settle });
      signal?.addEventListener('abort', abort, { once: true });
      const refused = this.#send({ kind: 'request', id, method, params, deadline_ms: Date.now() + timeoutMs });
      if (refused) {
        const reason = detail(refused) ?? 'they could not be read as JSON';
        settle(new AppError('INVALID_ARGUMENT', `App request parameters cannot be sent: ${reason}`));
      }
    });
  }

  // Returns the error when a message is refused before any byte is written:
  // App code supplied a value that is not JSON (or too large), and only that
  // call fails. Any other transport failure closes the channel.
  #send(message) {
    if (this.#closed) return null;
    let envelope;
    try {
      envelope = validateMessage({ version: APP_WIRE_VERSION, generation: this.generation, ...message }, 'worker');
    } catch (error) {
      return error;
    }
    try {
      this.transport.send(envelope);
      return null;
    } catch (error) {
      // encodeFrame refuses a value (with a `detail`) before writing a byte.
      if (!this.#closed && detail(error) !== undefined) return error;
      this.close(error);
      return null;
    }
  }

  // SDK-built messages carry no App value; a refusal is a broken channel.
  #control(message) {
    const refused = this.#send(message);
    if (refused) this.close(refused);
  }

  // The App's result or error for one call. A malformed one fails that call
  // alone, as INVALID_OUTPUT or HANDLER_FAILED, and never closes the channel;
  // onInvalidOutput tells the App's developer why, before the reply is sent.
  #respond(request, outcome) {
    const refused = this.#send({ kind: 'response', id: request.id, ...outcome });
    if (!refused) return;
    const result = Object.hasOwn(outcome, 'result');
    // wireError bounds the message and makes it well-formed: the wire refuses
    // an AppError only for its code.
    const reason = detail(refused) ?? (result ? 'it could not be read as JSON'
      : 'its code is not 1 to 128 bytes without spaces or control characters');
    const reply = wireError(result ? new AppError('INVALID_OUTPUT', `The App's result cannot be sent: ${reason}`) : null);
    try { this.onInvalidOutput?.(request.method, request.params, reason, reply.code); } catch { /* best effort */ }
    this.#control({ kind: 'response', id: request.id, error: reply });
  }

  #receive(message) {
    if (this.#closed) return;
    try {
      validateMessage(message, 'supervisor');
      if (message.generation !== this.generation) throw protocolError('Stale App IPC generation');
      switch (message.kind) {
        case 'response': {
          const pending = this.#pending.get(message.id);
          if (!pending) return; // A timed-out or cancelled call cannot be revived.
          pending.settle(Object.hasOwn(message, 'error')
            ? new AppError(message.error.code, message.error.message, message.error) : undefined, message.result);
          break;
        }
        case 'request':
          this.#dispatch(message);
          break;
        case 'cancel':
          this.#active.get(message.id)?.cancel('CANCELLED', 'App call cancelled');
          break;
        case 'event':
          if (!message.name.startsWith('supervisor.')) throw protocolError('App occurrences require acknowledged delivery');
          this.onControlEvent?.(message.name, message.data);
          break;
      }
    } catch (error) { this.close(error); }
  }

  #dispatch(message) {
    if (this.#active.has(message.id)) throw protocolError('Duplicate active App call identity');
    if (this.#active.size >= this.limits.maxHandlers) {
      this.#control({ kind: 'response', id: message.id, error: wireError(new AppError('BUSY', 'App handler capacity is full', { retryable: true })) });
      return;
    }
    const deadline = Math.min(message.deadline_ms, Date.now() + this.limits.maxDeadlineMs);
    if (deadline <= Date.now()) {
      this.#control({ kind: 'response', id: message.id, error: wireError(new AppError('DEADLINE_EXCEEDED', 'App call deadline exceeded')) });
      return;
    }
    const controller = new AbortController();
    let replied = false;
    const reply = (value) => {
      if (replied) return;
      replied = true;
      clearTimeout(timer);
      this.#respond(message, value);
    };
    const cancel = (code, reason) => {
      // Mark terminal before notifying App listeners: they may synchronously fail.
      reply({ error: wireError(new AppError(code, reason)) });
      controller.abort(new AppError(code, reason));
    };
    const timer = setTimeout(() => cancel('DEADLINE_EXCEEDED', 'App call deadline exceeded'), Math.max(1, deadline - Date.now()));
    this.#active.set(message.id, { cancel, controller, timer });
    const context = Object.freeze({ ...message.context, signal: controller.signal, deadlineMs: deadline });
    Promise.resolve().then(() => {
      if (controller.signal.aborted) throw new AppError('CANCELLED', 'App call cancelled before dispatch');
      if (!this.handleRequest) throw new AppError('METHOD_NOT_FOUND', 'App operation is not registered');
      return this.handleRequest(message.method, message.params, context);
    }).then((result) => reply({ result: result ?? null }), (error) => reply({ error: wireError(error) })).finally(() => {
      clearTimeout(timer);
      this.#active.delete(message.id);
    });
    // An ignored cancellation retains its slot until the handler actually stops.
    // The supervisor must terminate a worker that fails to cooperate.
  }

  close(reason = new AppError('DISCONNECTED', 'App IPC disconnected')) {
    if (this.#closed) return;
    this.#closed = true;
    const error = reason instanceof AppError ? reason : new AppError('DISCONNECTED', 'App IPC disconnected');
    for (const { settle } of this.#pending.values()) settle(error);
    for (const { controller, timer } of this.#active.values()) {
      clearTimeout(timer);
      controller.abort(error);
    }
    this.#active.clear();
    this.unsubscribe?.();
    this.transport.close();
  }
}

// The refused value's location and problem, never the App's own error text.
function detail(error) {
  try {
    if (typeof error?.detail === 'string') return error.detail;
  } catch { /* an App-made error object */ }
  return undefined;
}

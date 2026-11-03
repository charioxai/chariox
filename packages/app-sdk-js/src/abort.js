export class AbortSignal {
  #listeners = new Map();
  aborted = false;
  reason;
  onabort = null;
  addEventListener(type, listener, options = {}) {
    if (type === 'abort' && listener) this.#listeners.set(listener, Boolean(options?.once));
  }
  removeEventListener(type, listener) {
    if (type === 'abort') this.#listeners.delete(listener);
  }
  throwIfAborted() { if (this.aborted) throw this.reason; }
  _abort(reason) {
    if (this.aborted) return;
    this.aborted = true;
    this.reason = reason ?? Object.assign(new Error('The operation was aborted'), { name: 'AbortError' });
    const event = { type: 'abort', target: this };
    for (const [listener, once] of this.#listeners) {
      if (once) this.#listeners.delete(listener);
      if (typeof listener === 'function') listener.call(this, event);
      else listener.handleEvent(event);
    }
    this.onabort?.call(this, event);
  }
  static abort(reason) { const signal = new AbortSignal(); signal._abort(reason); return signal; }
}

export class AbortController {
  signal = new AbortSignal();
  abort(reason) { this.signal._abort(reason); }
}
